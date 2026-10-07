# Answer keyring and GPG prompts

`gnome-keyring` and `pinentry-gnome3` ask a prompter service on the session bus
for passphrases and confirmations. In a GNOME session `gcr-prompter` shows
those prompts. Gnoblin can serve them instead and publish each one as an event.
Your config or shell shows the prompt and returns the answer.

This is off by default. Turn it on with
[`gnoblin.configure.prompts`](/config/configure/prompts):

```lua
gnoblin.configure {
    prompts = {enabled = true},
}
```

Write a handler first. A prompt that nothing answers is cancelled after 120
seconds.

## Answer a prompt from Lua

This config uses `zenity` to ask for a password or a yes or no answer. For a
password prompt, `zenity --password` prints the password to standard output.
For a confirmation, `zenity --question` reports the answer in its exit status.

```lua
local function text(value, fallback)
    if value == nil or value == "" then
        return fallback
    end
    return value
end

gnoblin.on("gnoblin.prompt.requested", function(event)
    local title = text(event.title, "Passphrase")
    local message = text(event.message, title)

    if event.kind == "password" then
        gnoblin.commands.capture {
            argv = {"zenity", "--password", "--title", message},
        }:on_complete(function(result, err)
            if err or result.exit_code ~= 0 then
                gnoblin.prompt.cancel {request_id = event.request_id}
                return
            end
            local password = result.stdout:gsub("\n$", "")
            gnoblin.prompt.respond {request_id = event.request_id, password = password}
        end)
        return
    end

    gnoblin.commands.capture {
        argv = {"zenity", "--question", "--title", title, "--text", message},
    }:on_complete(function(result, err)
        if err or result.exit_code < 0 then
            gnoblin.prompt.cancel {request_id = event.request_id}
            return
        end
        gnoblin.prompt.respond {
            request_id = event.request_id,
            confirmed = result.exit_code == 0,
        }
    end)
end)
```

Run `echo test | gpg --symmetric --output /dev/null` in a terminal with
`pinentry-gnome3` as the pinentry program. The prompt appears, and `gpg`
continues after you enter a passphrase.

## Events

Both events carry `request_id`, an integer that stays the same for one prompt.
Gnoblin does not reuse a `request_id` while it runs.

| Event                      | Extra fields                                                                                                 | When it fires                                  |
| -------------------------- | ------------------------------------------------------------------------------------------------------------ | ---------------------------------------------- |
| `gnoblin.prompt.requested` | `kind`, `title`, `message`, `description`, `warning`, `password_new`, `choice_label`, `caller_window`, `continue_label`, `cancel_label` | A program asks for a password or a confirmation. |
| `gnoblin.prompt.finished`  | `result`                                                                                                     | The prompt ended.                              |

- `kind` is `"password"` when the program needs a password, or `"confirm"` when
  it needs a yes or no answer.
- `title`, `message`, `description`, and `warning` are text for the user. Any
  of them can be an empty string.
- `password_new` is `true` when the user chooses a new password. Ask for it
  twice if your prompt program supports it.
- `choice_label` is the text of an optional checkbox, for example "Automatically
  unlock this keyring". It is an empty string when there is no checkbox.
- `caller_window` identifies the window of the program that asked, in the form
  the program gave it. It is an empty string when the program gave none.
- `continue_label` and `cancel_label` are the preferred button texts. They are
  empty strings when the program gave none.
- `result` is `"answered"` after `prompt.respond`, `"cancelled"` after
  `prompt.cancel` or when the program withdraws the prompt, and `"timeout"`
  when nothing answered in 120 seconds.

## Methods

Each method takes a table and returns an operation. The operation succeeds with
`{request_id = ..., submitted = true}`.

| Method                  | Fields                                              | Effect                                          |
| ----------------------- | --------------------------------------------------- | ----------------------------------------------- |
| `gnoblin.prompt.respond`| `request_id`, `password`, `confirmed`, `choice`     | Answer the prompt.                              |
| `gnoblin.prompt.cancel` | `request_id`                                        | End the prompt. The program that asked sees a cancellation. |

For `prompt.respond`:

| Field        | Type    | Required for          | Meaning                                                                    |
| ------------ | ------- | --------------------- | -------------------------------------------------------------------------- |
| `request_id` | integer | Always                | The `request_id` from the event. Must be positive.                         |
| `password`   | string  | `kind = "password"`   | The password. UTF-8, up to 4096 bytes. Not allowed for a confirmation.     |
| `confirmed`  | boolean | `kind = "confirm"`    | `true` continues. `false` cancels. Not allowed for a password prompt.      |
| `choice`     | boolean | Never                 | The state of the checkbox that `choice_label` describes.                   |

A missing or wrong-typed field fails the operation with a message that names
the field.

## Limits

- Each prompt waits up to 120 seconds for an answer.
- At most 8 prompts wait at the same time. Extra prompts are cancelled.
- Gnoblin cancels every waiting prompt when `prompts.enabled` becomes `false`
  or when the session ends. A prompt that arrives while the Lua runtime is not
  ready is cancelled at once.
- `gcr-prompter` must not own the bus names. If it does, the `prompt-broker`
  capability is unavailable with the reason `name_unavailable`. The reason is
  `prompts_disabled` while the setting is off. Check it with
  `gnoblinctl capabilities`.

## Handle the password with care

Gnoblin keeps the password in secure memory while it holds it and wipes that
copy when the prompt ends. The password also passes through the Lua runtime,
and Lua strings are not wiped.

- Do not log `password` or `result.stdout`.
- Do not put a password in `argv`. Other users can read command lines.
- A prompt program that prints the password to standard output keeps it off the
  command line.
- A keyring password unlocks stored secrets. Show the prompt only for requests
  that come from your own session.

## Use the methods from a shell process

A shell that connects to the [compositor bridge](/compositor-bridge) uses the
same methods as `prompt.respond` and `prompt.cancel` with API version 1.79 or
later, and subscribes to the `gnoblin.prompt.*` events. Only a client that
received `gnoblin.prompt.requested` can answer that prompt.
