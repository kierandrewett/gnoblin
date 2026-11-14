# Authentication prompts

When a program needs administrator rights, polkit asks an authentication agent for a password. Examples are `pkexec` and
system settings changes. The compositor draws the password dialog itself and checks the password with polkit's own helper,
`polkit-agent-helper-1`. A script never draws the dialog, reads the password or says that a password was right.
`gnoblin.authentication` lets your config watch the requests and refuse one.

```ts
import { AuthenticationListener, defineConfig } from "gnoblin";
import type { AuthenticationRequest } from "gnoblin";

class NoRoot extends AuthenticationListener {
    onAuthenticationRequest(request: AuthenticationRequest) {
        if (request.identities.includes("unix-user:root") && request.subject !== "pkexec") {
            request.decline("only pkexec may ask for root");
        }
    }
}

defineConfig({ setup: (gnoblin) => gnoblin.authentication.addListener(new NoRoot()) });
```

Put the file in `~/.config/gnoblin/init.ts`. Run `pkexec true` to see the dialog open. A request for root from any other
program is refused and its dialog never opens.

## How a request is handled

1. polkit asks Gnoblin's services role, which is the session's agent.
2. The services role offers the request to the compositor.
3. The compositor shows each request to the listeners for a short window, and a listener may refuse it.
4. If none refuses, the compositor shows its own password dialog above every window. The dialog names the action, the
   program asking and the account to authenticate as.
5. The password goes to polkit's setuid helper, which runs the PAM stack and tells polkit when the password is right.
6. The compositor answers the request only when the helper reports success.

PAM prompts and messages, such as a fingerprint reader waiting or a second factor, appear in the dialog. A wrong password
shows the helper's message and asks again, up to five times.

## Watch and refuse a request

`gnoblin.authentication` has these members:

| Member                     | Meaning                               |
| -------------------------- | ------------------------------------- |
| `addListener(listener)`    | Attaches an `AuthenticationListener`. |
| `removeListener(listener)` | Detaches it.                          |
| `listeners`                | The attached listeners.               |

An `AuthenticationListener` can define `onAuthenticationRequest(request)`. The request carries:

| Field        | Meaning                                                                                 |
| ------------ | --------------------------------------------------------------------------------------- |
| `actionId`   | The polkit action, such as `org.freedesktop.policykit.exec`.                            |
| `message`    | The sentence polkit wants shown, in the session's language.                             |
| `subject`    | The program asking, such as `pkexec`. It is text from polkit, not an identity to trust. |
| `appId`      | Always empty, because polkit names a process and not an application.                    |
| `identities` | Who may authenticate, written `unix-user:alice`.                                        |
| `deadline`   | When the compositor takes the request back, in milliseconds on its own clock.           |
| `kind`       | Always `auth`.                                                                          |

There is no `allow` and no way to read or give a password. `request.decline(reason)` is the only call: polkit is told the
request is not authorized, and the dialog never opens. The reason is for the log. A listener that does nothing, throws or
runs past its budget leaves the request to the dialog.

The cookie polkit gives the request never reaches a script. A shell cannot answer the request, so a reload or a crash of
the shell leaves it in the compositor's dialog, where the user or the requester can still end it.

## Cancelling and ending a request

A request ends without an authentication when any of these happens:

- The user presses Cancel or Escape.
- polkit cancels it, for example because the program that asked exited.
- The request passes its deadline.
- The session locks.

The compositor stops the helper in each case.

## The helper

Gnoblin looks for `polkit-agent-helper-1` in the places distributions install it. Set `GNOBLIN_POLKIT_HELPER` to the
helper's path to use another one. A missing helper is shown in the dialog, and the request fails closed.

Polkit allows one agent per session. If another polkit agent is running, it remains the agent and Gnoblin's dialog does not
open.
