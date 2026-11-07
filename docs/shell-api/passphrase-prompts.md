# Answer keyring and GPG prompts

> **Status: not implemented yet.** The Lua handler below has no runtime behind it.
> `gnoblin.prompt.requested`, `request:answer` and `request:cancel` are not delivered
> to Lua, because `crates/gnoblin-lua` drops request events. No shell claims the
> `prompt.secret` slot, so with `prompts.enabled = true` a prompt is refused at once
> with "no provider" and is not drawn. See `design/e2e-prompts.md`.

`gnome-keyring` and `pinentry-gnome3` ask a prompter service on the session bus
for passphrases and confirmations. Gnoblin can serve them and publish each one as an event.
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
When the prompt window opens, `window:present()` gives it keyboard focus, so you
can type the password without clicking it first.

```lua
local prompts = {}

local function text(value, fallback)
    if value == nil or value == "" then
        return fallback
    end
    return value
end

-- Focus the window of a running prompt program.
gnoblin.on("gnoblin.window.created", function(event)
    if prompts[event.window.process] then
        event.window:present()
    end
end)

local function ask(argv, options, answer)
    local prompt = gnoblin.process.spawn(argv, options)
    prompts[prompt.id] = true
    prompt:wait():on_complete(function(exit, err)
        prompts[prompt.id] = nil
        answer(exit, err)
    end)
end

gnoblin.on("gnoblin.prompt.requested", function(event)
    local request = event.request
    local title = text(request.title, "Passphrase")
    local message = text(request.message, title)

    if request.kind == "password" then
        ask({"zenity", "--password", "--title", message}, {stdout = "capture"}, function(exit, err)
            if err or exit.exit_code ~= 0 then
                request:cancel()
                return
            end
            request:answer((exit.stdout:gsub("\n$", "")))
        end)
        return
    end

    ask({"zenity", "--question", "--title", title, "--text", message}, nil, function(exit, err)
        if err or exit.signal then
            request:cancel()
            return
        end
        request:answer(exit.exit_code == 0)
    end)
end)
```

Run `echo test | gpg --symmetric --output /dev/null` in a terminal with
`pinentry-gnome3` as the pinentry program. The prompt appears, and `gpg`
continues after you enter a passphrase.

## The request

`gnoblin.prompt.requested` carries `event.request`, a
[`Request`](/shell-api/requests). Its `kind` is `"password"` when the program
needs a password, or `"confirm"` when it needs a yes or no answer. Its fields:

- `id`: the same for the prompt's events. Gnoblin does not reuse it while it
  runs.
- `title`, `message`, `description` and `warning`: text for the user. Any of
  them can be an empty string.
- `password_new`: `true` when the user chooses a new password. Ask for it twice
  if your prompt program supports it.
- `choice_label`: the text of an optional checkbox, for example "Automatically
  unlock this keyring". It is an empty string when there is no checkbox.
- `caller_window`: the window of the program that asked, in the form the
  program gave it, or an empty string.
- `continue_label` and `cancel_label`: the preferred button texts, or empty
  strings.

| Method                       | Kind       | Effect                                                      |
| ---------------------------- | ---------- | ----------------------------------------------------------- |
| `answer(password, choice?)`  | `password` | Answer with the password, UTF-8 up to 4096 bytes.           |
| `answer(confirmed, choice?)` | `confirm`  | `true` continues; `false` cancels.                          |
| `cancel()`                   | both       | End the prompt. The program that asked sees a cancellation. |
| `finished()`                 | both       | An Operation that succeeds with the outcome.                |

`choice` is the state of the checkbox that `choice_label` describes.

The outcome is one of these values:

- `"answered"`: the request got an answer.
- `"cancelled"`: the prompt was cancelled, or the program withdrew it.
- `"timeout"`: nothing answered in 120 seconds.

`gnoblin.prompt.finished` reports the same outcome as `result`.

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

- Do not log the password or the prompt program's `stdout`.
- Do not put a password in `argv`. Other users can read command lines.
- A prompt program that prints the password to standard output keeps it off the
  command line.
- A keyring password unlocks stored secrets. Show the prompt only for requests
  that come from your own session.

## Answer from a shell process

A shell that connects to the [compositor bridge](/bridge/) needs API version
1.79 or later. It subscribes to the `gnoblin.prompt.*` events and answers with
these methods, each with `request_id`:

- `prompt.respond`, with `password` or `confirmed` and optional `choice`.
- `prompt.cancel`.

Only a client that received `gnoblin.prompt.requested` can answer that prompt.
