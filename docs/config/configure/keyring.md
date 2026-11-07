# gnoblin.configure.keyring

Choose which program provides the Secret Service on the session bus:

```lua
gnoblin.configure {
    keyring = {
        provider = "gnoblin",
    },
}
```

Programs such as libsecret, browsers and `secret-tool` look up passwords on
`org.freedesktop.secrets`. The `provider` field names the program that serves
that name.

| Field      | Type and accepted values | Default   | Effect                                                                                                    |
| ---------- | ------------------------ | --------- | --------------------------------------------------------------------------------------------------------- |
| `provider` | Nonempty text            | `gnoblin` | `gnoblin` serves the Secret Service from Gnoblin's store. Any other name leaves the name to that program. |

No other field is accepted. Gnoblin rejects a `keyring` value that is not a
table, a `provider` value that is not nonempty text, and any other key.

## What changes when it is on

With `provider = "gnoblin"`, Gnoblin requests `org.freedesktop.secrets` with
`DO_NOT_QUEUE`. If another program already owns the name, Gnoblin does not take
it and logs why. The two can run side by side: the one that lost the name does
not serve.

With any other value, Gnoblin does not request the name at all. The program
named in `provider` keeps serving it.

The store lives in `$XDG_DATA_HOME/gnoblin/keyring.db`, or under
`$HOME/.local/share/gnoblin/` when `XDG_DATA_HOME` is unset. It stays locked
until the login password reaches it.

## When a change applies

Gnoblin reads `provider` when the services role starts. A change takes effect at
the next session start.
