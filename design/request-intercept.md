# Request intercept: an extension API for privileged requests

Status: **proposal, 2026-11-08.** Not decided. Kieran confirms the open points in
section 6 and the decision row D32 in `design/rust-compositor.md`.

Source of the idea: Kieran's review of the privileged-request audit log. He said it
"probably suited better as an API that allows people to create an extension to intercept
requests and log before it". This document treats that as two questions: how a privileged
request is observed, and how an extension may act on it before it runs.

Labels used below: **[FACT]** is read from the code or a file named in the text, with a
line number at the commit read. **[INFERENCE]** is a reasoned guess not yet checked.
**[UNVERIFIED]** means the claim needs a read or a test before anyone relies on it.
**[JUDGEMENT]** is the author's recommendation.

Line numbers were read on branch `rust-compositor` at lane commit `4bc079d52`.

## Summary

- **[FACT]** Privileged requests do not pass through one choke point. They pass through
  at least six separate gates, in four crates, each with its own identity handling (section 1).
  An interceptor therefore cannot be a single hook. The first real work is a shared
  decision record that every gate writes.
- **[FACT]** The extension system (D22, decided 2026-11-05) already names a `hooks` power
  and a capability table. An interceptor fits that vocabulary. It cannot be given
  veto power until extensions get their own Lua environments, because today every
  Lua chunk shares one `gnoblin` global (`crates/gnoblin-lua/src/bindings.rs:389`).
- **[JUDGEMENT]** Recommend option E: a built-in append-only audit record that needs no
  extension (phase 1), an observe-after-decision event stream for extensions (phase 2),
  and a veto hook that can only make a request stricter (phase 3). An interceptor must
  never be able to approve a request the compositor would refuse.

## 1. Request kinds and where they pass

Each row is one kind of privileged request. "Choke point" asks whether the kind passes
one function that every such request reaches.

| #   | Kind                                                                                                                                                                                                                 | Entry and gate [FACT]                                                                                                                                                                                                                                                                                                                                                                                       | Identity used                                                                                                                                                           | One choke point?                                                                                                                                                                                     |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `gnoblin` namespace IPC: config commit, config reject, config current, config reload, portal UI, portal cancel, portal key, capture screen, subscribe, call, claim, answer, decline, offer, slot claim, slot release | `Dispatcher::request` at `crates/gnoblin-compositor/src/ipc/server.rs:830`. Capability gate `required_capability` at `server.rs:2421-2432`. Refusal with `wire::code::DENIED` at `server.rs:845-858`. Verb match at `server.rs:861-876`.                                                                                                                                                                    | Client id, then privileged role from `grants(role)` at `crates/gnoblin-compositor/src/ipc/privileged.rs:77`. Same-uid owners get `owner_grants` at `privileged.rs:108`. | **Partly.** The capability check is in `request()`, but OFFER is gated per slot inside `offer()`, and some verbs check power again inside their handler.                                             |
| 2   | Extension namespace requests (`ext` namespace)                                                                                                                                                                       | Routed by `extensions.route` at `crates/gnoblin-compositor/src/ipc/extension.rs:106`. The provider receives the request. The module doc at `extension.rs:1-13` says the payload is opaque to the compositor.                                                                                                                                                                                                | Caller client id. Whether the provider sees the caller id is **[UNVERIFIED]**.                                                                                          | Yes for routing. The compositor does not check the payload, so it cannot classify the request.                                                                                                       |
| 3   | Window verbs: close, set_minimized, set_maximized, set_fullscreen, move, resize, set_workspace, set_output                                                                                                           | Object dispatch at about `server.rs:1520-1550`, then `window_control_call` at `server.rs:1927-1975`. Power check at `server.rs:1936-1943`, then `graph.call`, `reply_with_operation`, `control.window_control`, `finish`.                                                                                                                                                                                   | Client id and its grants.                                                                                                                                               | No. The object dispatch and `window_control_call` are two gates.                                                                                                                                     |
| 4   | Layer verbs                                                                                                                                                                                                          | `layers_call` at `server.rs:1763-1790`. Power check needs `RegisterLayer`.                                                                                                                                                                                                                                                                                                                                  | Client id and its grants.                                                                                                                                               | No. Separate from #3.                                                                                                                                                                                |
| 5   | Effect verbs                                                                                                                                                                                                         | `add_effect_call` at `server.rs:2197`.                                                                                                                                                                                                                                                                                                                                                                      | Client id.                                                                                                                                                              | No. Separate function with its own checks.                                                                                                                                                           |
| 6   | Request broker: auth.polkit, secret.derive and prompt slots                                                                                                                                                          | `crates/gnoblin-compositor/src/ipc/requests.rs`. The module doc at lines 1-40 covers brokering, slots, providers (`shell`, `services`, `none`), refusal when no provider is connected, and ending requests when a provider departs.                                                                                                                                                                         | Requester client; provider client.                                                                                                                                      | **Yes, per slot.** Every request on a slot passes the broker. This is the natural place for a per-request record.                                                                                    |
| 7   | Portal consent                                                                                                                                                                                                       | `PortalService` in `crates/gnoblin-compositor/src/ipc/portal.rs`: `submit` at line 138, `key` at 187, `cancel` at 201, `departed` at 227, `secret_granted` at 253, `settle` at 281. Screenshot portal in `crates/xdg-desktop-portal-gnoblin/src/iface/capture.rs`. Consent in `crates/xdg-desktop-portal-gnoblin/src/consent.rs`.                                                                           | Requester (app id), capability.                                                                                                                                         | **Yes inside the portal service.** The portal service is the consent point for these.                                                                                                                |
| 8   | Authority decisions                                                                                                                                                                                                  | `Authority::gate(now, requester, capability)` at `crates/gnoblin-authority/src/authority.rs:163` returns `Decision`. Refusal with `Denial::NoAuthority` at `authority.rs:266`. `Denial` is at `crates/gnoblin-authority/src/denial.rs:9`. `PortalCapability` is `{Capture, InputInjection, Secret}` at `crates/gnoblin-authority/src/event.rs:190`. The crate is pure: no I/O and no clock (`lib.rs` docs). | Requester id, supplied by the caller.                                                                                                                                   | **Yes as a function, but not as a call site.** Callers today: `gnoblin-input-eis`, `gnoblin-wayland` (`protocols/extension.rs`, `protocols/xdg_activation_v1.rs`), and `xdg-desktop-portal-gnoblin`. |
| 9   | Wayland privileged globals: layer-shell, foreign toplevel, data-control, screencopy, image-copy-capture, virtual keyboard and pointer, gamma control, background effect                                              | Global enum at `crates/gnoblin-wayland/src/access.rs:63-80`. `decide(role, global)` at `access.rs:240`. `AccessPolicy::allows` at `access.rs:273` and `:392`. The gate is `can_view` on bind and on advertise: `zwlr_gamma_control_v1.rs:110`, `zwlr_screencopy_v1.rs:76-77`, `capture_shared.rs:209-212`, `zwp_virtual_keyboard_v1.rs:128`, `ext_background_effect_v1.rs:79`.                              | `ClientRole` and `ClientKey`.                                                                                                                                           | **Gate per global, not per request.** See below.                                                                                                                                                     |
| 10  | Screencopy copy and data-control requests                                                                                                                                                                            | `zwlr_screencopy_v1.rs:179` performs the copy. There is no per-request check there. Data-control requests are not checked per request: only the global's visibility is.                                                                                                                                                                                                                                     | Same as #9.                                                                                                                                                             | **No.** A client that can see the global can make every request on it.                                                                                                                               |
| 11  | Access logging                                                                                                                                                                                                       | `access.rs:322` and `access.rs:338` log by role at debug level only.                                                                                                                                                                                                                                                                                                                                        | Role.                                                                                                                                                                   | No audit record exists.                                                                                                                                                                              |
| 12  | EIS input injection                                                                                                                                                                                                  | `crates/gnoblin-compositor/src/eis.rs:1-7`: default deny; `EisGate::grant` at `eis.rs:47`; the module says "Nothing in this crate grants yet". Service in `crates/gnoblin-input-eis/src/service.rs`.                                                                                                                                                                                                        | Authority id for `InputInjection`.                                                                                                                                      | **Yes**, `EisGate`, by design. Per-event injection is not gated per event **[UNVERIFIED]**.                                                                                                          |
| 13  | Clipboard and data-control reads                                                                                                                                                                                     | Same gate as #9 and #10. Clipboard contents do not pass any audit point.                                                                                                                                                                                                                                                                                                                                    | Role.                                                                                                                                                                   | No.                                                                                                                                                                                                  |
| 14  | Capture still and streams                                                                                                                                                                                            | `CAPTURE_SCREEN` maps to `CaptureScreen` in `required_capability` (#1). Still images: `gnoblin-capture` `still::choose_output` used at `server.rs:1424`. Refusal codes at `server.rs:2436`. Capture while locked is refused by D22.                                                                                                                                                                         | Client id, portal consent.                                                                                                                                              | No. Two paths: the IPC verb and the portal.                                                                                                                                                          |
| 15  | Compat D-Bus services (gnome-shell screenshot, introspect, idle monitor, mutter displayconfig)                                                                                                                       | Each service calls `admit(call, self.gate())`, for example `crates/compat/gnoblin-compat-dbus-gnome-shell-screenshot/src/service.rs:151-155`. The `Gate` type is built from a lock state and a mode. Its definition was not read **[UNVERIFIED]**.                                                                                                                                                          | Caller bus name, presumably **[UNVERIFIED]**.                                                                                                                           | No. Each service has its own admission.                                                                                                                                                              |
| 16  | Lua process spawn                                                                                                                                                                                                    | `crates/gnoblin-lua/src/process.rs:644`. Spawn is `Proposed` in D22, not gated yet.                                                                                                                                                                                                                                                                                                                         | Lua chunk, which shares one global.                                                                                                                                     | No.                                                                                                                                                                                                  |
| 17  | Session spawns                                                                                                                                                                                                       | `crates/gnoblin-session/src/supervisor.rs:269` (`command.spawn` at line 378) and `crates/gnoblin-session/src/guardian.rs:429`. These are the session's own children, not an extension request.                                                                                                                                                                                                              | Session.                                                                                                                                                                | Not extension-reachable.                                                                                                                                                                             |
| 18  | Config commit                                                                                                                                                                                                        | `commit` at `server.rs:1192` reads the document. Gate is `CommitConfig` (#1).                                                                                                                                                                                                                                                                                                                               | Client id.                                                                                                                                                              | No. Same as #1.                                                                                                                                                                                      |
| 19  | Session logout and power                                                                                                                                                                                             | The guardian's `ShutdownReason::Logout` is at `crates/gnoblin-session/src/event.rs:175`. No extension-reachable logout verb was found in `crates/gnoblin-compositor/src/ipc/`.                                                                                                                                                                                                                              | Session.                                                                                                                                                                | Not extension-reachable **[INFERENCE]**.                                                                                                                                                             |

**Conclusion [JUDGEMENT].** There are at least six distinct gates: the `request()` capability
check (#1), the window power check (#3), the layer power check (#4), the request broker (#6),
the portal service (#7), and the Wayland global visibility checks (#9). Plus EIS (#12) and
the compat D-Bus admissions (#15). No interceptor can attach at one point. A decision record
has to be emitted at each gate, with a shared schema. Per-request Wayland checks (#10) are
missing today and would need adding before those requests can be recorded at all.

Two gaps matter most for the audit goal:

- **Per-request Wayland.** Screencopy copies and data-control requests are not checked per
  request (#10), so a client that once had the global can act without another decision.
  An audit record here would be per global bind, not per frame or per request. Per-frame
  records would be too costly (section 4.4).
- **Access records are debug-only** (#11). Nothing persistent records a Wayland privileged
  global being granted to a client.

## 2. Attribution: who a record names

Today the identities differ by layer:

- IPC: a client id (`gnoblin_types::ClientId`) and a privileged role (`grants(role)`,
  `privileged.rs:77`). The owner uid gets `owner_grants` (`privileged.rs:108`).
  Peer credentials are read in `crates/gnoblin-compositor/src/ipc/privileged.rs` **[UNVERIFIED]**
  whether a pid or exe is kept.
- Wayland: a `ClientRole` and a `ClientKey` (`access.rs`). No app id is recorded in the
  access decision.
- Authority: a `RequesterId` (`gnoblin-authority`), supplied by the caller. The crate
  does not read `/proc` (`lib.rs`), so the compositor must supply the identity.
- Extensions: the namespace owner (provider connection) under `extension.rs`.

**[JUDGEMENT].** Every audit record should carry the same tuple, filled as far as the layer
allows: client id; role; extension id if the request came through an extension namespace or
an extension's Lua chunk; Wayland app id if the client supplied one; pid and uid from peer
credentials if available **[UNVERIFIED]**. A record with only a client id is still
useful, because the compositor already keeps the client-to-role map.

## 3. The extension system and how an interceptor would register

### 3.1 What D22 already provides

- **[FACT]** D22 (`design/rust-compositor.md:2724`, decided 2026-11-05) packages extensions
  with `extension.toml`. The manifest has `[powers.ipc]`, `[powers.lua]`, `[powers.wasm]`,
  `[powers.process]`, `[powers.wayland]`, and `[powers.hooks]` (`rust-compositor.md:2744-2776`).
  `[powers.hooks]` names entries in the package (`:2770`).
- **[FACT]** The capability table (about `:2788-2811`) lists `window.read` and `input.observe`
  as Proposed, `capture` as Proposed and refused while locked, `spawn` as Proposed, and
  `model.<object>.<member>` as decided in principle. The never-grantable list is
  `lock.*`, `authority.*`, `secret.read` and `session.privileged` (about `:2813-2817`).
- **[FACT]** D22 says each tier calls the capability check at its own boundary (about `:2818`).
  The tiers are a Lua module, WASM, a process provider and a Wayland client (about `:2822-2827`).
- **[FACT]** D22 lists per-extension Lua environments as a blocker in its open decisions
  (about `:2940-2951`). Today every chunk gets the same `gnoblin` global
  (`bindings.rs:389`, `lua.globals().set("gnoblin", gnoblin)`), and `bindings.rs:192` installs it.
- **[FACT]** D22 caps hot-path hooks at a budget starting at 1 ms, and drops a hook after
  three consecutive overruns (`rust-compositor.md:2841`). That budget was set for layouts and
  input hooks. It is the model for an interceptor's deadline.

### 3.2 Proposed capability names

These are proposals, not decided:

- `request.observe`: receive the decision record after it is made (section 4, option A/B).
- `request.intercept`: receive a request before it runs and answer `deny` or `ask` (option C).
  The grant ceiling is restrict-only: the answer `allow` does not exist for an interceptor.
- `audit.read`: read the audit ring through `gnoblin.audit` or `gnoblinctl audit`.

Registration, two forms. **[JUDGEMENT]** Use the manifest form, so the compositor knows at
load time which kinds an extension intercepts and can refuse a load with an unknown kind:

```toml
[powers.hooks]
request-intercept = ["capture.still", "input.eis.grant"]
```

and the Lua form inside the package:

```lua
gnoblin.audit.on("decided", function(record) ... end)          -- observe, option A
gnoblin.audit.intercept("capture.still", function(request) ... end)  -- veto, option C
```

The Lua form names `gnoblin.audit` rather than `gnoblin.intercept` **[JUDGEMENT]**: the
object is the audit record stream, and the interceptor is a second method on it. This
matches the object model in `design/lua-api-goal.md`, where each object declares its
properties, methods and signals.

## 4. Options

Each option is additive. Option E combines them.

### 4.1 Option A: observe after the decision

An event `gnoblin.audit` `decided` carries the completed record after the decision and the
effect. The extension cannot change anything.

- **Pros.** Nothing to time out. No change to request latency beyond queueing. A crash of
  the extension does not affect any request. Good for logging and for notifications.
- **Cons.** It cannot stop anything. A log line can be written after the effect, not before.
- **Risks.** A flooding source fills the queue. Mitigate with a bounded per-extension queue
  and a dropped-count record, as in section 4.6.

### 4.2 Option B: observe before, without waiting

The extension is told before the request runs, and the compositor does not wait.

- **Pros.** The extension sees the request first, so it can notify the user before the
  effect.
- **Cons.** Ordering is not guaranteed: the effect may happen before the extension acts. The
  message is an announcement, not a gate, so it adds little over option A except timing.
- **Risks.** Users may read "about to capture" as a control. The record must say "delivered
  to observers", not "held for observers".

### 4.3 Option C: veto or force consent

The compositor parks the request, sends it to each interceptor, and waits up to a deadline.
Each interceptor answers `deny`, `ask` or no answer. The request resolves as follows.

- `deny` from any interceptor refuses the request.
- `ask` from any interceptor forces the compositor's own consent path, if the compositor
  would have allowed the request. It never resolves to allow by itself.
- No answer by the deadline, a crash, or a departed provider: a fallback that is set per
  kind (section 6, question 2). The recommended default is `ask`, not `deny`, because a broken
  extension must not silently disable capture or input.
- The compositor's own gates still run. The result is the stricter of the compositor's
  decision and the interceptors' answers. An interceptor cannot turn a refusal into an allow.
  Lock rules, capabilities, portal consent and authority decisions still apply first.

Invariants **[JUDGEMENT]**, each to be a test:

1. An interceptor answer can only lower the outcome. No answer raises it.
2. A request refused by the compositor is refused before any interceptor runs.
3. Locked state refuses request kinds regardless of interceptors, as D22 requires for capture.
4. Requests an interceptor makes while answering are not intercepted again. They still go
   through the normal gates. This stops a loop where an interceptor's own window action is
   re-intercepted.
5. An interceptor sees only the request, never another interceptor's answer. This avoids
   collusion and keeps the outcome order-independent.

Ordering with several interceptors: all run for the same request, in manifest load order,
and the outcome is the strictest answer. Parallel delivery with a combined answer is simpler
than a chain, because one slow interceptor cannot starve the others beyond the shared
deadline.

- **Pros.** Real policy: a site can refuse capture to an extension it does not trust, or
  require consent for every input injection.
- **Cons.** Parked requests are new state in the request path. The main loop must not block on
  Lua. `requests.rs` already parks requests and ends them when a provider departs, so the
  pattern exists. Per-extension Lua environments must exist first (D22 blocker).
- **Risks.** A slow or hostile interceptor adds latency to every matching request. Mitigate with
  the deadline, a per-kind opt-in, and dropping a hook after repeated overruns (D22's rule).

### 4.4 Option D: built-in append-only audit record

No extension needed. The compositor writes one record per decision at each gate in section 1.

Record fields **[JUDGEMENT]**:

- `seq` (monotonic), `time` (wall clock and monotonic tick), `kind` (one of section 1's rows),
  `gate` (the choke point that decided), `requester` (section 2 tuple), `target` (an object id,
  output name, window id, or stream id, never content), `decision` (`allow`, `deny`, `ask`,
  `refuse`), `reason` (a `Denial` or capability name), and `outcome` (`done`, `failed`, `dropped`).

Redaction rules, the core of the design:

- **Never** record clipboard or selection contents, typed keys, virtual keyboard or EIS key
  codes, pointer paths, portal screenshot pixels, stream frames, or secret values
  (`secret.derive`, `RetrieveSecret`).
- **Config commit** records the document's key names and a hash of its bytes. It does not record values.
- **Redaction is not a heuristic.** `gnoblin-log`'s `Redactor` (`crates/gnoblin-log/src/redact.rs`)
  says it is best-effort and "never a guarantee". The audit record must not depend on it. A field
  that might hold a secret is omitted by type, not scrubbed by pattern.
- **Diagnostic logs are separate.** D23's log (`gnoblin.jsonl`) is for debugging and may be
  rotated freely. The audit file is its own file under `$XDG_STATE_HOME/gnoblin/audit/`, owner-only.

Storage **[JUDGEMENT]**: an in-memory ring of recent records, and an append-only file of
JSON lines with size-based rotation. The compositor writes the file from a dedicated thread,
fed by a bounded channel, so a slow disk never blocks a request. A full channel drops the record
and increments a counter, which the next record reports as `dropped: N`. The file is not
tamper-evident. Same-uid processes can read it, and a hash chain is an option (section 6).

Read API: `gnoblinctl audit [--since T] [--kind K] [--json]` and a read-only Lua method
`gnoblin.audit.recent(n)` under `audit.read`.

Hot-path cost **[JUDGEMENT]**: the audit is per decision, not per event.

- **Input injection (EIS, virtual keyboard):** one record per grant of an input session, not per
  key or motion. Pointer motion is never audited.
- **Capture:** one record per capture request or stream start, not per frame. Screencopy copies
  (#10) are recorded once per bind.
- **Wayland globals:** one record per bind, not per request.

The ring push is an O(1) append on the compositor thread. Its cost must be measured before
phase 1 lands **[UNVERIFIED]**: no measurement exists yet. The acceptance test in phase 1 sets the
budget.

### 4.5 Option E: built-in audit plus the extension options

Recommended **[JUDGEMENT]**: D always on. A for extensions with `audit.read` or `request.observe`. C
for extensions with `request.intercept`, phase 3 only, after per-extension Lua environments exist.

### 4.6 Failure modes

| Failure                                      | Behaviour                                                                                                                        |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Extension crashes while a request is parked  | Treat as "no answer": the per-kind fallback applies. The request is ended, as `requests.rs` already does for departed providers. |
| Deadline passes                              | Same as no answer. Count the overrun. Drop the hook after three in a row (D22).                                                  |
| Extension floods events                      | Bounded queue per extension. Overflow drops and reports a count. Never blocks the compositor.                                    |
| Audit disk full or slow                      | The channel drops and reports `dropped: N`. Requests are not delayed.                                                            |
| Interceptor requests a new privileged action | The new request goes through the normal gates and is not intercepted again (invariant 4).                                        |
| Extension unloads while parked               | Parked requests resolve by the fallback. Never by allow.                                                                         |
| Two extensions disagree                      | The stricter answer wins (section 4.3).                                                                                          |

### 4.7 User-visible parts

- **Notification [JUDGEMENT].** A denial or a forced consent notifies the user, naming the
  extension. Whether this is opt-in is question 7 in section 6.
- **Privacy indicator.** `org.gnoblin.Privacy` (`crates/gnoblin-service-privacy/src/lib.rs`)
  publishes an activity triple through `ActivityChanged`. Capture and input sessions could feed it
  **[INFERENCE]**. Not checked whether the service accepts activity from the compositor today.
- **Audit view.** `gnoblinctl audit` is the only UI in this proposal. A settings page is not proposed.

## 5. Phase plan

Each phase is tests first (failure modes before the happy path), with no stubs. Crate and file
names are proposals.

### Phase 1: built-in audit record, `gnoblinctl audit`, read object

- **Files.** A new library crate `gnoblin-audit` with the record type, the ring, redaction by
  type, and the file sink. It depends on `gnoblin-log` with no feature, per D23. The binary enables
  the file sink from `[audit]` config. Call sites in `crates/gnoblin-compositor/src/ipc/server.rs`
  (`request`, `window_control_call`, `layers_call`), `ipc/requests.rs`, `ipc/portal.rs`,
  `crates/gnoblin-wayland/src/access.rs`, and `crates/gnoblin-compositor/src/eis.rs`. `gnoblinctl`
  gets the `audit` verb.
- **Tests first.**
    1. A refused `CONFIG_COMMIT` from an unprivileged client produces one `deny` record with the
       `Denial` reason, and no config value in the record.
    2. A `CAPTURE_SCREEN` refused while locked is recorded as `refuse`, and its target has no frame data.
    3. A ring overflow records `dropped: N` and does not delay the request.
    4. A full channel drops records and never blocks the compositor thread (measured against the
       budget in section 4.4).
    5. A record for a clipboard or key event does not exist: the type has no content field.
    6. The file rotates at the size limit, and records are not lost across rotation.
    7. `gnoblinctl audit --json` output matches the schema in the docs.
    8. Owner-only file mode.
- **Acceptance.** A new `docs/` page for the audit schema and `gnoblinctl audit`, with names that
  pass `xtask docs-parity`. The Lua `gnoblin.audit.recent(n)` read method under `audit.read`, with a
  test that an extension without the power is refused. The measured ring push cost.

### Phase 2: observe events for extensions

- **Files.** `crates/gnoblin-lua/src/bindings.rs` for `gnoblin.audit.on("decided", fn)`,
  `crates/gnoblin-compositor/src/ipc/extension.rs` for fan-out to granted extensions, and the
  manifest parser for `request.observe`.
- **Tests first.** Observers never receive records they lack the power for. A slow observer drops
  events and reports a count. An observer's crash does not affect any request. An observer cannot
  change a decision (the event carries no reply path).
- **Acceptance.** Option A works end to end with one test extension. No change to request latency
  beyond the measured queueing cost.

### Phase 3: veto hooks

- **Precondition.** Per-extension Lua environments (D22 blocker). Without them, an interceptor in
  the shared global could read or change another extension's state.
- **Files.** The parked-request state in `crates/gnoblin-compositor/src/ipc/requests.rs` (reuse its
  departure handling), the interception point in `server.rs` before each gate's effect, and
  `[powers.hooks]` validation in the manifest parser.
- **Tests first.**
    1. A request interceptor that never answers resolves by the configured fallback, not by allow.
    2. A crash mid-request ends the request by the fallback.
    3. An interceptor cannot turn a compositor refusal into an allow.
    4. A request made by an interceptor is not intercepted again.
    5. Locked state refuses capture with or without interceptors.
    6. Two interceptors: the stricter answer wins, and neither sees the other's answer.
    7. Three consecutive overruns drop the hook, and the drop is recorded.
- **Acceptance.** The invariants in section 4.3 each have a test. A test extension that denies a
  capture request sees the request refused and the record shows the interceptor's name.

## 6. NEEDS KIERAN

1. **Built-in audit or extension only?** Recommendation: built-in (option D), separate from the D23
   diagnostic log, with extensions added on top.
2. **Veto timeout fallback.** `ask` (recommended: the compositor's own consent path) or `deny`?
   `ask` keeps a broken extension from silently disabling capture; `deny` is stricter.
3. **Clipboard and input metadata for observers.** Recommendation: observers never see content, and
   see input only as "session granted" with no key or motion detail. Confirm.
4. **Retention.** Ring size, file size, rotation count, and whether same-uid readers are acceptable.
   Recommendation: owner-only file, no tamper evidence, unless question 5 says otherwise.
5. **Tamper evidence.** A hash chain over records, or not? It costs one hash per record.
6. **Power names.** `request.observe`, `request.intercept`, `audit.read`. Confirm the names and the
   grant ceiling: may an interceptor ever be granted to a WASM or process extension, or Lua only?
7. **Notifications.** Notify the user on each denial or forced consent, or only when opted in?
8. **Config values in the audit.** Recommendation: key names and a hash only. Confirm.
9. **Whether `ask` may be used by an interceptor at all**, or only `deny`. Recommendation: both, with
   `ask` forcing the compositor's own consent.
10. **Compat D-Bus gate (#15).** Whether to bring the compat services under the same record schema in
    phase 1, or later. Their `Gate` type was not read **[UNVERIFIED]**.

## 7. Unverified, and what would settle it

- The `Gate` type in the compat D-Bus services (#15): read its definition and its identity source.
- Peer credentials kept per client, and whether a pid or exe reaches the audit record (section 2).
- Whether the provider of an extension namespace sees the caller id (#2).
- Per-event injection in EIS (#12): whether any per-event check exists below `EisGate`.
- The hot-path cost of a ring push and a channel send on the compositor thread: measure in phase 1.
- Whether `gnoblin-service-privacy` accepts activity from the compositor (section 4.7).
