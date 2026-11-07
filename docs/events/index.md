# Lua Events

> **Rust status:** The pages below describe a broader legacy event contract than Rust implements.
> Rust delivers only these object-model signals:
>
> - `gnoblin.effect.closed`
> - `gnoblin.output.closed`
> - `gnoblin.request.offered`
> - `gnoblin.request.finished`
> - `gnoblin.session.ready`
> - `gnoblin.session.bell`
> - `gnoblin.session.reload_requested`
> - `gnoblin.window.closed` (schema payload, not the legacy record)
> - `gnoblin.window.menu_requested`
> - `gnoblin.workspace.layout_changed`
> - `gnoblin.workspace.layout_failed`

This event reference describes callback registration and legacy event payloads.
Names and payloads outside the list above are not delivered in Rust.

Use the [shell runtime API overview](/shell-api/) to find methods by task.
This page describes callback registration and event payloads.

Register repeating callbacks with `gnoblin.on`; use
`gnoblin.once` for one-time callbacks. Each returns a `Subscription`.

Remove a repeating callback with `subscription:unsubscribe()`. A `once`
subscription removes itself before invocation.

The callback receives `event.name`, the live schema object in `event.object`, and any fields declared by that signal.

```lua
gnoblin.on("gnoblin.session.ready", function(event)
    print(event.name, event.object.version)
end)
```

## Event domains

- [Mutter](mutter)
- [Gnoblin lifecycle](lifecycle)
- [Input and shortcuts](input-shortcuts)
- [Session events](session)
- [Launch events](launch-portal)
- [Window, workspace, and monitor records](desktop-state)
- [Listen to all forwarded events](listen-all)

See the [Lua runtime API](/shell-api/) for subscriptions and event-bound operations.
