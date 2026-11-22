# JS runtime: time units and error codes

Milliseconds everywhere, one easing vocabulary and one set of error codes for the JavaScript API. Part of the [JS/TS
runtime design](runtime.md).

## Decisions

The owner's specification says "milliseconds (the JS API uses ms consistently)" and names its easings the CSS way
(`design/js/research/owner-spec.md`, "TransitionOptions"). The specification gives no error codes, so the one scheme is
the one the host already spoke and `docs/shell-api/errors.md` documented.

- **Times.** Every duration, delay and timeout of the script API is milliseconds.

    Before the lane, these were seconds:

    - `Client.animate`, `gnoblin.show`, `gnoblin.motion` and `gnoblin.after`;
    - `Operation.timeout` and the `timeout` of a spawned process;
    - the slide binding's `seconds`, the toast's fades, the alert's `armSeconds` and the focus decision's delays.

    These were already milliseconds: `requestTransition`, `Client.set`, `gnoblin.timers.after`, `idle.watch` and the launch
    deadline.

    `host.animate` carries milliseconds too, and `bindings/host.rs` converts to the seconds the tree's wire counts. The
    exception is a drag event's `deltaTime`, which the specification gives in seconds.

    `Client.animate` is gone too, as `gnoblin.after` is: the specification has neither. A client's timed write is
    `Client.set(values, { duration, easing })`, which is one `requestTransition`, so the transition engine owns every
    timed property and the newest write supersedes the older (`Client.animate` wrote to the tree on its own and ran
    beside the engine). The shell's own timed writes go through the same engine: the workspace slide
    (`TransitionEngine.play`, which begins inside a layout pass, where `requestTransition` cannot) and the swipe's slide
    back (`requestTransition`).

- **Easings.** Two vocabularies (`out`, `out-cubic`, `in-cubic`, `in-out`, and the CSS aliases `ease-out`, `ease-in-out`,
  `ease-in`) became one: `Easing` is `linear`, `ease`, `ease-in`, `ease-out` and `ease-in-out`, each the CSS timing
  function of that name (a cubic Bezier, `data/js/gnoblin-std/transition/easing.ts`).
- **Error codes.** Three schemes existed. (1) `GnoblinError.code` held untyped kebab-case strings: `destroyed`,
  `read-only`, `refused`, `detached`, `busy`, `no-output`, `unknown-workspace`, `not-a-member`, `unsupported`,
  `no-seat`, `duplicate`, `spawn-failed`, `forbidden`, `nested`, `output-conflict`. (2) `OperationErrorCode` held
  snake_case codes (`invalid_argument`, `not_found`, `timed_out`, ...), and the host's process and session calls speak
  them.

    (3) Bespoke classes carried none: `TriggerError`, `BindingConflictError`, `SceneError` (whose `code` was the host's
    own word).

    One `ErrorCode` (`data/js/gnoblin-core/errors.ts`) replaces them: the snake_case set, extended with `already_exists`,
    `invalid_state` and `read_only` for what the kebab codes said that it could not.

- **The migration.** `destroyed`, `unknown-workspace` -> `not_found`; `read-only` -> `read_only`; `detached`,
  `forbidden` -> `invalid_state`; `duplicate` -> `already_exists`; `no-output`, `no-seat` -> `unavailable`;
  `not-a-member` -> `invalid_argument`; `nested`, `output-conflict` -> `busy`; `refused` -> `denied`, or the code the
  host's word maps to, and `config_rejected` for the configuration document's refusal; `spawn-failed` -> the process
  host's own code (`not_found`, `busy` or `internal`).

    The host's words for a refused write (`unknown-node`, `unknown-property`, `unknown-method`, `read-only`,
    `invalid-value`, `not-animated`, `no-seat`) stay between the Rust bindings and the library; `errorCodeOfRefusal` is
    the one place that translates them, and they appear only inside messages.

    `SceneError` extends `GnoblinError`; `TriggerError` and `BindingConflictError` carry a `code`.

- **Proof.** `crates/gnoblin-js/tests/time_units.rs` fails when a script-facing time is documented in seconds or an
  old easing name returns. `crates/gnoblin-js/tests/error_codes.rs` fails when a thrown code is not one of the
  documented ones, when `docs/shell-api/errors.md` and the declarations list different codes, or when an old kebab-case
  code comes back.
