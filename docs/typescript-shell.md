# TypeScript shell

The TypeScript shell is a preview of the shell language that replaces Lua. It runs in the runtime role, beside the
Lua host, and places and arranges windows from `init.ts`. Select it with an environment variable; unset, the session
runs the Lua shell as before.

```sh
GNOBLIN_SHELL_VM=js gnoblin
```

## Where the files go

The shell reads `init.ts` (else `init.js`) from `$XDG_CONFIG_HOME/gnoblin`, normally `~/.config/gnoblin`, and the
modules it imports from the same directory. Run `gnoblinctl types init` there for editor types.

## With no `init.ts`

A session with no `init.ts` is not a bare shell. It runs the stock default config as a real runtime config:

- one floating workspace, `desktop`, which is shown;
- `Super+1` to `Super+9` show the workspace at that position (the first, whatever it is called), sliding over a
  quarter of a second, and do nothing when there is none;
- `Super+Left` and `Super+Right` move focus between the columns of a `strip` workspace and do nothing on another layout;
- a three-finger swipe slides between workspaces: the windows follow the fingers, and lifting after a quarter of the
  travel finishes the slide;
- the volume, microphone and media keys run `wpctl` and `playerctl`: `XF86AudioRaiseVolume`,
  `XF86AudioLowerVolume`, `XF86AudioMute`, `XF86AudioMicMute`, `XF86AudioPlay`, `XF86AudioNext` and `XF86AudioPrev`;
- the floating layout's own pointer bindings: `Super+MouseLeft` drag moves a window and `Super+MouseRight` drag
  resizes it;
- a titlebar on every window (`decorations`).

`Super+space` is bound when the config names a `launcher`, a command line or an argv that it runs:
`defineConfig({ launcher: "wofi --show drun" })`. A `keys` entry for `Super+space` wins. The Lua shell's own command
entry is not ported.

The layouts a workspace can name are `floating`, `master-stack`, `grid`, `monocle`, `dwindle`, `kiosk`, `strip` and
`carousel-3d`; each is also a class (`DwindleLayout`, `KioskLayout`, `StripLayout`, `Carousel3dLayout`) with its own
options.

Creating `init.ts` later loads it at once. An `init.ts` that fails to load at startup is reported in the log and
leaves the stock config running until the file is fixed.

## `defineConfig` layers over the stock config

```ts
import { defineConfig, spawn } from "gnoblin";

export default defineConfig({
    workspaces: [
        { id: "web", layout: "master-stack" },
        { id: "dev", layout: "grid" },
    ],
    keys: {
        "Super+Return": spawn("foot"),
        XF86AudioMute: spawn("pactl", "set-sink-mute", "@DEFAULT_SINK@", "toggle"),
    },
});
```

A `defineConfig` is not the whole config: it is laid over the stock one, so a config that says nothing about keys
still has the media keys. The rule:

| Key                              | Merge                                                                               |
| -------------------------------- | ----------------------------------------------------------------------------------- |
| `workspaces`                     | Replaced as a whole. Leave it out, or empty, to keep the stock `desktop`.           |
| `rules`                          | Replaced as a whole.                                                                |
| `keys`                           | Merged by trigger. Your key replaces the stock key for the same trigger.            |
| `launcher`                       | Yours, when you give one.                                                           |
| `gaps`, `style`, `outputs`, `on` | Merged by key, your value winning. A nested object, such as a shadow, is one value. |
| `setup`                          | Yours, when you give one.                                                           |
| `dialogs`, `appearance`, ...     | The document settings (below), merged by key, your value winning.                   |

`decorations` is not layered. The stock config turns titlebars on, and a `defineConfig` that does not ask for
`decorations: true` has none.

Triggers are compared by their canonical spelling, so `"super+1"` replaces the stock `"Super+1"`. A stock key that
shows a workspace by id, with `show("desktop")`, is dropped when your config does not declare that workspace, because
it would name nothing. The stock `Super+1` to `Super+9` show by position, so they stay.

A `init.ts` that does not call `defineConfig` is a program, not a config. It runs as written; the stock keys are not
added, and a config that declares no workspace still gets the stock `desktop`.

## Settings of the config document

Keys that are settings of the compositor's config document (the ones `gnoblin.configure` takes in Lua) are applied
from `defineConfig` too. Write them in camelCase: `windowManagement`, `buttonLayout`, `reducedMotion`.

```ts
export default defineConfig({
    appearance: { colorScheme: "prefer-dark", reducedMotion: true },
    cursor: { size: 28 },
    dialogs: { style: { radius: 14, buttonLayout: "stretch" } },
});
```

The sections are:

- `compositor`, `windowManagement`, `cursor`, `appearance`, `dialogs`;
- `input`, `inputSources`, `accessibility`, `layerShell`, `keyring`;
- `auth`, `location`, `compat`, `capture`, `monitors`, `xwayland`, `logging`.

Each is the section of that name in the [configuration reference](/config/configure). The compositor applies a
setting the way the reference says: live, at the next start, or not at all. A setting that is not in effect is
logged as `not applied: ...`.

The shell lays your settings over the document the compositor holds and commits the whole result, so whatever the
Lua `init.lua` set is kept.

- A value the compositor's validation refuses fails the reload with the key path and leaves the running config, and
  the document, as they were.
- A reload that drops a key puts it back to its default.
- Sections that are lists, or tables keyed by names you choose, stay in `init.lua`. They are `workspaces`,
  `autostart`, `shortcuts`, `protocols` and `touchpadGestures`.

`style` and `outputs` are accepted and stored but have no effect: they need native style objects.

## Listeners and `setup`

`on` is sugar for listeners on `gnoblin.clients`: `clientAdded`, `clientRemoved`, `titleChanged` and `focusChanged`.
A listener that throws is logged once and the others still run.

```ts
export default defineConfig({
    on: {
        clientAdded: (client) => client.title === "Calculator" && (client.width = 320),
        focusChanged: (client, focused) => focused && gnoblin.spawn(["notify-send", client.title]),
    },
    setup(gnoblin) {
        // Runs once, after everything above, with the whole object API.
        gnoblin.dialogs.addListener(new NoScreenshots());
    },
});
```

`setup` runs inside the same time budget as the rest of the file, and a throw in it fails the config like any other
error in `init.ts`.

## Dialogs

The compositor draws its consent and passphrase dialogs itself. A script can restyle them (the `dialogs` key above)
and refuse a request before its dialog opens:

```ts
import { DialogListener } from "gnoblin";

class NoScreenshots extends DialogListener {
    onConsentRequest(request) {
        if (request.subject === "Screenshot") {
            request.decline("screenshots are off");
        }
    }
}

gnoblin.dialogs.addListener(new NoScreenshots());
```

`gnoblin.dialogs.addListener` and `removeListener` attach and detach a `DialogListener`. Its methods are optional:

- `onConsentRequest(request)` for a portal consent request;
- `onSecretRequest(request)` for a passphrase request.

A request has these members:

- `kind`, `consent` or `secret`;
- `appId`, the app id the compositor verified;
- `subject`, what is asked about;
- `deadline`;
- `decline(reason)`, which refuses the request.

There is no way to allow a request and none to read a secret. Each request is shown to the shell for 250 ms. A
listener that throws, runs past its budget or does nothing leaves it to the built-in dialog. See
[`gnoblin.configure.dialogs`](/config/configure/dialogs).

## Reloading

Saving `init.ts` or any module it imports reloads the shell after a short pause. `gnoblinctl config reload` reloads it
on request: see [`gnoblinctl config reload`](/gnoblinctl#config). Windows keep their identity through a reload, and
the new config is told about every window that exists.

A file that fails to load (a syntax error, a throw, a script that runs away) leaves the running config in place. The
failure is logged once as `file:line:column: message`, in your `.ts` coordinates, and `gnoblinctl config reload`
exits 1 with the same text.

Set `GNOBLIN_JS_WATCH=0` to turn the watcher off, for a config directory on a file system that cannot be watched. The
shell then reloads only when asked.

## When the shell stops

The shell is a client of the compositor. If it exits or is killed, the compositor keeps running and answers every
client. With no shell holding the tree:

- a window that maps is placed by the compositor's own policy, a cascade from 32, 32;
- windows the shell had moved go back to the places the compositor gave them.

The guardian restarts the runtime role. The shell attaches again, learns of every window, and applies the config
again. No window changes its identity.
