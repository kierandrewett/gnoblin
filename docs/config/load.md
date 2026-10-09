# gnoblin.load

Evaluate another Lua config file in the current config state. Relative paths resolve from the file making the call. A glob is sorted before evaluation; a pattern with no matches is ignored. See the [file loading guide](/guides/files_and_load_order).

```lua
gnoblin.load("conf.d/**/*.lua")
```

A missing explicit file is an error. Config loading is limited to 32 nested files.

## Lua `require`

`require` is a global function, not a member of `gnoblin`. It loads a local module from beside the calling file or its `lua/` directory, caches the result for the current reload, and returns it. It does not support Lua's normal installed module search paths.

```lua
local appearance = require("appearance")
gnoblin.configure(appearance)
```

Module names cannot contain `..`; use `gnoblin.load("parts/motion.lua")` for explicit subdirectories.

## Recover from a configuration error

### Create or restore defaults

If no user config exists, Gnoblin loads its built-in configuration tree
directly from the session binary. Run `gnoblinctl init` to create an editable
copy in `~/.config/gnoblin/`. The entry point loads focused files in `config/`. The command leaves an existing configuration unchanged.

To replace the whole configuration folder with the embedded defaults, run
`gnoblinctl config restore-default`. It moves the previous folder to a sibling
`gnoblin.recovery-UUID` backup, then installs the full default tree. Files in
the previous folder are preserved in that backup. In a running session, restore
also reloads the config, stopping Gnoblin-owned autostart groups that are absent
from the defaults.

### Login fallback

Gnoblin starts the compositor with its embedded Lua configuration, then applies
the user configuration through a reload transaction. At login it recovers in
this order:

1. **Ignore the broken item.** If a setting or list entry is invalid, or an
   included file fails to load, Gnoblin ignores only that item or file and
   applies everything else.
2. **Use the last accepted settings.** If the configuration cannot be used at
   all, for example the root `init.lua` has a syntax error, Gnoblin uses the
   last accepted settings for that config path.
3. **Use the embedded defaults.** They stay active if no accepted settings
   exist or the compositor rejects them.

For example, one misspelled keybinding action does not stop your cursor,
window, or input settings from loading. Each ignored item is written to the
session log as `ignored <setting>: <reason>` and added to the recovery notice.
A skipped file is logged as `ignored file <path>: <error>`.

Config-owned clients start only after their configuration is accepted.

A reload is stricter than login. A reload with an invalid setting is rejected
and the active settings stay in place.

Recovery resolves imports from the embedded tree, so broken user files cannot
prevent the built-in configuration from loading. It writes no default files
to the user config folder.

Startup validation also checks native keybinding groups and action names
against the Mutter build. An unsupported or misspelled action is ignored.

Gnoblin leaves failed files unchanged and logs the error and fallback choice.
It also shows the failure in a top-left ImGui panel at login or after a rejected
reload.

Saved settings cannot restore Lua callbacks. Recovery uses the default input
callbacks, and callbacks from failed files do not run.

### Worker recovery

If the Lua worker exits unexpectedly, Gnoblin reconnects it from the settings
accepted by the compositor, then applies embedded defaults. It does not
evaluate the user files again.

Input callbacks wait until the default configuration is accepted. The recovery
panel reports that the worker restarted.

Recovery needs a running compositor. A native compositor crash or a library
loading failure ends the session before the panel can display. See
[login troubleshooting](/troubleshooting#the-source-build-returns-to-the-login-screen)
for the logs to inspect.

### Fix and reload

When the recovery panel appears, open the configuration and fix the error.
**Check and reload configuration** evaluates the root file and every included
file before applying anything. If any file still fails, Gnoblin rejects the
reload and keeps the active settings. The panel shows the failure reason. A
successful reload clears the notice.

## Type definition

```lua
gnoblin.load(path_or_glob) -- string; missing explicit paths are errors
require(module_name) -- string; local module name, no `..`
```
