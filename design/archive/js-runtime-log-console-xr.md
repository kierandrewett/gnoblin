# JS runtime change log: console and XR

The finished work of the live console (P8), the console global and XR (P9). Part of the [JS/TS runtime
design](js-runtime.md).

## P8 console status

Lane JsConsole: `crates/gnoblin-js/src/console.rs` (the server), `data/js/gnoblin/inspect.ts` (the renderer and the
helpers), `crates/gnoblinctl/src/console.rs` (the client). User page: `docs/js-console.md`; the first of the
introspection tools of owner-spec section 18.

- **Server.** `JsConsole` listens on `js.sock` in the directory of the owner socket (`console_socket_path`: the path
  `socket_path` gives, with the file name replaced, so it follows `GNOBLIN_IPC_SOCKET` and `XDG_RUNTIME_DIR`), mode 0600.

    `JsRole::connect` binds it unless `GNOBLIN_JS_CONSOLE=0` (`JsRoleConfig::console`: `Environment`, `Off` or `At(path)`;
    a role built over a fake link has none unless told where).

    A shell that cannot bind one (the path is served by another shell, or not writable) logs a warning and runs without
    it. A stale socket file is replaced; a live one is refused. The file goes with the server.

- **Peer check.** On every accept the peer's uid is read with `SO_PEERCRED` (`rustix`, as `ipc/privileged.rs` does) and
  compared with the shell's own: another uid, or credentials that cannot be read, is closed unanswered (`peer_allowed`,
  unit-tested; a second uid cannot be made in a test). At most 16 connections; a request line is at most 1 MiB.
- **No thread.** The server is nonblocking and polled by `JsRole::tick` before `JsShell::frame`, so an expression runs
  on the shell's own thread between two frames, never beside one (J5's single-threaded runtime needs no lock), and its
  writes (`gnoblin.clients.at(0).x = 10`) leave in the very next frame's transaction.

    At most four expressions are evaluated per turn, and a connection that has not read its replies is not evaluated for.
    The cost is the loop's own cadence: a request waits for the next turn.

- **Budget, no strike.** An expression runs under `JsShellConfig::entry_budget` (1 s), the listeners' budget (not the 2
  s the research note suggested for a REPL: a console that holds the shell for 2 s is a visible stall).

    A trip answers with a `budget` error and is not counted as a strike (`console_target` does not call `count_strike`):
    the person at the prompt is not a listener the supervisor should restage the generation over, and the restage would
    drop the session. The interrupt is uncatchable, so `for (;;) { try { while (true) {} } catch {} }` is stopped too.

- **Session.** A script is evaluated in sloppy mode as `<console>`: `var`, `let` and `const` at the prompt, and an
  assignment to an undeclared name, are globals of the running generation and persist for every connection until a
  reload; the reload makes a new generation, so nothing the console defined survives it and nothing is written to disk.

    Not per connection: a connection-scoped session would need a scope object per socket and a `with` wrapper, which makes
    `let` at the prompt behave unlike a script. A leading `{` is read as an object literal when the text parses that way
    (oxc), so `{a: 1}` is the object.

    A promise the expression returns is shown by its result after the pending jobs run (at most 1000), and `await` at the
    top level is not supported. TypeScript is not stripped.

- **Rendering.** `entry.render` (the host's `inspect.render`) walks the value once and returns the JSON text of `{ type,
value?, text, truncated }`: `value` is plain JSON data, `text` is how a terminal shows it.

    A cycle is `[Circular]`; a function is `[Function name]`; `NaN`, the infinities and `-0` travel as their names; a
    class instance carries `$class`.

    The host registers its kinds with `inspect.register`: a `Client` is `Client#3 { appId, title, focused, ... }` (data `{
"$class": "Client", "id": "3", ... }`) and only app id and title when it is one of many; a `Workspace` likewise;

    a `Collection` is an array of those briefs. A getter that throws shows as `[Error: message]` for that field. The cap
    is 64 KiB of text (separators count): what does not fit is replaced by `... N more` and `truncated` is true. Strings
    are cut at 4096 characters.

- **Errors.** `{ id, ok: false, error: { kind, message, location, stack } }`; `location` is the first stack frame in
  `<console>` (so `null.x` on line 3 says line 3, not a frame of the library under it), else the error's own location.
  A syntax error has one too.
- **Inspector.** `inspect.clients()`, `inspect.workspaces()` and `inspect.bindings()` return plain data.

    `bindings()` is `InputContext.describeBindings()` over every seat. Each entry has:

    - id, trigger and kind;
    - owner (`context`, `layout` or `device`) and the layout's class or the device's name;
    - `priority`, `enabled` and `rank`, which orders the registrations of one trigger by the comparator a press uses (tier,
      then priority, then registration order).

    `gnoblin.clients.focused` (a getter on `Collection`) is the seat's focus, and with no seat the member whose own
    `focused` is set.

    `inspect` is a global of the generation (non-enumerable, like `gnoblin`) and is exported by `gnoblin` for `init.ts`.

- **`gnoblinctl console`.**

    A plain line loop over standard input (`gnoblinctl` has no line-editor dependency and the deps gate would count one).
    It has:

    - the terminal's own editing;
    - an in-memory history (`.history`, `!!`, `!N`);
    - `.exit` and Ctrl-D;
    - a `...` continuation after an error that says "end of input";
    - error blocks on standard error with the location and a hint.

    `--eval` prints the value as JSON and exits 0 or 1 (`undefined` prints `null`; `--json` prints the whole reply); with
    no shell it is an error block and exit 1, not 3, because the compositor may well be running. Without a terminal on
    standard input there are no prompts and a failing line exits 1. Arrow-key history needs `rlwrap`.

- **Proven against the real compositor** (`crates/gnoblin/tests/js_shell.rs`): with the stock config over two mapped
  windows, the socket is mode 0600 and the console answers:

    - `gnoblin.clients.length` (2), the app ids, a `Client` and a collection;
    - `clients.focused` equal to the seat's focus;
    - `inspect.bindings()` with the stock `XF86AudioMute` (owner `context`, rank 1) and `inspect.config()`.

    A syntax error is an error at `<console>:1` and the shell is as it was; `while (true) {}` is a `budget` error, then
    the console answers, a window mapped afterwards is seen (3 clients) and the generation id has not changed;
    `gnoblin.clients.at(0).x = 10` is in the compositor's scene. `GNOBLIN_JS_CONSOLE=0` binds no socket, and the socket
    goes when the role is dropped.

- **Proven against the fake link** (`crates/gnoblin-js/tests/shell.rs`, `crates/gnoblin-js/src/console.rs`,
  `crates/gnoblinctl/tests/console.rs`): the values and their rendering listed above; the cap and `truncated`; error
  locations on lines 1 and 3; bad requests answered without closing; a runaway with a 50 ms budget, a stubborn one, and
  no strike; session variables;

    a role that serves the console between frames and drops the session on reload; `inspect.bindings()` empty on a tree
    with no seat; the uid predicate; a stale socket file replaced and a live one refused.

    The client against a scripted server: the value as JSON, `undefined` as `null`, failures with the place and a hint,
    the truncation warning, history and recall, continuation, piped scripts, `--json`, a shell that goes away.

- **Not done.** `complete`, `inspect{path|id}`, `gens`, `modules`, `handlers`, `timers`, `stats`, `reload` and `kill` of
  the research note's protocol; top-level `await`; TypeScript at the prompt; per-connection sessions; the inspector's
  views of computed styles, transition tracks and layout placements (the object model has none yet); `displays` and
  `extensions` expressions of spec section 18 (no such objects).

## Console global status

Lane JsConsoleGlobal: `crates/gnoblin-js/js/console.js` (the formatting), `crates/gnoblin-js/src/script_console.rs` (the
limit, the call site and the log), `crates/gnoblin-js/tests/script_console.rs`. User pages: `docs/typescript-shell.md`
("Debugging") and `docs/js-console.md`. A user's first line `console.log("hi")` threw `ReferenceError` until this.

- **Every generation has it.**

    `JsGeneration::new` evaluates `console.js` as a function expression and calls it with three host functions (`admit`,
    `emit`, `remap`).

    So `init.ts`, a module, a listener, a console expression and the throwaway generation of `evaluate_document` all
    share one global, defined non-enumerable.

    It formats in JavaScript because the work is about JavaScript values, and it never throws at its caller: a value that
    cannot be printed (a getter that throws, a `toString` that throws) prints a note instead.

- **Methods.** `log`, `info`, `warn`, `error`, `debug`, `assert` (error level, `Assertion failed`, no throw) and `table`
  (the value as one line of JSON, not a drawn table). No `trace`, `dir`, `group` or `time`: the d.ts declares what exists.
- **Formatting.**

    It formats like Node:

    - strings, and numbers (`-0`);
    - `%s %d %i %f %j %o %O %c %%` (a first-string format applies only when there are further arguments);
    - an `Error`, which prints `Name: message` and its stack with every frame remapped to `.ts` coordinates (QuickJS's
      `stack` has frames only, so the head is added).

    An object is one line of JSON-like text:

    - keys and strings quoted; `undefined`, functions (`[Function: name]`), symbols and `NaN` named;
    - a cycle printed as `[Circular]`; a `Map`, `Set` and `Date` shown;
    - a class instance prefixed by its class name;
    - depth 6 and 100 items per level.

    It does not reuse `inspect.render`: that is the host's renderer (a `Client` as `Client#3 { ... }`) and is not there in
    a bare generation, and one formatter that behaves the same in every generation is worth more than the nicer text.

- **The log.** Component `js`, field `area=script`; `log` and `info` at info, `warn` at warn, `error` at error and `debug`
  at debug (the process default is info, so `console.debug` shows with `GNOBLIN_LOG=js:debug`). The message is
  `file:line:col: text`. One call is one record, which may hold newlines.
- **The call site.** The first frame of `new Error().stack` that is not in `console.js`, remapped through the
  generation's source maps (the same `remap` the error stacks use, now a free function `remap_stack`), with the file
  shown relative to the config root.

    The engine reports a call at its end and the source map rounds that to the nearest token before it, so the line is the
    call's, and the column is a token inside it: the last argument when that is an identifier (`init.ts:2:29` for
    `console.log("a", gnoblin.clients.length)`, the start of `gnoblin`), else the callee's property (`log`).

    A call that spans lines reports the line of its end.

- **Caps.** At most 4096 bytes of text per call, cut on a character boundary with `... (N more bytes)`. At most 200
  calls a second per generation: the rest are dropped before they are formatted (`admit`), and when the next second
  starts, or a frame ends after it, one warn line says `N console lines dropped`.

    The second is counted on `JsConfig::clock`, which `JsShell::new` sets to the shell's clock (`JsShellConfig::clock`),
    so a test moves it and no real time passes; the frame calls `flush_console` so the notice does not wait for a script
    that went quiet.

- **The prompt.** `JsConsole` starts a capture around each expression (`capture_console`, `take_console`), and every
  reply, ok or not, carries `output`: `[{ level, site, text }]` in order (`site` is `<console>:line:col`).

    `gnoblinctl console` prints `log`, `info` and `debug` lines on standard output and `warn` and `error` on standard
    error, before the value; with `--eval` all of them go to standard error so standard output stays the value.

    The lines are in the log too.

- **d.ts honesty.** `declare const console` is in `gnoblin-stock.d.ts`.

    `tests/dts_match.rs` now compares the value exports of `declare module "gnoblin"` (parsed from the d.ts: class,
    function, const, namespace, enum and re-export names) with `Object.keys` of the runtime module namespace in a booted
    shell, both ways, each exception in an allow-list with its reason (`DECLARED_NOT_IN_RUNTIME` names the lane that lands
    it and fails once it has; `RUNTIME_NOT_DECLARED` is plumbing the stock modules share).

    The first run found:

    - `ArcLayout` and `WorkspaceSwitchBinding` declared with no runtime (removed; they live on in owner-spec.md only);
    - a second `class Transition extends NativeObject` and `Easing` from the proposal beside the real `Transition`
      (removed, with the `delay` and `autoplay` options the real one refuses);
    - `NativeObject`, `MemberListener` and `DragBinding` declared as importable base classes the runtime does not have
      (now not exported: types only);
    - `StyleElement` real but not re-exported (now exported by `gnoblin`);
    - `GnoblinError` exported and documented but not declared (now declared).

    The header of the d.ts no longer says "PROPOSED API".

- **Proven.**

    `tests/script_console.rs` (bare generation and shell over the fake link, reading the log with the level of each
    record) covers:

    - the levels, the `.ts` call site with a type annotation stripped from the line, and the specifiers;
    - objects, cycles, errors with stacks, `assert` and `table`, and a hostile value;
    - the byte cap, the 200-a-second limit and its notice on the injected clock, and the notice at a frame end;

    `tests/shell.rs` (the reply's `output` and the log from a console expression, including a throw after output);
    `crates/gnoblinctl/tests/console.rs` (where each level is printed);

    `crates/gnoblin/tests/js_shell.rs` (a real compositor: a load-time call and a listener's call are in the session log
    as `init.ts:2:29` and `init.ts:8:34`).

- **Not done.** `console.trace`, `dir`, `group`, `time` and `count`; a table drawn as columns; the host's `inspect` text for
  a `Client` in `console.log` (it prints its own enumerable fields); a per-generation log of the last lines for
  `gnoblinctl console` to replay.

## P9 XR status

Design: `design/js-xr.md` (which spec class is a tree node, a JS class or new native work). User docs: `docs/xr.md`,
"Scripting XR in TypeScript". No headset, OpenXR runtime or `gnoblin-xr` binary was used; the program was not run.

- **Scene classes.** `data/js/gnoblin/scene/` has `Node`, `Camera3D` and `View` over the tree (`__gnoblinHost.xr`,
  `crates/gnoblin-js/src/xr.rs`: make, insert, destroy and find nodes of those three classes; ids come from the UI
  toolkit's count; `bindings/props.rs` gains the `node`, `view`, `camera` and `controller` property tables).

    A write of what a node already holds is left out, because the tree sends every write it is given. `Vector3`,
    `Transform3D`, `Ray` and `Plane` are the pose arithmetic; a rotation is the program's, Euler about X, then Y, then Z.

- **The module.**

    `data/js/gnoblin/extensions/xr/` (`exports.ts` has all its public names; `index.ts` is the guarded import path) is the port of `xr.lua` and the spec's classes:

    - from `xr.lua`: `arrangeRing` with the Lua golden numbers, `XRRoom`, `SpatialLayout`, panels for clients, none for
      dialogs or secure clients, head-locked panels;
    - from the spec: `XRCamera3D`, `XRViewport`, `XRSession`, `XRExtension`, `XRController`, `ViewMoveBinding`,
      `ViewResizeBinding`.

    It runs once a frame from the host's UI entry and costs one `seats()` call while no program has a session.

- **Presence.** `gnoblin.extensions.xr` is null until the seat named `xr` exists and null again after it goes; a held
  session reads `alive === false`. `gnoblin/extensions/xr` fails at load unless `JsShellConfig::xr_declared`
  (`GNOBLIN_XR=1`, read in `crates/gnoblin/src/main.rs`) or the seat exists.
- **Bindings.** `Binding.readTrigger` is a static hook, so an XR binding keeps the `Binding` lifecycle, `enabled`,
  `priority` and the `BindingCollection` API with its own trigger grammar (`ControllerTriggerParts`, `Grip+Drag`). No seat
  claims one. They are in `SpatialLayout.bindings`, which a script turns off and replaces like any collection.
- **Proven against the fake tree** (`crates/gnoblin-js/tests/xr.rs`, 50 cases): absent XR, the import gate, XR1, the
  Lua room cases and ring goldens, XR3 move (View only), the ray-drag, XR4 resize (zero window writes while dragging,
  one transaction on release, none when aspect and density hold), cancel, throw, disable and replace, XR6 secure, the
  `docs/xr.md` snippets.
- **Proven on the real compositor** (`crates/gnoblin/tests/js_shell.rs`, the two XR tests): an observer makes the `xr` seat,
  the stock shell builds the room with a head camera and a View per real window, and the room goes with the seat. The
  seat is the only thing of the program in that test.
- **Controllers (lane XrControllers).** The program publishes `controller` nodes (`design/js-xr.md`, "New native work",
  now built): the `controller` schema class, an observer may make and write it, `extensions/gnoblin-xr` writes one for
  each hand and obeys `controller.claimed`. Proven against the fake runtime and a fake owner, and on the compositor's
  wire (`tests/tree_ipc.rs`).
- **Claims (lane JsXrClaims).** The host converts a script's array of strings to the tree's `list<string>` value
  (`JsScriptValue::Texts`, checked against the property's allowed names) and `controller.claimed` is writable.

    `XRController.claim(owner, names)` and `release(owner)` keep one claim for each owner and write the union; the runtime
    claims the buttons of a drag binding's trigger when `onStart` accepts and releases them on end, cancel, a throw, the
    binding's removal, the session's end and `retire`.

    Proven on the fake link (`crates/gnoblin-js/tests/xr.rs`, the "Claims" cases): claim and release of both stock
    bindings and a ray drag, two owners, the typed errors, every way a drag ends and `retire`.

    Proven on a real compositor (`crates/gnoblin/tests/js_shell.rs`, `the_stock_grab_claims_the_grip_...`): an observer
    plays the program.

    The stock shell's `ViewMoveBinding` writes `claimed = ["grip"]` on the real `controller` node, the compositor refuses
    the observer the same write, and the list is empty again when the drag ends and when the shell dies.

    A shell's detach now clears the `claimed` of every controller (`reap_transactions`). The program's withholding of a
    claimed grip from the window is proven against the fake runtime and a fake owner only (`extensions/gnoblin-xr`,
    `bridge/controller_nodes.rs`, `xr-input/tests/hands.rs`); no real program, runtime or headset ran it. **The claim now
    comes before the press (lane XrGrip; design/js-xr.md item 6).**

    The program holds every press of the trigger, grip, face button and stick for `CLAIM_HOLD` (150 ms of display time)
    and forwards it only if no claim came, a tap as a press then a release, so a grip the shell claims never reaches the
    window and an unclaimed one reaches it at most one frame after the hold.

    `session.chordSettleMs` is a getter and setter capped at 60 ms, so the claim fits inside the hold
    (`the_chord_settle_time_is_a_number_...`, `a_drag_claims_its_grip_after_the_settle_time_...` in `tests/xr.rs`). The
    real-compositor claims test is unchanged: an observer plays the program there, so it cannot show the program's hold,
    which is fake-runtime only. No headset was used.

- **The spec's XR surface (lane SpecXr; design/js-spec-native.md, 2.7).**

    `data/js/gnoblin/extensions/xr/bindings.ts` holds the trigger grammar, `XRControllerDragBinding` (exposed as
    `xr.ControllerDragBinding`) and the four stock bindings: `ViewMoveBinding`, `ViewResizeBinding` (now with `edges`),
    the new `ViewRotateBinding` (the layout's `rotate` trigger, `Trigger+Thumbstick+Drag`) and `ViewScaleBinding` (added
    by a script).

    `SpatialLayout.bindings` stays a plain `BindingCollection`: a duplicate id is refused, so fences 066, 068 and 117
    still fail on it (an owner decision).

    The runtime now pins the View under the ray for every drag and unpins it on a cancel unless it was pinned before, so a
    script's own binding (snippet 067) does not have to know about the ring.

    `VirtualOutput` (`data/js/gnoblin/virtual_output.ts`) is a checked description; `gnoblin.outputs.add` throws a
    `GnoblinError` with code `unsupported` because the compositor has no request that creates an output.

    Proven on the fake link in `tests/xr.rs`, the owner's snippets 065, 067 and 069 to 071 run verbatim and 118 fails only
    on that typed error. No headset was used.

- **Not built.** The TypeScript rules have no `xr = false` key (`accept` replaces it); `ThrowViewBinding` (the spec
  makes it a user class with a physics stepper of the author's); a View of a workspace; a compositor-made virtual
  output. XR5 (a stalled script does not block pose writes or layers) is unverified.
