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

`decorations` is not layered. The stock config turns titlebars on, and a `defineConfig` that does not ask for
`decorations: true` has none.

Triggers are compared by their canonical spelling, so `"super+1"` replaces the stock `"Super+1"`. A stock key that
shows a workspace by id, with `show("desktop")`, is dropped when your config does not declare that workspace, because
it would name nothing. The stock `Super+1` to `Super+9` show by position, so they stay.

A `init.ts` that does not call `defineConfig` is a program, not a config. It runs as written; the stock keys are not
added, and a config that declares no workspace still gets the stock `desktop`.

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
