# Authentication prompts

When a program needs administrator rights, for example `pkexec` or a system
settings change, polkit asks the session's authentication agent for a password.
GNOME Shell is that agent in a GNOME session. Gnoblin has no shell, so it can
take that role. It does so only when you set
`gnoblin.configure { auth = { polkit_agent = true } }`. With the setting on,
Gnoblin registers as the agent and publishes each request as an event. Your
config or shell shows the prompt and returns the answer.

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
    gnoblin.auth.begin {request_id = event.request_id}
end)

gnoblin.on("gnoblin.auth.prompt", function(event)
    gnoblin.commands.capture {
        argv = {"zenity", "--password", "--title", event.prompt},
    }:on_complete(function(result, err)
        if err or result.exit_code ~= 0 then
            gnoblin.auth.cancel {request_id = event.request_id}
            return
        end
        local password = result.stdout:gsub("\n$", "")
        gnoblin.auth.respond {request_id = event.request_id, response = password}
    end)
end)
```

Run `pkexec id -u` in a terminal. The prompt appears, and the command prints `0`
after a correct password.

## Events

All events carry `request_id`, an integer that stays the same for one request.

| Event                     | Extra fields                                                  | When it fires                              |
| ------------------------- | ------------------------------------------------------------- | ------------------------------------------ |
| `gnoblin.auth.requested`  | `action_id`, `message`, `icon_name`, `details`, `identities`  | A program asks for authentication.         |
| `gnoblin.auth.prompt`     | `prompt`, `echo`                                              | The check needs input from the user.       |
| `gnoblin.auth.message`    | `text`, `error`                                               | The check has a line of text for the user. |
| `gnoblin.auth.finished`   | `result`                                                      | The request ended.                         |

- `message` is the text polkit wants shown, for example "Authentication is
  needed to run `/usr/bin/id -u' as the super user".
- `details` is a table of strings that describe the action.
- `identities` lists who can authenticate. Each entry has `kind` (`user`,
  `group`, or `other`) and `name`.
- `echo` is `false` for a password. Hide what the user types.
- `error` is `true` when `text` is an error line from the check.
- `result` is `authorized`, `denied` (wrong password), or `cancelled`.

## Methods

Each method takes a table and returns an operation. The operation succeeds with
`{request_id = ..., submitted = true}`.

| Method                | Fields                                  | Effect                                                         |
| --------------------- | --------------------------------------- | -------------------------------------------------------------- |
| `gnoblin.auth.begin`  | `request_id`, `identity` (optional)     | Start the check for one identity. `identity` counts from 1.    |
| `gnoblin.auth.respond`| `request_id`, `response`                | Answer the latest prompt. Up to 4096 bytes of UTF-8.           |
| `gnoblin.auth.cancel` | `request_id`                            | End the request. The program that asked reports a refusal.     |

`identity` defaults to `1`. Pick another entry from `identities` when the user
must authenticate as a different account.

A wrong password ends the request with `gnoblin.auth.finished` and
`result = "denied"`. The program that asked reports a refusal, and Gnoblin does
not ask again. The program can start a new request.

## Limits

- A request that nothing claims with `begin` or `cancel` is cancelled after 15
  seconds.
- After `begin`, each prompt waits up to 120 seconds for `respond`.
- At most 8 requests wait at the same time. Extra requests are cancelled.
- Only one authentication agent can own a session. Check the `auth-agent`
  capability with `gnoblinctl capabilities`. It reports `auth_disabled` while
  `polkit_agent` is not `true`. It reports `polkit_unavailable` when the setting
  is on but another agent, such as `polkit-gnome`, already owns the session.

## Handle the password with care

The password passes through Gnoblin's memory and the Lua runtime's memory.
Gnoblin wipes its own copies where it can, but Lua strings are not wiped.

- Do not log `response` or `result.stdout`.
- Do not put a password in `argv`. Other users can read command lines.
- A prompt program that prints the password to standard output keeps it off the
  command line.

## Use the methods from a shell process

A shell that connects to the [compositor bridge](/compositor-bridge) uses the
same methods as `auth.begin`, `auth.respond`, and `auth.cancel` with API version
1.78 or later, and subscribes to the `gnoblin.auth.*` events. Only a client that
received `gnoblin.auth.requested` can answer that request.
