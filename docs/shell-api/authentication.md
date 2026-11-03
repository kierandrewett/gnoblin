# Authentication prompts

When a program needs administrator rights, polkit asks the session's
authentication agent for a password. Examples include `pkexec` and system
settings changes.

Gnoblin can take that role when you set
`gnoblin.configure { auth = { polkit_agent = true } }`. It registers as the
agent and publishes each request as an event. Your config or shell shows the
prompt and returns the answer.

Polkit allows one agent per session. If `polkit-gnome`, `polkit-mate`, or a
shell's own agent is running, stop it or do not start it before you turn the
setting on. See [`gnoblin.configure.auth`](/config/configure/auth).

Without a listener, a request fails after 15 seconds and the program that asked
reports "Not authorized".

## Answer a prompt from Lua

This config asks for the password with `zenity` and sends it back. Any program
that prints the password to standard output and exits with status `0` works the
same way.

```lua
gnoblin.configure {auth = {polkit_agent = true}}

gnoblin.on("gnoblin.auth.requested", function(event)
    local request = event.request
    request:claim()
    request:on("prompt", function()
        local prompt = gnoblin.process.spawn({"zenity", "--password", "--title", request.prompt},
            {stdout = "capture"})
        prompt:wait():on_complete(function(exit, err)
            if err or exit.exit_code ~= 0 then
                request:cancel()
                return
            end
            request:answer((exit.stdout:gsub("\n$", "")))
        end)
    end)
end)
```

Run `pkexec id -u` in a terminal. The prompt appears, and the command prints `0`
after a correct password.

## The request

`gnoblin.auth.requested` and its follow-up events carry `event.request`, a
[`Request`](/shell-api/requests) whose `kind` is `"authentication"`. Its fields:

- `id`: the same for every event of one request.
- `message`: the text polkit wants shown, for example "Authentication is
  needed to run `/usr/bin/id -u' as the super user".
- `action_id` and `icon_name`: the polkit action and its icon.
- `details`: a table of strings that describe the action.
- `identities`: who can authenticate. Each entry has `kind` (`user`, `group`,
  or `other`) and `name`.
- `prompt` and `echo`: the current question. `echo` is `false` for a password;
  hide what the user types.
- `text` and `error`: the last message line, and whether it is an error.

| Method              | Effect                                                                |
| ------------------- | --------------------------------------------------------------------- |
| `claim(identity?)`  | Start the check for one identity, counted from 1. The default is `1`. |
| `on("prompt", fn)`  | Call `fn(request)` when the check needs input.                        |
| `on("message", fn)` | Call `fn(request)` for each line of text for the user.                |
| `answer(response)`  | Answer the current prompt. Up to 4096 bytes of UTF-8.                 |
| `cancel()`          | End the request. The program that asked reports a refusal.            |
| `finished()`        | An Operation that succeeds with the outcome.                          |

The outcome is `authorized`, `denied` (wrong password) or `cancelled`. A wrong
password ends the request, and Gnoblin does not ask again. The program can
start a new request.

The events are `gnoblin.auth.requested`, `gnoblin.auth.prompt`,
`gnoblin.auth.message` and `gnoblin.auth.finished`.

## Limits

- A request that nothing claims or cancels is cancelled after 15 seconds.
- After `claim()`, each prompt waits up to 120 seconds for an answer.
- At most 8 requests wait at the same time. Extra requests are cancelled.
- Only one authentication agent can own a session. Check the `auth-agent`
  capability with `gnoblinctl capabilities`. It reports `auth_disabled` while
  `polkit_agent` is not `true`. It reports `polkit_unavailable` when the setting
  is on but another agent, such as `polkit-gnome`, already owns the session.

## Handle the password with care

The password passes through Gnoblin's memory and the Lua runtime's memory.
Gnoblin wipes its own copies where it can, but Lua strings are not wiped.

- Do not log the answer or the prompt program's `stdout`.
- Do not put a password in `argv`. Other users can read command lines.
- A prompt program that prints the password to standard output keeps it off the
  command line.

## Answer from a shell process

A shell that connects to the [compositor bridge](/bridge/) needs API version
1.78 or later. It subscribes to the `gnoblin.auth.*` events and answers with
these methods, each with `request_id`:

- `auth.begin`, with optional `identity`, claims the request.
- `auth.respond`, with `response`, answers the current prompt.
- `auth.cancel` ends the request.

Only a client that received `gnoblin.auth.requested` can answer that request.
