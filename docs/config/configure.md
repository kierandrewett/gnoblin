# Configuration reference

A setting is a property of an object that your shell reaches from its root: a device, an output profile, the clients'
style, a workspace, a binding.

The root is the `gnoblin` global in TypeScript (`shell.ts`) and Python (`shell.py`), and the `Gnoblin` value passed to
`main` in a Rust shell library. The same settings exist in all three languages, spelled in each language's own style.
[Settings in your shell](/js-settings) lists them all with an example for each, in TypeScript, Python and Rust.

A setting you do not write keeps the shell's default, and an out-of-range value throws an error that names the
property, so a typo never goes unnoticed. Sizes use logical pixels.

## Settings

- [`input`](/config/configure/input): what each device object carries, with the pages for the
  [keyboard](/config/configure/input/keyboard), [mouse](/config/configure/input/mouse) and
  [touchpad](/config/configure/input/touchpad).
- [`monitors`](/config/configure/monitors): mode, scale, transform and position of an output.
- [`touchpadGestures`](/config/configure/touchpad_gestures): swipes and pinches that run a function.
- [`windowManagement`](/config/configure/window_management): focus, raise, tiling and titlebar clicks.
- [`dialogs`](/config/configure/dialogs): the compositor's consent and passphrase dialogs.
- [`location`](/config/configure/location): the GeoClue agent.
- [`keyring`](/config/configure/keyring): the built-in secret store.
- [`portals`](/config/configure/portals): which backend answers a portal interface.

Every other setting of the old configuration document, among them the colour scheme, the cursor theme, the
magnifier, `autostart`, `protocols` and `xwayland`, has no typed home yet: see
[settings with no object yet](/js-settings#dropped-settings). Window rules are in that list as well.

See [recipes](/recipes/) for complete examples, [files and load order](/guides/files_and_load_order) for splitting a
config over files, and [configuration files](/config/configure/config-files) for where the file lives in each language.
