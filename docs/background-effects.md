# Standard background effects

Gnoblin supports version 1 of `ext_background_effect_manager_v1` and advertises
its `blur` capability. The protocol is the Wayland staging specification
[`ext-background-effect-v1`](https://wayland.app/protocols/ext-background-effect-v1).
The XML is copied unchanged from wayland-protocols, with its licence. Staging
protocols are standards-track protocols, but are not yet in the stable directory.

A client supplies a `wl_region` in surface-local coordinates. Gnoblin copies the
region when requested and applies it with the next surface commit, including
synchronised subsurface transactions. The renderer preserves holes and disjoint
rectangles, clips to the surface size, and can blur fully transparent pixels.
Destroying the effect or setting a NULL/empty region removes blur on the next
commit. Destroying the manager does not remove existing effect objects.

The region defines the material shape. Foreground icon alpha, text, borders and
separators cannot cut gaps in it. Surfaces which have never requested the
standard protocol retain the existing rule-based alpha mask. Once a client has
committed standard state, that state takes precedence over automatic whole-window
blur. An empty standard region must remain empty rather than trigger fallback.

The standard does not select blur strength. Gnoblin uses a default radius of 24
for client requests. Existing window rules override it, including `blur = 0`.
The Bingux dock therefore retains its configured radius of 48. The standard also
does not carry per-item animation opacity. Bingux retains `gnoblin-blur-fade-v1`
for item fades within a shared buffer. This metadata is applied in the same
surface transaction as the standard region.

Bingux requests standard regions for the dock and shared `ShellPopup` component.
The Qt client combines all requested items in each window into one region and
sends it during scene synchronisation, before the matching buffer commit. Rounded
corners and item transforms are included. When the compositor does not advertise
blur support, Bingux retains its previous blur material and region fallback.
Other Bingux surfaces continue to use their existing blur paths.

Protocol registration is limited to Gnoblin mode. It can be disabled at compositor
startup with this Lua configuration:

```lua
return { protocols = { ['ext-background-effect-v1'] = false } }
```

Native changes require a new compositor session. Rebuilding and installing does
not replace libraries already loaded by the current desktop.

## Validation

Run the protocol tests in an isolated compositor:

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/scripts/test-background-effect.sh" \
bash scripts/run-gnome-shell.sh
```

This checks capability negotiation, duplicate/dead object errors, copy and commit
semantics, empty regions, holes, disjoint rectangles, surface-size clipping,
buffer scale, destruction/recreation and synchronised subsurfaces through actual
rendered pixels. The same script checks that the global is absent in stock mode.

`scripts/test-blur-detail-coverage.py` checks the legacy path by default. Set
`GNOBLIN_TEST_STANDARD_BLUR=1` for the standard path. Both use the same seven icon,
separator and border samples. Set `STEAM_ICON` and `LOCALSEND_ICON` to installed
icons to include those exact images.

The paired Bingux `tests/standard-background.py` test opens the actual dock's
right-click menu. It checks both installed icons, backdrop leakage and the
configured radii. Run it with `MONITOR=1920x1080` and the matching `QS_TEST_BIN`.
For fallback coverage, set `EXPECT_STANDARD=0` and start the compositor with a
`GNOBLIN_CONFIG` file that disables this protocol and retains the dock/popup rules.
`tests/shared-buffer-fades.py` and `tests/panel-blur-fade.py` cover item and native
window fades with the migrated popup component.
