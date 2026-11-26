# Compositor Bridge

Connect a shell or session tool to Gnoblin’s compositor through the local Unix
socket. The bridge provides versioned snapshots, events, and operations.

This reference describes the socket the Mutter-based compositor serves. Its
"shell runtime" is the shell VM the session runs. A shell can be written in
TypeScript, Python or Rust; see the [TypeScript shell](/typescript-shell),
[Python shell](/python-shell) and [Rust shell](/rust-shell) references.

## Start here

- [Connect and frame requests](/bridge/connection): socket path, greeting,
  request format, API versions, limits, and reconnect behavior.
- [Sessions and Windows](/bridge/session-and-windows): snapshots, session
  state, locking, thumbnails, layers, and shell UI sessions.
- [Input and focus](/bridge/input-and-focus): input sources, trusted contexts,
  text insertion, snapping, and focus policy.
- [Shortcuts and events](/bridge/shortcuts-and-events): event subscriptions,
  shortcut bindings, and operation completion.
- [Privacy](/bridge/privacy): privacy state, monitor privacy screens, and
  location authorisation.
- [Runtime and appearance](/bridge/runtime-and-appearance): runtime health,
  orientation lock, and desktop appearance.
- [Examples](/bridge/examples): a shell command, and raw socket clients in
  Node.js and Go. No Python or Rust bridge client library exists yet.

## Choose a transport operation

Use `windows` or `monitors` for a snapshot stream. Use `events` to subscribe to
named events. Use `api` for one versioned method call. The [connection
reference](/bridge/connection) documents each request and the server greeting.

Before calling an optional method, check `hello.methods`, `hello.events`, and
`hello.capabilities`. The [shell API reference](/shell-api/) documents the
same Gnoblin operations for configuration and shell integrations.
