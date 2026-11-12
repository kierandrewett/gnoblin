# Answer keyring passphrase prompts

> **Status:** not available yet: `gnoblin.configure.keyring`. The sections that use it describe the target, not current behaviour.

Gnoblin's built-in keyring is locked until it receives its login password. It asks
for that password through the `prompt.secret` capability slot. A shell that claims
the slot shows the prompt and returns the answer.

There is no config switch for this. The keyring asks whenever it needs its
password. Turn the keyring off with
[`gnoblin.configure.keyring`](/config/configure/keyring) if you use another keyring.

## Fails closed without a claimant

The stock shell claims `prompt.secret` and draws the prompt (`prompts.lua`). If no
shell has claimed it, the request is refused at once and names `prompt.secret`.
Nothing waits out a deadline, and the keyring stays locked.

Claim the slot before you expect unlock prompts to appear.

## What a claimant receives

The keyring sends a request for `prompt.secret`. Its kind is a passphrase or a
confirmation. The claimant answers with the typed passphrase or with the yes or
no answer. It can also refuse.

A request expires after 120 seconds. An answer that arrives after that is
discarded, and the keyring stays locked.

## Handle the passphrase with care

- Do not log the typed passphrase, and do not write it to a file.
- Do not pass the passphrase on a command line. Other users can read command
  lines.
- Show the prompt only for the keyring's own unlock request.

## Prompts this slot does not carry

`prompt.secret` carries the keyring's own unlock request and nothing else.

A passphrase that GnuPG asks for through `pinentry-gnome3` takes another route:

- `pinentry-gnome3` hands the request to `gcr-prompter` over D-Bus.
- `gcr-prompter` opens its own window. It is an ordinary toplevel that Gnoblin maps and focuses like any other.
- The shell draws nothing for it and claims no slot.
- `gcr-prompter` has to be running on the session bus. Without it, `pinentry-gnome3` falls back to a terminal prompt.

`crates/gnoblin-compositor/tests/pinentry.rs` runs both programs on a private bus against a live session. It checks
that the window maps, takes the keyboard and returns what was typed to `gpg-agent`.

## Related

- [Keyring configuration](/config/configure/keyring)
- [Requests](/shell-api/requests)
