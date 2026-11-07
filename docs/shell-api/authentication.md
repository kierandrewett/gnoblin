# Authentication prompts

When a program needs administrator rights, polkit asks an authentication agent
for a password. Examples include `pkexec` and system settings changes.

An authentication prompt is a [request](requests.md). The compositor offers it
to the provider that holds the authentication slot, that provider claims it,
shows the prompt and answers or declines. The password is a secret answer, so
the compositor redacts it from every frame, dump and log.

## Status

Not usable. The request lifecycle is implemented in the compositor, but:

- The Lua runtime does not receive offers or settle them.
- There is no polkit agent registration.

With no provider holding the authentication slot, a request is refused at once.
A provider that claims it but never answers ends it as timed out.

Polkit allows one agent per session. If `polkit-gnome`, `polkit-mate`, or a
shell's own agent is running, it remains the agent.
