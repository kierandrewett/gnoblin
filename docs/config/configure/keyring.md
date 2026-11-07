# gnoblin.configure.keyring

Choose whether Gnoblin runs its built-in keyring:

```lua
gnoblin.configure {
    keyring = {
        enabled = true,
    },
}
```

Programs such as libsecret, browsers and `secret-tool` look up passwords on
`org.freedesktop.secrets`. With `enabled = true`, Gnoblin serves that name from
its own store. With `enabled = false`, Gnoblin serves nothing and another program
(gnome-keyring, KeePassXC, KWallet) or no keyring at all provides it.

| Field     | Type and accepted values | Default | Effect                                                                                      |
| --------- | ------------------------ | ------- | ------------------------------------------------------------------------------------------- |
| `enabled` | Boolean                  | `true`  | Run Gnoblin's built-in keyring. Turn it off to use a keyring from another program, or none. |

No other field is accepted. Gnoblin rejects a `keyring` value that is not a
table, an `enabled` value that is not a boolean, and any other key.

## What changes when it is off

With `enabled = false`, the services role does not request
`org.freedesktop.secrets`, creates no store file or directory, and runs no
keyring code. The portal does not export the Secret object, so Gnoblin answers
no Secret request. The system's keyring answers them.

## What changes when it is on

With `enabled = true`, Gnoblin requests `org.freedesktop.secrets` with
`DO_NOT_QUEUE`. If another program already owns the name, Gnoblin does not take
it and logs why. The two can run side by side: the one that lost the name does
not serve.

The store lives in `$XDG_DATA_HOME/gnoblin/keyring.db`, or under
`$HOME/.local/share/gnoblin/` when `XDG_DATA_HOME` is unset. It stays locked
until the login password reaches it. When the login session has no password, the
keyring asks for it through a `prompt.secret` request that a shell answers. With
no shell claiming that request, the store stays locked. See
[passphrase prompts](/shell-api/passphrase-prompts).

## When a change applies

Gnoblin reads `keyring.enabled` when the services role starts. A change takes
effect at the next session start.
