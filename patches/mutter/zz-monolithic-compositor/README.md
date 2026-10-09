# Monolithic compositor build

These patches finish the compositor build after the native-control and
supervised-runtime patches in `99-typed-window-api`.

`0001` replaces Mutter's standalone entry point with the Gnoblin supervisor and
compositor entry point. `0002` adds the compositor-owned Dear ImGui diagnostic
panel sources. `0008` links the plugin and ImGui sources directly into Gnoblin
and selects the built-in plugin type instead of loading a module. Gnoblin-owned source files are supplied by the
overlay manifests under `src/`; keep substantial implementation there and use
these patches only for the Mutter build integration.

Patch discovery sorts directory and file paths, so this group follows the
existing `99-typed-window-api` series.
