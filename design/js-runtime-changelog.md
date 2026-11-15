# JS runtime change log

The finished work of the JavaScript shell, one section per lane, kept as it was written when the lane landed. Each
section names the tests that prove it and what it left undone. Part of the [JS/TS runtime design](js-runtime.md).

## Logs by topic

- [Types and the stock library](js-runtime-log-library.md): work packages P2 (types) and P4 (the stock library and its
  layouts).
- [Tree binding, input and workspaces](js-runtime-log-input.md): the tree binding (P3), key and gesture bindings (P3c),
  pointer chord claims and workspaces (P3b).
- [Role wiring and the config document](js-runtime-log-config.md): the runtime role (P6), the JS-owned role and the
  config document, slots and starter.
- [Focus, client requests and window policy](js-runtime-log-policy.md): focus, window rules and snapping, the answers to
  client requests, window policy and tiling bindings.
- [Outputs, modes and displays](js-runtime-log-outputs.md): outputs and desks, output modes and displays.
- [Host API, polkit, lock and the object model](js-runtime-log-host.md): the session, process and timer API (A), idle
  watches, authentication and launch feedback (B), polkit, the lock screen, input contexts and the scene.
- [UI toolkit, stock shell UI, style and transitions](js-runtime-log-ui.md): the UI toolkit (P7), the stock shell UI,
  window style (P8a) and transitions.
- [Console and XR](js-runtime-log-console-xr.md): the live console (P8), the console global and XR (P9).

Time units, error codes, budgets, performance and live verification have their own documents, listed in the
[runtime design](js-runtime.md#where-the-rest-lives).
