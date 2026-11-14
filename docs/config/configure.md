# Configuration reference

Gnoblin's settings are the keys of one `defineConfig` call in `~/.config/gnoblin/init.ts`:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    windowManagement: { focusMode: "click" },
});
```

Defaults apply before your config loads, and your config is laid over them. A key you leave out keeps its default.
Write each setting name in camelCase: `focusMode` for `focus-mode`. The kebab-case spelling is accepted as well. An
unknown key fails the load with a suggestion, so a typo never goes unnoticed. Sizes use logical pixels.

The [defineConfig reference](/typescript-config) lists every key, how it combines with the stock config, and what
each value does. The pages below say more about the settings that are sections of the compositor's configuration.

## Settings

Each page names the `defineConfig` key it describes.

- [`windowManagement`](/config/configure/window_management): focus, raise, tiling and titlebar clicks.
- [`compositor`](/config/configure/compositor): bells, renderer, blur and the pointer locator.
- [`input`](/config/configure/input): [keyboard](/config/configure/input/keyboard),
  [mouse](/config/configure/input/mouse), [touchpad](/config/configure/input/touchpad),
  [trackball and pointing stick](/config/configure/input/trackball-pointing-stick), tablets and styluses.
- [`inputSources`](/config/configure/input_sources): keyboard layouts and input methods.
- [`monitors`](/config/configure/monitors): scale and privacy screen.
- [`touchpadGestures`](/config/configure/touchpad_gestures): swipes and pinches that start a command.
- [`location`](/config/configure/location): the GeoClue agent policy.
- [`keyring`](/config/configure/keyring): the built-in secret store.
- [`auth`](/config/configure/auth): the polkit authentication agent.
- [`layerShell`](/config/configure/layer_shell): focus when a layer-shell surface opens.
- [`protocols`](/config/configure/protocols): the Wayland protocols clients may use.
- [`appearance`](/config/configure/appearance): colour scheme, accent and accessibility preferences.
- [`dialogs`](/config/configure/dialogs): the look of the compositor's consent and passphrase dialogs.
- [`accessibility`](/config/configure/accessibility): the magnifier and colour filters.
- [`cursor`](/config/configure/cursor): theme, size and image providers.
- [`xwayland`](/config/configure/xwayland): the X11 server.
- [`autostart`](/config/configure/autostart): commands the session starts.

Two sections of the old configuration have no key. Shortcuts and keybindings are replaced by
[`keys`](/guides/shortcuts). `animations` and `portals` are read by nothing, so there is nothing to set: see
[portals](/config/configure/portals) for how to route a portal interface.

## Where a setting takes effect

Each page says whether a setting applies on a reload, at the next start of its process, or not at all. A setting that
is accepted and not in effect is logged as `not applied: ...`. A reload commits the whole document your file
describes, so a setting you delete from `init.ts` goes back to its default.

See [recipes](/recipes/) for complete examples, [files and load order](/guides/files_and_load_order) for splitting a
config over files, and [configuration files](/config/configure/config-files) for where the file lives.
