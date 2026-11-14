# Authentication agent

The `auth` key lets Gnoblin act as the session's polkit authentication agent:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    auth: { polkitAgent: true },
});
```

Programs such as `pkexec` ask polkit for administrator rights. Polkit then asks the session's authentication agent
for a password. The compositor draws the password dialog itself and checks the password with polkit's own helper.
The shell never sees the password. A script may watch requests and refuse one: see
[authentication requests](/js-idle-auth-launches).

| Field         | Type and accepted values | Default | Effect                                                                           |
| ------------- | ------------------------ | ------- | -------------------------------------------------------------------------------- |
| `polkitAgent` | Boolean                  | `false` | `true` registers Gnoblin as the polkit agent. `false` leaves the role to others. |

No other field is accepted. Gnoblin rejects an `auth` value that is not a table, a `polkitAgent` value that is not a
boolean, and any other key.

## What changes when it is on

Gnoblin registers as the polkit agent for the login session, and each request opens the compositor's password dialog.

Polkit allows one agent per session. Stop any other agent, such as `polkit-gnome` or `polkit-mate`, or do not start
it, before you turn this on. If another agent owns the session, Gnoblin cannot register.

While `polkitAgent` is `false`, Gnoblin leaves the role free for an agent that you start at login.

## Reload behaviour

Changes apply when Gnoblin reloads the configuration. Setting `polkitAgent: false` cancels every request that is
waiting and unregisters, so another agent can take the session.
