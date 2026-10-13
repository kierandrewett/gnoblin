# Compositor Bridge

Connect a shell or session tool to Gnoblin’s compositor through the local Unix
socket. The bridge provides versioned snapshots, events, and operations.

## Start here

- [Connect and frame requests](/bridge/connection): socket path, greeting,
  request format, API versions, limits, and reconnect behavior.
- [Sessions and Windows](/bridge/session-and-windows): snapshots, session
  state, locking, thumbnails, layers, and shell UI sessions.
- [Input and focus](/bridge/input-and-focus): input sources, trusted contexts,
  text insertion, snapping, and focus policy.
- [Shortcuts and events](/bridge/shortcuts-and-events): event subscriptions,
  shortcut bindings, and operation completion.
- [Permissions and privacy](/bridge/permissions-and-privacy): portal grants,
  permission policy, privacy state, and monitor privacy screens.
- [Runtime and appearance](/bridge/runtime-and-appearance): runtime health,
  orientation lock, and desktop appearance.
- [Examples](/bridge/examples): complete shell, Python, Node.js, and Go clients.

## Choose a transport operation

Use `windows` or `monitors` for a snapshot stream. Use `events` to subscribe to
named events. Use `api` for one versioned method call. The [connection
reference](/bridge/connection) documents each request and the server greeting.

Before calling an optional method, check `hello.methods`, `hello.events`, and
`hello.capabilities`. The [Lua runtime reference](/shell-api/) documents the
same Gnoblin operations for configuration and shell integrations.
