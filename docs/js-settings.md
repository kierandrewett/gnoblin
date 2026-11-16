# Settings in init.ts

`init.ts` has no config object. It is a program that runs against live objects, and a setting is a property you set
on one of them. This page lists what you can set, one example for each kind of object, and the settings that have no
object to set them on yet.

There is no config object to fill in. Delete a statement and that part of the setup is gone after the next reload. See
[the TypeScript shell](/typescript-shell) for where the file lives and how it reloads.

## Device settings

Devices are in `gnoblin.input.devices`. A `Keyboard`, `Mouse` or `Touchpad` has the settings below, and `undefined`
puts one back to the system's value. The compositor keeps one setting for each kind of device, so a write on one mouse
is the setting of every mouse.

```ts
import { Keyboard, Mouse, Touchpad } from "gnoblin";

for (const device of gnoblin.input.devices) {
    if (device instanceof Keyboard) {
        device.repeatRate = 40;
        device.repeatDelay = 250;
    }
    if (device instanceof Mouse) {
        device.sensitivity = 0.5;
        device.acceleration = "flat";
    }
    if (device instanceof Touchpad) {
        device.naturalScroll = true;
        device.tapToClick = true;
    }
}
```

Each device page lists the properties and their ranges: [keyboard](/config/configure/input/keyboard),
[mouse](/config/configure/input/mouse) and [touchpad](/config/configure/input/touchpad).

## Outputs

An `OutputProfile` describes the mode, scale, transform and position of the output that matches it, and survives
hotplug. Add it to `gnoblin.outputs.profiles`. See [monitors](/config/configure/monitors).

```ts
import { OutputProfile } from "gnoblin";

const main = new OutputProfile({
    match: { connector: "DP-1" },
    mode: { width: 3440, height: 1440, refreshRate: 144 },
    scale: 1.25,
    position: { x: 0, y: 0 },
});
gnoblin.outputs.profiles.add(main);
gnoblin.outputs.profiles.add(
    new OutputProfile({ match: { connector: "HDMI-A-1" }, position: { rightOf: main, align: "top" } }),
);
```

## Window style

`gnoblin.clients.style` is the look of every window. A workspace has its own `workspace.clients.style`, and
`gnoblin.clients.where({ appId })` selects some windows. See [window style](/js-style).

```ts
gnoblin.clients.style.cornerRadius = 12;
gnoblin.clients.where({ appId: "foot" }).style.opacity = 0.9;
```

## Workspaces and gaps

A workspace is an object with a layout. The `gaps` of a workspace set its outer and inner gap, and a layout's own `gap`
overrides the inner one. See [workspaces](/shell-api/workspaces).

```ts
import { MasterStackLayout, Workspace } from "gnoblin";

gnoblin.workspaces.add(
    new Workspace({
        id: "dev",
        name: "Development",
        layout: new MasterStackLayout({ gap: 12 }),
        gaps: { outer: 8 },
    }),
);
```

## Keys and gestures

`input.bind` binds a key, a pointer button or a touchpad gesture to a function. A trigger that is already bound at the
same priority is refused, so delete the stock line first. See [shortcuts](/guides/shortcuts).

```ts
const input = gnoblin.input.primary;
input.bind("Super+Return", () => {
    gnoblin.spawn(["foot"]);
});
input.bind("Swipe4Up", () => {
    input.focusedClient?.close();
});
```

## Dialogs

`gnoblin.dialogs` shows you each consent or passphrase request before its dialog opens, and a listener may refuse it.
See [dialogs](/config/configure/dialogs).

```ts
import { DialogListener } from "gnoblin";

gnoblin.dialogs.addListener(
    new (class extends DialogListener {
        onConsentRequest(request: { subject: string; decline(reason: string): void }) {
            if (request.subject === "Screenshot") {
                request.decline("screenshots are off");
            }
        }
    })(),
);
```

## Settings with no TypeScript home yet {#dropped-settings}

No object of the shell carries these settings, so `init.ts` cannot set them. The shell keeps its default for each. The
table says what stays in each case.

| Setting                                                                                                                                                            | What stays                                       |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------ |
| Window rules (`rules`)                                                                                                                                             | Listeners and `where(...).style`: see above      |
| Focus mode, new-window focus, raise, auto raise, `edgeTiling`, `dragConstraint`                                                                                    | Click focus, raise on click, edge snapping on    |
| `autostart`                                                                                                                                                        | `gnoblin.spawn` at the top of `init.ts`          |
| `protocols`, `keyring`, `compositor`, `appearance`, `accessibility`, `layerShell`, `auth`, `location`, `xwayland`, `cursor`, `monitors`, `dialogs.style`           | The default of each                              |
| `touchpadGestures` (drawn paths), `workspaceSets`, `launcher`, `inputSources`, tablets, styluses, trackballs, pointing sticks, `orientationLock`, keyboard options | A swipe or pinch is `input.bind`; the rest: none |

Window rules, titlebar clicks and the other seat policy have no replacement yet.
