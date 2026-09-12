# Developer console

Implementation checklist:

- [x] Add JavaScript evaluation, retained bindings and object inspection.
- [x] Add the top-edge console, keyboard input, history and completion.
- [x] Replace the Gnoblin Run dialog and Looking Glass entry points.
- [x] Verify keyboard focus, lock behaviour, stacking and rendered output.
- [x] Commit and push each completed part.

`Alt+F2` opens the console in the primary unlocked Gnoblin session. It is a
top-edge modal above the Shell chrome, with a multiline input, retained
bindings, top-level `await`, result history (`r(id)` and `$_`), completion,
descriptor-safe object inspection and selectable stack traces. `Escape` closes
it, and a lock transition closes it before the stock unlock UI takes over.

The JavaScript context is intentionally compositor-local: it exposes `global`,
`Main`, GNOME Shell libraries and `windows()`, while the remote D-Bus control
surface remains separate. Build the Shell theme resource after changing the
console Sass file with:

```sh
ninja -C build/gnome-shell data/gnome-shell-theme.gresource
meson install -C build/gnome-shell --no-rebuild
```

Run the focused integration check with `just gnome-developer-console-verify`.
