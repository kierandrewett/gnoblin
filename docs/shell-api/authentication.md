# Authentication prompts

When a program needs administrator rights, polkit asks an authentication agent
for a password. Examples include `pkexec` and system settings changes.

The compositor draws the prompt itself and checks the password itself. No
script, and no shell, answers an authentication request or reads the password.

## How a request is handled

1. polkit asks Gnoblin's services role, which is the session's agent.
2. The services role offers the request to the compositor on the
   `auth.polkit` slot.
3. The compositor shows its own password dialog above every window. The dialog
   names the action, the program asking and the account to authenticate as.
4. The password goes to polkit's setuid helper, `polkit-agent-helper-1`,
   which runs the PAM stack. The helper tells polkit when the password is right.
5. The compositor answers the request only when the helper reports success.

PAM prompts and messages, such as a fingerprint reader waiting or a second
factor, appear in the dialog. A wrong password shows the helper's message and
asks again, up to five times.

## What a shell can do

A shell cannot claim the `auth.polkit` slot, and it cannot offer or answer a
request. It can watch requests through the dialog observation slot and refuse
one by policy.

A refusal only makes things stricter: polkit is told the request is not
authorized and the dialog never opens. See
[observe the dialogs](/guides/replace-built-in-dialogs) and, for the TypeScript
API, [Idle, authentication and launch feedback](/js-idle-auth-launches).

## Cancelling and ending a request

A request ends without an authentication when any of these happens:

- The user presses Cancel or Escape.
- polkit cancels it, for example because the program that asked exited.
- The request passes its deadline.
- The session locks.

The compositor stops the helper in each case.

## The helper

Gnoblin looks for `polkit-agent-helper-1` in the places distributions install
it. Set `GNOBLIN_POLKIT_HELPER` to the helper's path to use another one.

A missing helper is shown in the dialog, and the request fails closed.

## Status

The compositor's half is tested against a fake helper and a fake polkitd on a
private bus. A real `polkit-agent-helper-1` and a real polkitd have not been
run against it. Check it on your machine with `pkexec true`.

Polkit allows one agent per session. If `polkit-gnome`, `polkit-mate`, or a
shell's own agent is running, it remains the agent.

The Lua runtime has no authentication API, so a Lua config cannot watch or
refuse a request. Use the TypeScript shell for that.
