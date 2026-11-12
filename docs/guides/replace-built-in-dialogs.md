# Replace the built-in dialogs

Gnoblin draws its own consent dialog for portal requests. It is the default,
and it is replaceable: a program of your own, such as your shell, claims the
`portal.ui` slot and draws the dialog instead. The built-in dialog stays in
the compositor as the fallback, so a request is never left without a way to
ask the user.

## What the built-in dialog covers

The portal backend asks for consent before an app can capture the screen,
control input, read its secret or use the accessibility dialog. The compositor
draws the question on top of everything, grabs the pointer and keyboard while it
is open, and opens it with Deny focused.

## Who serves a request

For each consent request the compositor picks one of two servers:

| A shell holds `portal.ui`        | Who serves the request                       |
| -------------------------------- | -------------------------------------------- |
| No                               | The built-in dialog.                         |
| Yes, and the session is unlocked | The shell. The built-in dialog stays closed. |
| Yes, and the session is locked   | Neither. The request fails with `locked`.    |

The shell gets half of the time the requester will wait. The built-in dialog
gets what is left.

## Claim the slot

Only the session owner's connection can claim a slot. Any program that connects
to the IPC socket as the same user is that connection. Start your program with
an [autostart](/guides/autostart) entry, so Gnoblin launches it with the session:

```lua
gnoblin.configure {
    autostart = {
        dialogs = {command = {"/home/you/bin/my-dialogs"}},
    },
}
```

Replace the path with your own program. The program finds the socket through
`$GNOBLIN_IPC_SOCKET`, or at `$XDG_RUNTIME_DIR/gnoblin/ipc.sock` when that is
unset, and claims the slot with `gnoblin-client`:

```rust
use gnoblin_client::unix::{UnixTransport, socket_path};
use gnoblin_client::{Client, Event};
use gnoblin_model::Provider;
use gnoblin_types::Version;

let path = socket_path(|name| std::env::var(name).ok())?;
let mut client = Client::connect(UnixTransport::connect(&path)?, Version::new(0, 3, 0))?;
client.claim_slot("portal.ui", Provider::Shell)?;
loop {
    if let Event::RequestOffered { request, title, message, subject, .. } = client.next_event()? {
        client.claim(request)?;
        // Draw the dialog for `subject`, then answer once the user has chosen.
        client.answer(request, b"deny")?;
    }
}
```

Replace the version with the one your Gnoblin build serves. The Lua runtime
cannot claim a slot yet, so the claimant is a program like this one.

A second program that claims `portal.ui` while one holds it is refused. Only the
holder gives the slot back with `release_slot`, and a holder that disconnects
gives it back too. The built-in dialog serves again from then on.

## What your program receives

Each consent request arrives as a `request.offered` event for the `portal.ui`
slot:

| Field      | Content                                                                         |
| ---------- | ------------------------------------------------------------------------------- |
| `slot`     | `portal.ui`.                                                                    |
| `title`    | The portal interface that asked, such as `Screenshot` or `ScreenCast`.          |
| `message`  | The question to show, written by the portal backend.                            |
| `subject`  | The app id of the caller, as UTF-8. It is empty when the caller has no app id.  |
| `deadline` | When the compositor takes the request back and hands it to the built-in dialog. |

`subject` is the identity to show. The portal backend verifies it from the
caller's connection to the session bus, and the request body never supplies it.
Never show the app's own text as its identity.

## What your answer must contain

Claim the request, then answer with exactly one of these byte strings:

- `allow` lets the app have what it asked for.
- `deny` refuses it.

Anything else is not understood, and never counts as consent. That includes
`Allow`, `allow\n` and an empty answer. The built-in dialog serves the request
instead.

## What the compositor still enforces

A replacement dialog cannot waive these rules:

- **Deny is the default.** Only the exact answer `allow` grants. Every failure
  below ends in the built-in dialog, which opens with Deny focused. No failure
  turns into consent.
- **An early allow is a refusal.** An `allow` that arrives within 500 ms of the
  offer counts as `deny`. The built-in dialog ignores input for the same time,
  so a click already in progress cannot accept it.
- **The identity comes from the compositor.** The request names the caller by
  the app id the portal backend verified. Your program cannot change it.
- **A failing replacement falls back to the built-in dialog.** It serves the
  request when your program declines it, answers something it does not
  understand, does not answer before the request's deadline, or disconnects
  while it holds the request.
- **A request that is over ends the offer.** If the app gives up, the portal
  backend goes away or the session locks, the compositor tells your program the
  request ended. A late answer is refused. A lock refuses the request with
  `locked` and offers nothing.
- **Only the owner's connection takes part.** A connection that is not the
  session owner's cannot claim the slot, answer a request or give the slot back.
- **Grants are the compositor's record.** An allowed Secret request is recorded
  by the compositor, never by your program. A lock clears every secret grant.

## Other prompts

The `prompt.secret` slot works the same way for the keyring's passphrase
prompt, with one difference: it has no built-in dialog. Without a claimant the
request is refused at once. See [passphrase prompts](/shell-api/passphrase-prompts).
The `auth.polkit` slot is the polkit authentication prompt, described in
[authentication prompts](/shell-api/authentication).

## Related

- [Requests](/shell-api/requests)
- [Autostart](/guides/autostart)
- [Portal routing](/config/configure/portals)
