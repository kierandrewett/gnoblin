# JS runtime: hot reload and generations

How the runtime swaps a whole generation when a file changes, what a failed reload keeps, and how seat claims survive
the swap. Part of the [JS/TS runtime design](runtime.md).

## Hot reload

Status of P5: the engine-side half is implemented in `crates/gnoblin-js` (`reload/`) over the language-neutral shell host
(`crates/gnoblin-shell-host`: `watch.rs`, `stage.rs`, `supervisor.rs`, `process/`), which every shell language shares (design/modular-framework.md,
section 17). It runs without a compositor. Process spawning, the Rust fallback shell and the real
compositor commit are not done: they belong to P3 and the process supervisor.

- **Watcher.** `ConfigWatcher` (`gnoblin-shell-host`) uses `notify` (inotify, CC0-1.0, no default features) on the config directory and on the
  directory of every module in the last generation's graph (`JsGeneration::module_paths`), non-recursively, because
  editors replace the inode.

    It drops editor scratch files (`*~`, `*.swp`, `4913`, `.#x`, `#x#`) and anything the caller's test does not call
    relevant: the JavaScript runtime passes `.ts`, `.mts`, `.js` and `.mjs`, and another language passes its own
    extensions (`with_extensions(&["py", "so", "toml"])`).

    After the first event it waits for 100 ms of quiet and returns one `ReloadRequest` with the changed paths; a burst
    that never goes quiet is cut off after 20 debounces. Any module file in a watched directory counts, imported or not: a
    new `./x.ts` can shadow an import that resolved to `./x.js`.

    A backend error or an inotify queue overflow becomes a request with no paths, because "possibly changed" must reload.
    Not done: comparing content hashes to skip a save that changed nothing.

- **Reloader.** The shell host's `Reloader` drives a language's `Stager`; `JsReloader` is the one over `JsHost`. It runs
  `JsHost::stage`, passes the staged generation to a caller-supplied `commit` closure
  (the compositor transaction, step 5 of the pipeline above), and only when that returns `Ok` calls `JsHost::commit`.

    A parse error, a throw, an interrupted loop or a rejected commit leaves the running generation untouched and returns a
    `ReloadReport { ok: false, errors }`. Each `Diagnostic` (`JsDiagnostic` for this runtime) carries file, line and column in `.ts` coordinates (line and
    column are 0 when the engine reported no position, and `file` is then the entry).

    `reload_now()` is the manual trigger for `gnoblinctl reload`; `poll()` waits on the watcher, reloads and re-arms the
    watcher from the new graph. A module that failed to load is in no graph, so the file named by the last error stays in
    `watch_set()` until a stage succeeds.

- **Supervisor.** `StrikeSupervisor` (`gnoblin-shell-host`) is pure policy over an injectable `Clock`. A budget interrupt or a
  panic is a strike (`GenerationFault`); other failures are not, because a thrown exception leaves the generation consistent. After a strike
  `JsReloader::recover` stages a fresh generation from disk and replaces the suspect one; a restage that is itself
  interrupted is another strike.

    Three strikes within 30 s latch `Health::Degraded`: retrying stops and the last good generation, if one was ever
    committed, keeps running. A fault during the 5 minute cool-down is refused without staging. A deliberate reload (file
    save or `gnoblinctl reload`) clears the strikes and the degraded state. The defaults are `StrikePolicy::default()`.

- **Not in this package.** Spawning and restarting the `gnoblin-js` process, exponential backoff between process
  restarts, the heartbeat watchdog (L3), the Rust fallback when the runtime is down, and passing `gnoblin.hot.data`
  between generations.

### Seat claims across a reload

Lane JsReloadClaims. The rule: **a reload releases the old generation's claims and takes the new generation's in one
transaction, releases first.** Nothing the shell holds on the compositor may be refused for being held, and none may be
held by nobody after the swap.

What went wrong.

A reload stages the new generation while the old one still runs, so the new generation's entry made its claims at stage
time, queued in the shell's pending writes ahead of the old generation's `unclaim` calls that `retire` queued at the
swap. The claims were:

- `bind` calls `Seat.claim`;
- a gesture calls `claim_gesture`;
- a layout's drags are pointer chords;
- `bindingOnly` calls `claim_device`.

The compositor applies a transaction's calls in order, and a chord that another claim holds is refused (`a seat claim's
chord is already bound for that device`; for a gesture, `a seat gesture claim was refused`). The first calls were
refused because the old claims were still there, and the old claims were then released: the chord was held by nobody.

The live run logged 19 refusals and one refused gesture for the stock keys alone.

Designs considered, and why the third:

- _Diff by chord_ (keep an identical chord's claim id, swap the handler, unclaim only what went). It sends the least, but
  it needs the claims to be shared state between two generations that have nothing else in common, a refcount that the
  old generation's `unclaim` and the new one's `claim` both move, and still refuses a chord whose grab flag or device
  changed. The id space is shared with gestures, device claims and drags, so every kind needs its own equality.
- _A compositor `replace` verb_ changes the protocol and the schema for a problem that one transaction already solves.
- _Release first, claim behind it, in one transaction_ (chosen). It uses the order the compositor already keeps, needs no
  new verb, treats chords, gestures, pointer chords and device claims alike, and cannot refuse anything the new
  generation claims, because a generation's own claims never collide with each other (the input module shares one claim
  among the bindings of a chord).

How it is done (`crates/gnoblin-js/src/input.rs`, `shell.rs`):

- `JsShell::begin_stage` starts keeping claims. A claim made while staging gets its id at once (the script goes on) and
  is held in `JsInput::staged`, not sent. A claim the staged generation gives up again is dropped without a call.
  `abandon_stage` drops what was kept, so a failed stage leaves the compositor and the host's table as they were, and a
  claim of a failed stage is never mistaken for a held one.
- The claims are not the only side effect of a stage. `HostState::begin_stage` (`bindings/stage.rs`) takes one mark of
  everything the staged entry touches in the shared state: the pending writes, the UI toolkit's canvases and measurements,
  the XR module's nodes, the output applies and the lock's password checks. `abandon_stage` gives them all back. A
  `gnoblin.spawn` at the top level is held back, not started (`Processes::begin_stage`, `gnoblin-shell-host`): the staged generation starts
  its programs when `swap_generation` takes over, and a program the running config started with the same command and
  options is taken over instead (same `id` and `pid`), so a reload does not double it. A program the running config
  started and the new one does not is sent SIGTERM; one a handler started is left alone. Each process records its owner,
  the generation that started or took it over. A restage inside one stage (the supervisor's) starts the stage's spawns
  over (`JsStaging::prepare`). A session verb called at the top level is not held back.
- `swap_generation` ends the old generation (`cancelAll`, its drags' `onCancel`, its `unclaim` calls), releases any claim
  the old script did not (it threw first), then sends the kept claims, and sends the one transaction. A transaction still
  in flight holds all of it back together.
- Ids stay one number space per shell, from 1, never reused: a kept claim's id is allocated when it is made, so the new
  generation's ids are above the old generation's.
- **Slots.** The `dialogs.observe` claim is made on the shell's slot connection and is not part of the transaction.
  `JsDialogs::hand_over` answers the old generation's unanswered requests `pass` and keeps the slot claimed; the new
  generation's first frame keeps the claim if it has a dialog listener and releases it if it has none, so the
  compositor never offers a request to nobody in between. A shutdown still releases it (`retire`).
- The `idle` holds and the XR program's controller claims are not seat claims. A reload gives the first back by design
  and the XR program's are released by the generation's `retire` as before.

Proof. `crates/gnoblin-js/tests/input.rs` runs the role over a fake compositor that refuses a held chord, gesture or
device claim like the real one: chords kept, added and removed, a swipe, a layout's drags and a device across reloads,
five in a row; every release is sent before every claim;

the compositor holds exactly the new generation's set when the swap returns; and a stage that claims and then throws
sends nothing.

`crates/gnoblin/tests/js_shell.rs`,
`a_reload_hands_every_seat_claim_over_so_each_shortcut_fires_once_and_a_removed_one_is_released`, runs the stock shell
in a real compositor. It presses these on the generation it booted and after each of six reloads (each adds a key and
takes the last one's away; the last also takes `Super+y` away):

- every default chord (`Super+1` to `Super+9`, `Super+Left`, `Super+Right`, `Super+space`, the seven media keys);
- the user's keys;
- a three-finger swipe;
- a `Super+Button1` drag.

Each chord must fire exactly once, the client's keyboard must not hear a claimed key, a key the reload removed must
reach the client again and not fire, the drag must move the window, and neither the compositor's nor the shell's log may
hold a refused claim.

A real client's keyboard is the witness of a release (`Mapper::keyboard`), since a claim that nobody holds is a key that
types. The generation's actions are replaced by counters in the test's `init.ts`: running them would start `wpctl`, open
the launcher and slide the window the test drags.
