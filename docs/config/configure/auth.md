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
the session's authentication agent for a password. The compositor draws the
password dialog itself and checks the password with polkit's own helper. No
config or shell answers the request. See
[Authentication prompts](/shell-api/authentication).

| Field          | Type and accepted values | Default | Effect                                                                           |
| -------------- | ------------------------ | ------- | -------------------------------------------------------------------------------- |
| `polkit_agent` | Boolean                  | `false` | `true` registers Gnoblin as the polkit agent. `false` leaves the role to others. |

No other field is accepted. Gnoblin rejects an `auth` value that is not a
table, a `polkit_agent` value that is not a boolean, and any other key.

## What changes when it is on

Gnoblin registers as the polkit agent for the login session. Each request opens
the compositor's password dialog. A TypeScript shell may watch requests and
refuse one. See [Authentication prompts](/shell-api/authentication).

Polkit allows one agent per session. Stop any other agent, such as
`polkit-gnome`, `polkit-mate`, or a shell's own agent, or do not start it,
before you turn this on. If another agent owns the session, Gnoblin cannot
register, and the `auth-agent` capability reports `polkit_unavailable`. Check
the capability with `gnoblin.capabilities.list()`.

The `auth-agent` capability reports `auth_disabled` while `polkit_agent` is
`false`. In that state Gnoblin leaves the role free for an agent that you start
at login.

## Reload behavior

Changes apply when Gnoblin reloads the configuration. Setting
`polkit_agent = false` cancels every request that is waiting and unregisters so
another agent can take the session.
