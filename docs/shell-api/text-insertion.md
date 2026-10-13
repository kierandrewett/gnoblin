# Text Insertion

Insert text into the focused Wayland client using a trusted input context.

Text insertion is available for the focused Wayland client when the runtime receives a supported event context.

## Request a text target

Use `gnoblin.input.text_target(context)` from a shortcut event callback to
request a one-use text target. It returns an operation; on success, its value
is an opaque `TextTarget`:

```lua
gnoblin.events.on("gnoblin.shortcut.activated", function(event)
    if not event.focus_context then
        return
    end

    gnoblin.input.text_target(event.focus_context):on_complete(function(target, err)
        if err then
            return
        end
        target:insert_text("😀")
    end)
end)
```

The `gnoblinctl lua` console can use the same capability inside an event
callback. Save this as `insert-text.lua` and run it from a Gnoblin session:

```lua
gnoblin.events.once("gnoblin.shortcut.activated", function(event)
    if not event.focus_context then
        return
    end

    local target = gnoblin.input.text_target(event.focus_context)
    target:insert_text("hello from Gnoblin")
end)
```

```sh
gnoblinctl lua insert-text.lua
```

The console returns completed results synchronously. It keeps the target token
private and sends both requests over the connection that delivered the event.

The compositor consumes the `FocusContext` when it handles the request. It
creates a target only while the same Wayland surface and client keep keyboard
focus and an active text-input-v3 session.

The target expires with its five-second `FocusContext` deadline. Session lock,
runtime reload, focus loss, or runtime shutdown revokes it. X11 clients are
not supported.

`TextTarget.window_id` is present when the focused surface belongs to a managed
window. It contains that window's stable ID. `TextTarget.caret` is an optional
rectangle in compositor logical coordinates.

These fields help a shell place its UI. Only the opaque target authorizes
insertion.

## Insert text

Call `target:insert_text(text)` once. It returns an operation and consumes the
target on the first attempt. Text must contain 1 to 256 bytes of valid UTF-8
without NUL or control characters.

Insertion succeeds only while the surface, client, focus epoch, and active text
input remain current. The session must be unlocked. Mutter commits the text
through its focused input-method path.

Modifiers held when the shortcut activated may remain held. Adding another
modifier invalidates the target. Ctrl, Alt, Shift, Lock, Meta, or Hyper state
prevents target creation. Gnoblin allows at most 128 active targets per
compositor; creation fails when that limit is full.
