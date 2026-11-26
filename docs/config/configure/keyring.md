# Keyring

Gnoblin runs a built-in keyring. Programs such as libsecret, browsers and `secret-tool` look up passwords on
`org.freedesktop.secrets`, and Gnoblin serves that name from its own store. Whether it does is the `keyring.enabled`
setting, which no shell object sets yet in any language: see [settings with no object yet](/js-settings#dropped-settings).
The keyring stays enabled.

## What the keyring does

Gnoblin requests `org.freedesktop.secrets` with
`DO_NOT_QUEUE`. If another program already owns the name, Gnoblin does not take
it and logs why. The two can run side by side: the one that lost the name does
not serve.

The store lives in `$XDG_DATA_HOME/gnoblin/keyring.db`, or under
`$HOME/.local/share/gnoblin/` when `XDG_DATA_HOME` is unset. It stays locked
until the login password reaches it. When the login session has no password, the
keyring asks for it through a passphrase request. A script may refuse the request (see
[passphrase requests](/shell-api/requests#keyring-passphrase)); with no script answering it, the compositor
draws its own passphrase dialog.

While the store is locked, an app that saves a new password makes the keyring ask for
the password first. The app's save finishes after you answer. If the request is
dismissed or answered wrongly, nothing is saved. Searching and reading
while locked return nothing until the store is unlocked.

## When the keyring starts

The services role waits for the compositor to accept the session's configuration before it decides. If nothing is
accepted within 20 seconds it uses the flag of the last accepted configuration file, and then the schema default, which
is on. The log names which of these decided: `accepted revision`, `last accepted file` or `schema default`.

The Secret portal reads the last accepted file when it starts, not the accepted document.

## Apps reach gnome-keyring instead

While it is on, Gnoblin does not stop other programs from taking
`org.freedesktop.secrets`. Apps then reach that program, and their secrets do not
go to Gnoblin's store.

This page lists each route, what Gnoblin does about it, and
whether that was tested. "Unverified" means the behaviour is read from the package
files or the code, but was not run on a live session.

| Route                                                                                                            | What Gnoblin does                                                                                          | Status                                                                            |
| ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `pam_gnome_keyring` in the login PAM stack                                                                       | Nothing. PAM runs before the session                                                                       | Outside Gnoblin's control. No file in `/etc/pam.d` names it on the tested machine |
| D-Bus activation: `/usr/share/dbus-1/services/org.freedesktop.secrets.service` (shipped by gnome-keyring)        | Nothing. Gnoblin does not ship a file that overrides it                                                    | Unverified. Starts when a client calls the name before anything owns it           |
| systemd user units `gnome-keyring-daemon.socket` and `.service` (`WantedBy=sockets.target` and `default.target`) | Nothing. Gnoblin does not mask them                                                                        | Unverified in a Gnoblin session. Disabled on the tested machine                   |
| XDG autostart `gnome-keyring-secrets`, `-ssh`, `-pkcs11` (`OnlyShowIn=GNOME;Unity;MATE;`)                        | Gnoblin does not read XDG autostart files, and sets `XDG_CURRENT_DESKTOP=Gnoblin`                          | Read from the files; Gnoblin's non-reading is a code search                       |
| `xdg-desktop-portal` Secret request, through `gnome-keyring.portal` (`DBusName=org.freedesktop.secrets`)         | `portals.conf` routes `Secret` to `gnoblin` alone, with no fallback, so no keyring backend answers         | Routing test, static. Live refusal unverified                                     |
| gnome-keyring already owns the name when Gnoblin starts                                                          | The services role logs a warning naming the owner, and does not serve or take the name back                | Integration test on a private bus                                                 |
| Gnoblin owns the name, then gnome-keyring starts                                                                 | Gnoblin does not set `ALLOW_REPLACEMENT`, so a later request with `REPLACE_EXISTING` is refused by the bus | Bus rule only. Whether gnome-keyring sends `REPLACE_EXISTING` is unverified       |
| An app asks for a secret before the services role has claimed the name                                           | Nothing. The bus activates whatever file matches first                                                     | Unverified. Start order is not tested                                             |

Gnoblin never sets `ALLOW_REPLACEMENT` and never takes the name from another owner.

Known gap: with the keyring off, the Secret portal has no provider. The shipped
`portals.conf` routes `org.freedesktop.impl.portal.Secret=gnoblin`, and nothing rewrites that
line from the setting, so Secret portal requests are refused in that mode. Earlier builds routed
`Secret=gtk;*`, which did reach any installed backend.

To get a provider back with the keyring off, add this line under `[preferred]` in your own
`~/.config/xdg-desktop-portal/portals.conf`:

```ini
org.freedesktop.impl.portal.Secret=gnome-keyring
```

This override is not tested by Gnoblin. It is the line to add, not a verified fix.

To check who holds the name in a session, run:

```sh
busctl --user status org.freedesktop.secrets
journalctl --user -b --no-pager | rg 'keyring-service|Secret Service'
```

To stop gnome-keyring from starting at login, remove its autostart entries and
its PAM line, and mask its user units:

```sh
systemctl --user mask gnome-keyring-daemon.service gnome-keyring-daemon.socket
```

The autostart check reads each word of an entry's command. A shell string that
starts gnome-keyring inside one word, such as `sh -c "gnome-keyring-daemon --start"`,
is not caught. Pass the program as its own word, or remove the entry.
