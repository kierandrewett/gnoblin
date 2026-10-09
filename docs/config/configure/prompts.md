# gnoblin.configure.prompts

Let Gnoblin answer keyring and GPG passphrase prompts:

```lua
gnoblin.configure {
    prompts = {
        enabled = true,
    },
}
```

Programs such as `gnome-keyring` and `pinentry-gnome3` ask a prompter service
on the session bus for a passphrase. In a GNOME session `gcr-prompter` is that
service. When `prompts.enabled` is `true`, Gnoblin takes its place and
publishes each prompt as an event. Your config or shell shows the prompt and
returns the answer. See [Answer keyring and GPG prompts](/shell-api/passphrase-prompts).

| Field     | Type and accepted values | Default | Effect                                                              |
| --------- | ------------------------ | ------- | ------------------------------------------------------------------- |
| `enabled` | Boolean                  | `false` | `true` serves keyring and GPG prompts. `false` leaves them to others. |

No other field is accepted. Gnoblin rejects a `prompts` value that is not a
table, an `enabled` value that is not a boolean, and any other key.

## What changes when it is on

Gnoblin owns `org.gnome.keyring.SystemPrompter` and
`org.gnome.keyring.PrivatePrompter` on the session bus. It owns neither name
while `enabled` is `false`.

Write a handler before you turn this on. A prompt that no handler answers is
cancelled after 120 seconds, and the program that asked sees a cancellation.

GNOME's `gcr-prompter` must not be running. If it already owns either name,
Gnoblin cannot take it, and the `prompt-broker` capability reports
`name_unavailable`. Stop `gcr-prompter`, then reload the config. Check the
capability with `gnoblinctl capabilities`.

The `prompt-broker` capability reports `prompts_disabled` while `enabled` is
`false`.

## Reload behavior

Changes apply when Gnoblin reloads the configuration. Setting `enabled = true`
starts the prompter. Setting `enabled = false` releases both bus names and
cancels every prompt that is waiting.
