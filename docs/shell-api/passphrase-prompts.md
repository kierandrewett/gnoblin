# Answer keyring passphrase prompts

Gnoblin's built-in keyring is locked until it receives its login password. It asks
for that password through the `prompt.secret` capability slot. A shell that claims
the slot shows the prompt and returns the answer.

There is no config switch for this. The keyring asks whenever it needs its
password. Turn the keyring off with
[`gnoblin.configure.keyring`](/config/configure/keyring) if you use another keyring.

## Fails closed without a claimant

Nothing claims `prompt.secret` by default. If no shell has claimed it, the
request is refused at once and names `prompt.secret`. Nothing waits out a
deadline, and the keyring stays locked.

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

## Related

- [Keyring configuration](/config/configure/keyring)
- [Requests](/shell-api/requests)
