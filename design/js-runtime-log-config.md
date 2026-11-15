# JS runtime change log: role wiring and the config document

The finished work of the runtime role (P6), the JS-owned role and the config document, slots and starter. Part of the
[JS/TS runtime design](js-runtime.md).

## P6 role wiring

`JsRole` (`crates/gnoblin-js/src/role.rs`) is what `ShellRole` is for Lua: the shell the runtime role runs, one frame
per turn of its loop.

It is tested against a real compositor session in `crates/gnoblin/tests/js_shell.rs`; a fake compositor stands in only
for the unit tests of `swap_generation` (`crates/gnoblin-js/tests/shell.rs`).

- **Selection.**

    Superseded by "JS-owned runtime role". `GNOBLIN_SHELL_VM` chose the role:

    - unset or `js`: the JavaScript role.
    - `js:<dir>`: the JavaScript role, with the stock `gnoblin` modules read from `<dir>` (the layout of
      `data/js/gnoblin`; a file that is not there is the compiled-in one).
    - `lua`: the Lua role.

    The user's entry is the first of `init.ts`, `init.js` in `$XDG_CONFIG_HOME/gnoblin` (else `$HOME/.config/gnoblin`).
    With neither file the shell runs the stock default config (below) and loads `init.ts` when it appears.

- **Wiring without a cycle.** `gnoblin-js` and `gnoblin-lua` are siblings and neither names the other. The `gnoblin`
  binary, which already names both ends, asks `gnoblin_js::choose_runtime_from_environment` which role to run and calls
  `JsRoleRunner::run` or `RoleRunner::run`.

    `xtask deps` lists `gnoblin-js` among the binary's role entry crates for this. (Until the JS-owned role the Lua host
    ran the JavaScript shell through a `ShellFrame` trait; that trait, `ShellSlot::from_env_or_js` and
    `RoleRunner::run_with_js` are gone.)

- **The loop.** `JsRole::connect` reads the socket by the `GNOBLIN_IPC_SOCKET` rule, handshakes as a client at this
  build's version, and hands the handshaken socket to a `SocketTreeLink`. Each `tick` acts on a reload request, runs
  `JsShell::frame`, and then acts on strikes. A link failure ends the shell: `tick` returns it, the slot empties and the
  role keeps serving, as for Lua.

    Slots and the configuration document use two more owner connections the role opens (see "P6 config document and slots
    status"). `gnoblin.spawn` starts the program in a process group of its own with no terminal and reaps it.

- **Reload.** The role owns a `ConfigWatcher` (100 ms debounce), a `JsReloader` and a `JsSupervisor`.

    The reloader stages the entry in a generation of its own (`JsHost::set_prepare` gives it the shell's host objects over
    the shell's tree mirror, `JsStaging::prepare`), and the commit closure keeps a second handle on it
    (`JsGeneration::share`).

    When the reload is accepted the role calls `JsShell::swap_generation`: it reads the old generation's
    `workspace_snapshot`, `retire`s it (drags cancelled, seat claims released, in the transaction that carries the new
    generation's own claims), drops it with its listeners and queued calls, runs `finishLoad` on the new one and
    `reconcile`s the windows back onto the workspaces of the same id.

    The new generation has already told its listeners of the windows that exist (replay on `addListener`) and is announced
    `ready` for every window on the next frame. Windows are nodes of the compositor's tree, so their ids do not change and
    nothing is re-created or moved.

    `JsShell::begin_stage` and `abandon_stage` take back a write the staged entry made at its top level (`client.x = 5`)
    when the stage fails after it.

- **Failure.** A syntax error, a throw, a budget trip while loading or a stage that is rejected keeps the running
  generation and logs one warning, `the running generation stays: <file>:<line>:<column>: <message>`, in the
  author's `.ts` coordinates. The file the error named stays watched, so the edit that fixes it is seen.
- **Runaway handlers.** A listener call that runs past `entry_budget` (1 s) is interrupted, reported, and counts a
  strike on the shell.

    The role then asks the supervisor: it restages the generation from disk, and three strikes in 30 s latch `Degraded`,
    after which the last generation keeps running and a deliberate reload (a save, or `JsRole::reload_now`) clears the
    strikes.

    The shell is a process of its own, so the compositor never waits for a script: the test runs the role in a thread,
    spins a handler for ever, reads the scene through an observer connection while it spins, and maps one more window
    afterwards.

- **Zero-config, and no layering.** `JsShell::load_default_config` (the `loadDefault` hook) runs `installDefaults` of
  `gnoblin/starter`: the same calls an `init.ts` makes.
    - The role runs it when there is no entry file and when the entry fails to load at startup, so a shell is never bare.
    - There is no merge. An `init.ts` has none of the stock keys or workspaces unless it calls `installDefaults(gnoblin)` or
      copies the starter's statements.
    - It gets the stock workspace `desktop` only when it declared none (`finishLoad`).
    - A tree with no seat has no keyboard, so keys are left out.
    - The device settings its objects write reach the compositor (see "P6 config document and slots status").
- **Reload request path (`gnoblinctl config reload`).** Superseded by "JS-owned runtime role": the JavaScript role
  itself watches `session.reload_requested`, reloads the config, and answers the request with a `config.commit` or a
  `config.reject`. `GNOBLIN_JS_WATCH=0` (`JsRoleConfig::watch`) turns the watcher off.

    With no entry file `reload_now` is a successful no-op, which the role answers as a refusal because nothing was
    committed.

- **Proven against the real compositor** (`crates/gnoblin/tests/js_shell.rs`, in-process shell over a real headless
  compositor; `crates/gnoblin/tests/js_guardian.rs`, the shipped binary under its guardian; slice criteria 1, 3, 4, 5
  and 6): `init.ts` with annotations and an enum importing `./slots.ts` places two mapped windows; a rewrite reloads
  with the same window ids and new places;

    a syntax error leaves the old behaviour in force (a window mapped afterwards is placed by it) and is logged once as
    `init.ts:3:<col>`; fixing the file recovers;

    a `while (true)` in `onAdded` is interrupted and the generation restarts (the restaged one reads the mark the first
    wrote and does not hang), the scene answers an observer meanwhile and still updates;

    a handler that hangs in every generation (it hangs the restage too, while it loads) latches `Degraded` after three
    strikes and the last generation keeps placing windows.

    Criterion 6: with the shell gone (its owner connection closed in `js_shell.rs`; the runtime process `SIGKILL`ed in
    `js_guardian.rs`, where the guardian restarts it):

    - the compositor answers a fresh connection;
    - the Rust fallback policy places a window mapped meanwhile at the next step of the cascade (96, 96 after (32, 32) and
      (64, 64));
    - the fallback takes the two windows the shell had moved back to the places it gave them (it never knew the shell's);
    - the restarted shell reattaches, is told every window is ready, learns of each and places them again with their ids
      unchanged.

    Zero-config: with no `init.ts` the shell runs the stock default, and a user's `init.ts` replaces it whole; the file's
    appearing is a reload.

    `gnoblinctl config reload` against the real session with the watcher off: an edit alone changes nothing; the request
    exits 0 and the shell takes the file with the same window id; a syntax error exits 1 with `init.ts:3:` and the old
    generation places a new window; the fixed file recovers.

- **What the real tree changed in the binding.** The fake compositor's ids are small and its windows hang under the
  root; the real tree's are not.

    1. The compositor's own nodes have ids with bit 63 set, which a script number cannot hold.

        A node crosses into script as a number from `NodeHandles` (`bindings/mod.rs`): an id below 2^53 - 2^24 is its own
        number, any other gets a stable handle in the band above, for the life of the shell, so a reloaded generation sees
        the same number for the same window.

        `client.id` and the error targets print the real decimal id (`host.label`), the one `gnoblinctl` prints.

    2. The compositor makes a window with no parent and the shell parents it, so the collections are built from every
       node of the mirror (`TreeMirror::ids`), the root's tree first, and not from a walk of the root.

- **Protocol facts the real compositor showed.** It sends no change message for a transaction that changed no node (a
  call-only one still advances its serial), so `JsShell` moves the mirror to the serial of an applied reply
  (`TreeMirror::advance_serial`) and opens the in-flight gate for an applied transaction that published nothing;

    without it the next batch is a `Gap` or every later write waits for ever. The stock floating layout places an arrival
    after the listeners have run, so a config that wants its own listener to place windows declares a workspace with the
    base `Layout`, as the tests do.

- **Not done.** Input events and chords (the binding lane). The document, slots, `on` and `setup` are in "P6 config
  document and slots status"; `style` is in ["P8a style status"](js-runtime-log-ui.md#p8a-style-status); `outputs` is in ["Output modes status"](js-runtime-log-outputs.md#output-modes-status). `rules` are in
  ["Focus, rules and snapping status"](js-runtime-log-policy.md#focus-rules-and-snapping-status). Removing `init.ts` under a running
  shell keeps its generation until the session restarts.

## JS-owned runtime role

Lane JsOwnConfig, cutover steps 5 and 6 of `design/js-parity.md`. Before it the JavaScript shell ran inside the Lua
host: the Lua host located `init.lua`, validated and committed the whole document, answered `config.reload`, sent the
guardian's `autostart` list and read the keyring first-boot input, and the JavaScript shell laid its settings over what
the Lua host committed.

Now the JavaScript role does all of it and starts no Lua. Code: `crates/gnoblin-js/src/runner.rs` (`JsRoleRunner`,
`choose_runtime`), `crates/gnoblin-js/src/document.rs` (`evaluate_document`), `crates/gnoblin-config/src/locate.rs`,
`crates/gnoblin/src/main.rs`.

- **Selection (`GNOBLIN_SHELL_VM`).** `choose_runtime` is a pure function of the environment and the search path.

    Unset or empty: the JavaScript role, unless the located config is an `init.lua` (that is, there is no `init.ts` and no
    `init.js`), in which case the Lua role runs for this release and the binary logs one warning that names
    `docs/migrating-from-lua.md`.

    `js` and `js:<dir>`: the JavaScript role, always (with only an `init.lua` it runs the stock config and logs that the
    Lua file is not read). `lua`, `0` and any other value (a Lua shell directory): the Lua role, as before, with no
    warning because the user asked.

    The decision not to switch a Lua user silently is the owner's: no silent loss of their config. The variable is
    documented in `docs/typescript-shell.md`, and the Lua role is removed with step 8.

- **Locating the config.**

    `gnoblin_config::locate` takes the directory `GNOBLIN_CONFIG`, `$XDG_CONFIG_HOME/gnoblin` or `$HOME/.config/gnoblin`
    names and returns the first of `init.ts`, `init.js`, `init.lua` that is a file, with the others that exist in
    `ConfigPath::ignored()` (the JavaScript role warns that they are not read).

    With none it names `init.ts`. `GNOBLIN_CONFIG` names one file and its extension is its language. Every compositor-side
    reader of the last-good store uses the same function, so the store key and the file agree. `locate_lua` is the Lua
    host's: `init.lua` alone. There is no search of system directories: there is none today.

- **What the role does** (`JsRoleRunner::run`).

    It adopts the privileged connection the guardian passes, subscribes to the object graph (the `session` object's
    `reload_requested` is the signal it watches), builds a `JsRoleConfig` with `owns_document` set and a `JsCommitLog`,
    and boots `JsRole::connect`.

    The shell commits during that boot; the role then reports to the guardian, in the Lua role's order: one `autostart`
    line per enabled entry of the document, then `config-accepted REVISION`, then `ready` with `GNOBLIN_CONFIG_REVISION`.

    It then serves one frame per turn (`FRAME_WAIT`, 16 ms, is how long a turn waits on the compositor), reloads on
    `reload_requested`, and reports every commit the watcher's reload made.

- **The shell owns the document** (`JsShell::own_document`, `ConfigState::owns_document`). Each commit is the
  generation's document patch laid over an empty document: nothing the compositor held before is read, so a key deleted
  from `init.ts` is gone after a reload and after a runtime restart.

    Every load commits, even when the result equals the document in force, because the compositor holds a `config.reload`
    caller until a commit or a reject answers it.

    The document's `autostart` section is the guardian's: it starts the entries (named by the program's file name, `foot`,
    then `foot-2`), matches a running one by name and command (so a reload, or a restart of the runtime role, starts no
    second copy), stops one a reload no longer lists, and does not restart one that exits (`ChildSpec::autostart`).

    No `init.ts` object writes the section any more (the `autostart` setting was dropped, design/js-spec-native.md 3.11)
    and the shell starts no program itself.

    The keyring conflict rule (`enabled_autostart`, in `gnoblin-config`) still runs on the document the shell would
    commit, so a document that would start a second keyring is refused before the commit and the old generation keeps
    running.

- **Reload.** A request evaluates the entry in a staged generation and swaps it in when it evaluated cleanly (the
  machinery of ["Hot reload"](js-runtime-reload.md#hot-reload)).

    If the stage failed, or it evaluated and nothing was committed (the compositor refused the commit, or the file is
    gone), the role calls `config.reject` with the reason, which `gnoblinctl config reload` prints with the file, line and
    column, and exit 1. The accepted document and the running generation stay.

    A document refused by the compositor after the stage leaves the new generation running, as it did before. A shell link
    failure ends the role non-zero and the guardian starts it again.

- **Keyring first boot** (`crates/gnoblin-services/src/keyring.rs`, `decide`). Order: a configuration the compositor
  already accepted; an accepted configuration waited for (20 s); the last accepted file; the schema default. The user's
  `init.ts` is not evaluated: no object of the spec carries `keyring.enabled`, so a config has no say, and the services
  role no longer depends on `gnoblin-js` (`evaluate_document` stays for `gnoblinctl check`).
- **`gnoblinctl config validate FILE`** routes `.ts`, `.mts`, `.js` and `.mjs` to the same in-process check as
  `gnoblinctl check`.
- **Proven against the real compositor.** `crates/gnoblin/tests/js_runtime_role.rs` runs the shipped `JsRoleRunner` in a
  thread over a headless compositor: commit, then `config-accepted`, then `ready`; a config that fails to load runs the
  stock config and still commits;

    the twins of the Lua role's `runtime_reload.rs` (a script's `reloadConfig()` commits what the file says now, a broken
    file keeps the accepted document and the role); `config.reload` answers with the revision and a removed file is
    refused with why; the role ends cleanly when the compositor goes; and the shipped starter loads (the twin of
    `starter_config.rs`).

    `crates/gnoblin/tests/js_guardian.rs` runs the binary under its guardian with `GNOBLIN_SHELL_VM` unset and no Lua
    anywhere. It covers:

    - `init.ts` committed as revision 1, `config reload` and the watcher;
    - a killed runtime committing what the file says and not what it left behind;
    - the Lua fallback with one warning;
    - an `init.ts` beside an `init.lua`;
    - `GNOBLIN_SHELL_VM=lua` and `js` with only an `init.lua`.

    `crates/gnoblin/tests/guardian.rs` is unchanged and now runs the JavaScript role (no `GNOBLIN_SHELL_VM`): the killed
    runtime restarts and commits revision 2.

- **Faked.** Nothing in those tests. The unit tests of `ConfigState` use a fake document link (no compositor), and
  `choose_runtime` is tested as a pure function.
- **Not done.** The shared-document mode of `ConfigState` (the shell lays its settings over a document another host
  committed, `owns_document = false`, `JsRole::config_committed`) has no product caller any more; the shell tests still
  run in it (about fifty tests in `crates/gnoblin-js/tests` and `js_shell.rs`), so it goes when they move to the owned
  mode.

    `scripts/xr-smoke.sh` still selects the Lua stock directory (a Lua shell directory still selects the Lua role). The
    first-boot Secret portal still reads the last accepted file (`design/e2e-keyring.md`).

## P6 config document and slots status

Lane JsConfigSlots, as it stands after the declarative config layer was removed (design/js-spec-native.md). The settings
a script writes through the spec objects reach the config document, and a script can observe and refuse the compositor's
own dialogs.

Code: `crates/gnoblin-js/src/config.rs` (the document), `crates/gnoblin-js/src/dialogs.rs` and
`data/js/gnoblin/dialogs.ts` (`gnoblin.dialogs`), `JsShell::apply_document`/`run_dialogs`, `JsRole` (the connections).
Tests: `crates/gnoblin-js/tests/config_slots.rs` over fakes, `crates/gnoblin/tests/js_shell.rs` against the real
compositor.

### Where the settings of the first design went

The first design carried the settings of the config document as keys of a plain-data config. Each now has the object the
owner's spec gives it, or is dropped:

| Setting                                                    | Now                                                                                                         |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `gaps`, `workspaces`, `keys`, `gestureActions`, `launcher` | `Workspace.gaps` and a layout's `gap`, `gnoblin.workspaces.add`, `input.bind` on a key or a gesture trigger |
| `decorations`, `menus`, `alerts`                           | `startDecorations()`, `startWindowMenu()` and `startNotRespondingAlert()`                                   |
| `on`, `setup`                                              | `gnoblin.clients.addListener(...)`, and the body of the file                                                |
| `style`                                                    | `gnoblin.clients.style` and `gnoblin.clients.where(...).style`                                              |
| `outputs`                                                  | `OutputProfile` and `Output.apply` (["Output modes status"](js-runtime-log-outputs.md#output-modes-status)) |
| `input` (the device settings)                              | `Keyboard`, `Mouse` and `Touchpad` properties, written to the document's `input` section                    |

Dropped, because no object in the spec carries them (design/js-spec-native.md, 3.11, says what a home would need):

- `rules`, `focus`, `edgeTiling` and `dragConstraint`.
- `autostart` and `workspaceSets`.
- `protocols`, `touchpadGestures` and `dialogs.style`.
- The other sections of the document.

The per-setting rule behind the compositor's answers is `crates/gnoblin-config/src/apply.rs` (`RULES`).

### The document commit

- **The patch.** The only writer of the document is the device objects (`Keyboard.repeatRate`, `Mouse.sensitivity`, ...):
  `deviceDocument()` is the `input` section they wrote, the entry exports it as `document()`, and `documentRevision()`
  moves on every accepted write so the shell commits again.
- **The shell's connections.** `JsRole::connect` opens two owner connections besides the tree's: a `ClientSlotLink` (the
  one the Lua role uses) and a `JsClientConfigLink`, which reads `config.current` and writes `config.commit`, the two
  verbs the Lua host uses. `JsRole::over_link` takes them as `JsRoleLinks`; each is optional and the role runs without
  it (the compositor's own dialogs serve, and the document keeps what the Lua host committed), reported once.
- **Full document, not a patch.** The compositor holds one document and derives its settings from each one it accepts,
  so a commit must carry everything. The shell commits the document in force with the generation's patch laid over it,
  and remembers the key paths the patch set.

    The next commit starts from the document in force with those paths taken out (a table the removal empties goes with
    it) and the new patch laid on.

    That is the generation-overlay rule of spec-review 1.27: a reload that drops a `Keyboard.repeatRate` write puts it
    back to its default, and a setting the shell never made (the Lua file's) is not touched. A document that would not
    change is not committed.

- **Validation and where it is reported.** `Config::from_value`, the validator the compositor and the Lua host use, runs
  on the document the commit would send.

    It runs when a device setting is written, which may be while a reload is staging: `host.config.check` makes the setter
    throw (`the configuration refused it: input.keyboard...: ...`, the document's key path), the entry fails,
    `ReloadReport.errors` carries it with the file and line, and the running generation and the document stay.

    The compositor refuses nothing here itself: it records and parses a committed document and keeps its last good
    settings when one does not decode, so the check is the runtime's.

    A commit the compositor does refuse (it can, for `xwayland`) happens after the swap, which cannot be taken back: it is
    a diagnostic with the compositor's reason, and the notices of an accepted commit (`not applied: ...`) are diagnostics
    too.

- **The Lua host's reload commits too.** `gnoblinctl config reload` reloads the shell first and then the Lua host,
  which commits a document with none of the shell's settings. `ShellFrame::config_committed` (called by the Lua
  role right after its commit, before it announces the reload) makes the shell lay its settings back over the document
  in force. For the sliver between the two commits the settings are at their defaults.

### Slots and `gnoblin.dialogs`

- **The slot.** `dialogs.observe`, from design `popups-portals.md` 7.5: the compositor shows the shell each consent and
  passphrase request for 250 ms before its own dialog opens, under the title `consent:<interface>` or `secret`. The only
  answer it reads is `deny`, which refuses the request by policy (the requester sees `cancelled(user)`); anything
  else, silence, a decline or a departure leaves it to the built-in dialog.
- **Claim and release.** The shell claims the slot while the running generation has a dialog listener and gives it back
  when it has none, so a config with no listener never delays a dialog. A refused claim, or no connection, is reported
  once and not retried until the listeners are gone.

    `JsShell::retire` (a reload swap, a restage, a shutdown) answers each observation it still holds `pass` and releases
    the slot; the next generation claims it again on its first frame. A crash releases it, because the compositor releases
    a connection's claims when it closes.

- **Per frame.** `JsShell::frame` polls the `SlotLink` once, hands the offers to `entry.dialogs`, runs the queued
  listener calls under the usual budgets, takes `entry.takeVerdicts` and sends each: `claim_request`, then `answer` with
  the fixed bytes `deny` or `pass`.

    Nothing a script wrote is ever an answer, and the slot has no way to carry a passphrase. A `finished` event (the
    window ended) closes the request, so a late `decline` refuses nothing.

- **API.** `DialogListener` with optional `onConsentRequest(request)` and `onSecretRequest(request)`;
  `gnoblin.dialogs.addListener/removeListener`. A request has `kind`, `appId`, `subject`, `deadline` and `decline(reason
= "policy")`. `appId` is the compositor's verified id.

    `subject` is the portal interface of a consent request and the app's own prompt text of a passphrase request. There is
    no method that allows a request and no access to a secret.

    One queued call per listener, then one that settles the request, so a listener that throws or runs out its budget
    costs the others nothing; the failure is reported once per site.

- **Budget.** A listener that runs past `entry_budget` is interrupted, counts a strike (the supervisor restages the
  generation after it, as for any listener) and the request passes. The compositor's window is 250 ms in any case: a
  shell that is wedged costs the user at most that.

### Proof

Against the real compositor (`crates/gnoblin/tests/js_shell.rs`): the device settings of a `Keyboard` are in the
document the compositor holds, read back through a second owner connection (`repeatRate` is the document's
`repeat-interval` in milliseconds); a `repeatDelay` of 20000 is refused by the setter, which names the key, the
generation stays and the document is untouched;

a reload without the write removes it from the document. A listener that declines a request from one app refuses it (the
requester gets `cancelled(user)` without anything withdrawing it) while a request from another app stays open for the
built-in dialog;

a listener that never returns is interrupted, the request is not refused and the built-in dialog serves it, the
generation restarts and the next request is refused again.

Against fakes (`crates/gnoblin-js/tests/config_slots.rs`): the patch, the removal and the no-op commits, a foreign
commit, a refused commit, notices, the missing links, the claim and release, the verdict words, isolation, the budget, a
held request at `retire`, a lost connection.

What the real tests do not show: that the built-in dialog opens is shown by the request staying open until its
requester withdraws it, not by a screenshot of the dialog; `retire` answering a held request is proven against the
fake link only (the real compositor answers within 250 ms, so a request is rarely held across a swap).

### The starter and `config init`

Lane JsConfigLists, as it stands.

- **The starter.** `data/js/starter/init.ts` (embedded as `STARTER_CONFIG`) holds the statements of `installDefaults`. Its
  examples are comments that end the way a line of code ends, so a test can switch them on. `starter_parity.rs` holds the
  file and `installDefaults` together.
- **`gnoblinctl config init [DIR] [--force]`.** It writes the starter, refuses when `init.ts` or `init.js` exists (an
  `init.ts` is read first and would hide `init.js`), runs `types init`, prints three steps, and has a `--json` form
  (`crates/gnoblinctl/tests/js_check.rs`).
- **Gone.** The lane's config keys, shape checks and `rejectUnknown` messages went with the declarative config layer (see
  "Where the settings of the first design went").
- **Shortcuts.** The document's `shortcuts` is not a key: bind a key with `input.bind`. A release shortcut has no
  equivalent, since a key acts on press, so it stays in `init.lua`.
- **Not done.** Typed completions for nested document settings: only the device settings of `input` reach the document.
