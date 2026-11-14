# Portals

Gnoblin's portal backend answers some XDG Desktop Portal interfaces for apps in the session. This page says which,
how to route an interface to another backend, and which settings it publishes.

`portals` is not a `defineConfig` key. The `portals` section of the config document is carried and no process reads
it, so a config that sets it has no effect. Route portals with a standard `portals.conf` file instead, as
[below](#route-an-interface-to-another-backend).

Gnoblin routes interfaces only. It does not manage app permissions: the selected backend decides what each app may
do. The Gnoblin backend shows its normal consent dialogs. A shell can also ship its own backend and route interfaces
to it; Bingux does this.

Consent is per request. A caller with an app ID, such as a Flatpak or sandboxed app, that is refused is not asked again
for a short time.

A caller with no app ID, such as an unsandboxed host app, has no identity the backend can see. Each of its requests
prompts on its own, and no earlier answer is reused. Requests wait behind open dialogs in a bounded queue. When the
queue is full, the request fails.

## What Gnoblin serves

- ScreenCast, Screenshot, RemoteDesktop, InputCapture, GlobalShortcuts, Clipboard, Settings, Inhibit, and Access (the
  accessibility consent dialog).

It does not serve FileChooser, AppChooser, Print, Account, Email, Notification, Wallpaper, Background, Lockdown, or
DynamicLauncher. The shipped `portals.conf` routes them to `gtk`, then `kde`, then any installed backend, so
Gnoblin never answers them. Add a route to `gnoblin` for one of them only once it serves that interface.

Screenshot takes one picture of the whole output. The compositor sends the PNG, and the portal writes it to your
Pictures folder. The response carries its `file://` URI.

The Gnoblin consent dialog asks first. Deny has focus; Tab moves to Allow and Return presses it. Escape or Deny
returns response 1. To restyle the dialog or refuse a request from a script, see [dialogs](/config/configure/dialogs).

A request with `interactive` set to true asks you to choose an area. The portal refuses it by name after consent.

PickColor is refused. The compositor does not expose colour sampling.

ScreenCast shares an output (`types` 1) or one window (`types` 2):

- The portal API cannot name a window, so Gnoblin has a key of its own in `SelectSources`: `window`, a `t` holding the
  `id` of the window's `window` object, which `gnoblinctl window list` prints.
- A selection that asks for windows only and names none is refused at `SelectSources` (`InvalidArgs`), before the user
  is asked.
- The user is asked as for any share, the privacy indicator is on while it runs, and a lock ends it.
- A window share asks about that window every time. The question names the window by its app id and process first,
  then its title as quoted text, both as the window reports them: another window can pick any title, so read the app
  id. An earlier yes for the app, or for another window, is never reused. The compositor refuses the share
  (`no-consent`) unless it saw you allow that very window, and refuses the question (`no-window`) for a window it does
  not show.
- A window stream is the window's rectangle of the screen at the window's size, so it follows the window when it is
  moved or resized, and another window on top of it shows in the overlap. A secure window is the secure treatment and
  never its pixels. The stream ends when the window closes or the session locks.
- The `Start` reply carries `source_type` (`u`), `position` and `size` (`(ii)`, layout pixels). `position` is where
  the window was when the stream started: a later move shows in the stream's pixels and in a new share, not in the
  property, because the portal has no way to update it.
- Only a caller the frontend says is not sandboxed may share a window.

## Route an interface to another backend

The portal service reads standard `portals.conf` files. The backend lists are ordered: for each request, the portal
service tries the first installed backend in the list that implements that interface. Common backend IDs include
`gtk`, `gnome`, `kde`, and `gnoblin`. Use IDs provided by the backends installed on your system.

The Gnoblin session installation provides a desktop-specific default that prefers `gnoblin`, then tries any installed
backend. The optional `xdg-desktop-portal-gnoblin` package supplies that backend. The portal service selects the
Gnoblin file only in a Gnoblin session; other desktop sessions keep their own portal configuration.

To prefer GTK for every interface and keep Gnoblin first for ScreenCast, write
`~/.config/xdg-desktop-portal/portals.conf`:

```ini
[preferred]
default=gtk;gnoblin;*
org.freedesktop.impl.portal.ScreenCast=gnoblin;gtk;*
```

The special backend value `*` selects the first installed implementation in name order, and `none` disables that
portal interface. The interface name is the D-Bus name the portal service supports. See the
[XDG portal configuration reference](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html) for the file
format. The portal service reads the file when it starts, so end the running service for the change to take effect:

```sh
systemctl --user restart xdg-desktop-portal
```

## Settings interface

The `org.freedesktop.impl.portal.Settings` interface answers `ReadAll` and `Read` from the appearance values Gnoblin
already holds. A toolkit reads them through the portal, and each change is announced with `SettingChanged`. The
backend publishes these keys:

| Namespace                          | Key                                                      | Source                                                   |
| ---------------------------------- | -------------------------------------------------------- | -------------------------------------------------------- |
| `org.freedesktop.appearance`       | `color-scheme`                                           | [`appearance.colorScheme`](/config/configure/appearance) |
| `org.freedesktop.appearance`       | `contrast`                                               | `appearance.highContrast`: 1 on, 0 off                   |
| `org.freedesktop.appearance`       | `reduced-motion`                                         | `appearance.reducedMotion`: 1 on, 0 off                  |
| `org.freedesktop.appearance`       | `accent-color`                                           | `appearance.accent`                                      |
| `org.gnome.desktop.interface`      | `gtk-theme`, `icon-theme`                                | theme names                                              |
| `org.gnome.desktop.interface`      | `cursor-theme`, `cursor-size`                            | [`cursor`](/config/configure/cursor)                     |
| `org.gnome.desktop.interface`      | `font-name`, `document-font-name`, `monospace-font-name` | fonts                                                    |
| `org.gnome.desktop.interface`      | `text-scaling-factor`                                    | text scale                                               |
| `org.gnome.desktop.interface`      | `button-layout`                                          | window buttons                                           |
| `org.gnome.desktop.interface`      | `enable-animations`                                      | the inverse of `reducedMotion`                           |
| `org.gnome.desktop.a11y.interface` | `high-contrast`                                          | `appearance.highContrast`                                |

An accent colour that is not set is published as the sentinel the Settings portal defines: all three components out of
range. A key Gnoblin does not publish is absent, not guessed.

The contrast preference reaches GTK through the dconf layer as well, under `org.gnome.desktop.a11y.interface`
`high-contrast`. `enable-animations` is the inverse of `reducedMotion` and is written by the same layer, so GTK stops
animating when reduced motion is on.

Gnoblin does not publish Qt or KDE settings through this interface. A Qt or KDE toolkit needs its own bridge to read
these values.
