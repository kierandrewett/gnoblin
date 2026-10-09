# gnoblin.configure.auth

Let Gnoblin act as the session's polkit authentication agent:

```lua
gnoblin.configure {
    auth = {
        polkit_agent = true,
    },
}
```

Programs such as `pkexec` ask polkit for administrator rights. Polkit then asks
the session's authentication agent for a password. In a GNOME session GNOME
Shell is that agent. When `auth.polkit_agent` is `true`, Gnoblin takes that role
and publishes each request as an event. Your config or shell shows the prompt
and returns the answer. See [Answer authentication prompts](/shell-api/authentication).

| Field          | Type and accepted values | Default | Effect                                                                 |
| -------------- | ------------------------ | ------- | ---------------------------------------------------------------------- |
| `polkit_agent` | Boolean                  | `false` | `true` registers Gnoblin as the polkit agent. `false` leaves the role to others. |

No other field is accepted. Gnoblin rejects an `auth` value that is not a
table, a `polkit_agent` value that is not a boolean, and any other key.

## What changes when it is on

Gnoblin registers as the polkit agent for the login session and sends
`gnoblin.auth.requested`, `gnoblin.auth.prompt`, `gnoblin.auth.message`, and
`gnoblin.auth.finished` events.

Write a handler before you turn this on. A request that no handler claims with
`gnoblin.auth.begin` or `gnoblin.auth.cancel` is cancelled after 15 seconds,
and the program that asked reports "Not authorized".

Polkit allows one agent per session. Stop any other agent, such as
`polkit-gnome`, `polkit-mate`, or a shell's own agent, or do not start it,
before you turn this on. If another agent owns the session, Gnoblin cannot
register, and the `auth-agent` capability reports `polkit_unavailable`. Check
the capability with `gnoblinctl capabilities`.

The `auth-agent` capability reports `auth_disabled` while `polkit_agent` is
`false`. In that state Gnoblin leaves the role free for an agent that you start
at login.

## Reload behavior

Changes apply when Gnoblin reloads the configuration. Setting
`polkit_agent = true` registers the agent. Setting `polkit_agent = false`
cancels every request that is waiting, publishes `gnoblin.auth.finished` for
each, and unregisters so another agent can take the session.
