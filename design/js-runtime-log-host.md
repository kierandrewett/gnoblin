# JS runtime change log: host API, polkit, lock and the object model

The finished work of the session, process and timer API (A), idle watches, authentication and launch feedback (B),
polkit, the lock screen, input contexts and the scene. Part of the [JS/TS runtime design](js-runtime.md).

## Host API A status

User docs: `docs/js-host-api.md`. Rust: `crates/gnoblin-js/src/host_{api,process,session}.rs` (`__gnoblinHost.api`, JSON in and
out). TypeScript: `data/js/gnoblin/{operation,process,session}.ts`, wired in `data/js/gnoblin/host.ts` (`HostApi`, `apiFrame`).

- **Session.** The shell holds one more owner connection (`JsClientSessionLink`, a thread of its own).

    It subscribes to the whole graph, keeps the `session` object's lock properties for `status()`, calls `logout`,
    `poweroff`, `reboot`, `suspend` and `hibernate` on it and forwards every signal.

    `lock` and `reload-config` run on short connections of their own, so a reload that waits for this very process cannot
    hold the status thread. A refusal is an `Operation` failure with the code of `docs/shell-api/errors.md`.

- **Frame.** `run_host_api` runs before the layouts: it drains the session and process events into `apiFrame`, which settles
  operations, delivers signals and exits and fires due timers, all as queued calls under the call budget.
- **Processes** are children of the runtime (a waiter thread each, process groups, timeout, captured output). A captured
  stream is a string. `gnoblin.spawn` returns the handle; the host spawner now serves `autostart` only.
- **Reload.** A successful reload replaces the generation that asked, so its callbacks are gone; a refused one reaches them.
- **Proven on the real compositor** (`js_shell.rs`): status, a timer, two processes with exit events and captured output,
  and `reloadConfig` through the compositor's `reload_requested` signal to a typed `config_rejected`. The power verbs and
  `lock` are proven against a fake link only: calling them for real would suspend the machine.
- **Not built.** `process.on("changed")`/`watch`, `Window.process`, `session.activity`, `capabilities`, `focus.policy`,
  `runtime.status`, `settings`. `gnoblin.requestTransition` landed afterwards (see ["Transitions status"](js-runtime-log-ui.md#transitions-status)).

## Host API B status

Lane JsHostApiB. User page: `docs/js-idle-auth-launches.md`. The Lua pages are `docs/shell-api/{idle,privacy,input-grab,text-insertion,
launch-feedback,tablet-pad-help,window-snapping,authentication}.md`; every one of them is marked "not available yet" and
the Lua host implements none of them (`design/docs-parity/allow.toml` lists them), so the Lua host was no reference for
behaviour here: the page and the schema were. What is built is what the compositor serves today; the rest is named below
and not invented.

**Built.**

- **`gnoblin.idle`** (`data/js/gnoblin/idle.ts`, `crates/gnoblin-js/src/host_idle.rs`). `watch(ms, { onIdle, onResume,
onClosed })` returns an `IdleWatch`; `inhibit(holder)` returns an `IdleInhibitor`. A watch is a threshold over the
  compositor's `idle` object (`idle_ms`, `inhibitors`): idle is `inhibitors === 0 && idle_ms >= ms`, so a hold delays
  idleness as it delays a screen blank. The shell reads the object over an owner connection of its own
  (`JsClientIdleLink`: one lookup of the object's id on a throwaway connection, then a subscription to that object
  alone), opened when the running generation first has a watch or a hold and closed when it has neither, because every
  change of `idle_ms` goes to every subscriber. The connection is a `JsIdleConnector` in `JsRoleLinks`, so a test
  passes a fake. `JsShell::retire` gives every hold back (`uninhibit`) before it drops the connection. The compositor
  keeps each hold for the connection that made it (`idle.toml`; `Dispatcher::close` and `lost` release them), so a shell
  killed without retiring leaves none behind, and a lost idle connection ends the script's holds (`IdleMonitor.lost`).
- **`gnoblin.authentication`** (`authentication.ts`). Replaced by the observation-only API of the "Polkit status" section: there is no provider, no `allow` and no second slot connection.
- **`gnoblin.launches`** (`launches.ts`). The shell's own registry: `begin`, `list`, `finish`, four handlers, the page's
  limits (token 128, application 512, 100 to 10000 ms, 64 records).

    Windows answer a launch through two listeners on `gnoblin.clients` attached when the module loads (an arrival, a
    focus, an application ID that arrives late); the deadline is checked once a frame from `plan`. The compositor has no
    launch state (`allow.toml` says "documented, not implemented"), so this record is not shared with any other client.

- One entry call a frame, `wants()`, says whether a script needs the idle connection (bit 1); a shell that does not
  pays for that call. `JsShell::run_host_api` is the rest.

**Proven.** Against fakes: `crates/gnoblin-js/tests/host_idle.rs` (16 cases), `host_auth.rs` (16) and `host_launches.rs`
(13), covering the page's rules, the validation tables and the failures first (no connection, a lost one, a refused hold
or claim, a throwing handler, a request open at `retire`).

Against the real compositor (`crates/gnoblin/tests/js_shell.rs`, the idle and launch tests): a watch hears the
compositor's own idle clock pass its time, the input that ends it, and the next idle period;

a hold is counted in the compositor's `inhibitors`, keeps the watch from firing with the session idle for far longer
than the watch's time, and a reload that retires the generation gives it back;

a shell that dies without retiring leaves the compositor counting no hold and the next shell's watch hears the session
go idle; a launch is started by a real mapped window and another is timed out by its deadline.

**Gaps.**

- `privacy`, `input-grab`, `text-insertion` and `tablet-pad-help` are not built, because the compositor serves nothing
  to build them on: no privacy object (the PipeWire monitor is a session-bus service, `org.gnoblin.Privacy`, and carries
  microphone and camera only), no key-event stream and no keyboard lease (a seat claim takes a chord), no text-input
  target or `text_target` grant, no `pad-help-requested` event.

    A guaranteed release of a grab on `retire` or a crash, and a test of it, wait for the lease to exist. A privacy API
    must be read-only whatever it is built on.

- The snapping grant has no counterpart because there is nothing to grant: the shell owns the tree and writes `bounds`
  (`gnoblin/snap`), where the Lua callback holds a one-use capability over a window it does not own.
- Idle: no `watch:watch(field)`, and the `closed` reasons `config_reloaded` and `runtime_stopped` do not exist (a
  generation's watches end with it, without a signal); times are milliseconds. Launch feedback: a launch is not linked to
  the window or process that answered it, as on the Lua page.

## Polkit status

Design: `design/js-research/polkit.md`. A script never decides whether a password was right (J8).

- **The compositor verifies.** `gnoblin-service-polkit` offers each `BeginAuthentication` on `auth.polkit` with a
  payload (`gnoblin-polkit-auth::AuthRequest`).

    The compositor holds the request itself, draws `PolkitUi` as a secure modal (`ipc/secret.rs`, `ipc/polkit.rs`) and
    runs polkit's `polkit-agent-helper-1` (`HelperSession`) with the cookie on its standard input.

    Only the helper's `SUCCESS` answers the request; the helper has already told polkitd.

- **No script in the path.** A shell cannot claim `auth.polkit` or offer to it (`ipc/requests.rs`). `gnoblin.authentication
.addListener(new AuthenticationListener())` observes through `dialogs.observe` (`authentication.ts` over
  `dialogs.ts`): `actionId`, `message`, `subject`, `identities`, `deadline`, never the cookie, and `request.decline(reason)`
  is the only verdict. `setProvider`, `allow` and `host_auth.rs` are gone, with no alias.
- **Proven**, with a fake helper script and a fake polkitd on a private bus: `gnoblin-polkit-auth` tests, the dialog and
  service unit tests, `gnoblin-compositor/tests/polkit.rs`, `gnoblin-service-polkit/tests/dbus.rs`, and the
  `js_shell.rs` authentication cases. **Not proven:** a real `polkit-agent-helper-1`, a real polkitd, fprintd.

## Lock status

The session lock screen (lane JsLock; `docs/session-lock.md`, `design/js-research/popups-portals.md` row 9, decision J8).

- **Lock screen.**

    `data/js/gnoblin/lock.ts` ports `lock.lua` and adds the clock: `LockScreen` (`frame()` once a frame, by `runLockFrame`
    ahead of the toolkit's frame), `PasswordEntry` (a `Box` that keeps typed characters as code points and overwrites them
    when it submits), `LockStyle`.

    It reads the `lock` node's `state` (`__gnoblinHost.lock`, `crates/gnoblin-js/src/lock.rs`), mounts its scrim under the
    lock node so the compositor draws it over the cover, writes the node's `engaged`, takes the seat's focus for the entry
    and gives it back, and sends the password as the call's secret text.

    It uses the toolkit's `Canvas.onKey` (`UiKeyEvent`), `uiFocus` and `uiFocused`; a key event reaches only the canvas
    that holds the seat's focus.

- **Rust side.** The cover now draws, for the compositor's own lock only, the canvases under the `lock` node once the
  shell holds the tree and has engaged the lock (`Composer::compose_lock_ui`; it redraws when the subtree changes).

    The lock card (`LockUi`, `LockCardService`, `lock_card.rs` in `gnoblin-wayland`) is wanted when the lock is the
    compositor's own and proven and no shell holds the lock screen, with a 1 s grace while a shell is attached.

    "Holds" is a **lease**, not the `engaged` flag: the screen renews it with the lock node's `heartbeat` call (at most
    twice a second, only while it is up and its frame ran to its end).

    A lease not renewed for 2 s (`LOCK_UI_LEASE_MS`) lapses, so a shell that engaged and then hung, or whose lock frame
    throws every frame, loses the password entry to the card after 3 s.

    A shell that detaches has its `engaged` released. The card is the last layer of the frame and takes every key while
    up; a shell may not use the card's reserved check token. `ipc/localtime.rs` reads the time zone (TZif and its POSIX
    footer) because the compositor links no time library and forbids unsafe code.

- **J8, as built.** Decision J8 says Rust draws the lock password entry.

    The stock lock screen is TypeScript and collects the password in the script, which departs from it; the compositor
    holds the line that matters: only its own PAM check unlocks, and the Rust card is the entry whenever the screen is
    absent, not alive or not answering.

    A check that is lost is answered `failed` at once (a refused transaction cannot send a password again), and one the
    compositor never answers is answered `failed` after 30 s (`CHECK_DEADLINE`), so the screen takes another attempt
    instead of waiting on "Checking...".

- **Limits, honestly.** A JavaScript string cannot be wiped: the password is in the script engine's heap in the string
  built for the `authenticate` call and in the key events that carried it, until the collector frees them. The entry's own
  buffer is cleared, and the Rust card, which has no script, wipes. A `SecureEntry` node (popups-portals.md, 4.3) would
  remove it. The lock screen is part of the runtime: there is no config key to turn it off yet.
- **Proven against the fake link** (`crates/gnoblin-js/tests/lock.rs`, 23 tests): the Lua test's cases (engage, wait for a
  screen, attach while locked, unlock before the screen, external unlock and a later lock, Enter sends the typed password as
  secret text, ok, wrong password, rate limited) and the clock, focus, keys after the unlock and password-in-the-tree cases.
- **Proven against the real compositor** (`crates/gnoblin-compositor/tests/lock_card.rs`,
  `crates/gnoblin/tests/js_lock.rs`, scripted password check in place of PAM): no shell, right password unlocks; wrong
  password stays locked and the card still works; a shell that holds the lock screen keeps the card away and its death
  brings the card; a shell that attaches takes over;

    the JavaScript shell paints its screen under the lock node, answers a wrong password on it and unlocks.

- **Not proven.** Pixels: no test reads the output picture of the card or of the shell's screen. Real PAM. Several
  outputs: both draw on the first output. A shell reload while locked (the grace period is tested only by the policy, not
  by a reload).

## Input contexts, devices and the extension registry status

Lane SpecInput (design/js-spec-native.md 2.6; docs/shell-api/input.md). `data/js/gnoblin/devices.ts` holds `InputDevice`
and the `Keyboard`, `Mouse`, `Touchpad`, `Touchscreen` and `Tablet` classes with the settings ledger that `gnoblin.ts`
merges into the entry's `document()`;

`input/context.ts` holds `InputContext` (public constructor, `attach`, device ownership and per-device claims) and
`InputManager` (`contexts` and `devices` are live lists from `input_list.ts`); `native_object.ts` is the `NativeObject`
base. `JsShell::frame` commits the document again when `documentRevision` moved. Tests:
`crates/gnoblin-js/tests/input_devices.rs`.

Not native: a second `wl_seat` with its own focus, per-device (rather than per-kind) settings, applying the
`input.mouse` and `input.touchpad` groups.

## Scene status

Lane SpecScene (design/js-spec-native.md, section 2.5; docs/shell-api/scene.md).

Built: `gnoblin.scene` and `Scene`, writable `position`, `rotation` (Euler or quaternion), `scale` and `transform` on
script nodes and on `Client`, `Client.committedSize`, `Node.input` as `forward | capture | none`, `View.size` and
`contentFit`, a `View` of a workspace (a mirror of child Views, `data/js/gnoblin/scene_mirror.ts`), `ArcLayout` and
`Placement.transform`, and property bindings (`data/js/gnoblin/properties.ts`; `node.bind` and `node.properties.bind`).

Tests: `tests/scene_spec.rs` and the fences of `tests/spec_conformance.rs`. Not done: a perspective camera on a flat
workspace, pixel verification of a workspace View, and `contentFit` for a client's picture.
