# Client-requested background blur

Gnoblin implements `ext-background-effect-v1` with the `blur` capability.
A shell can use it to specify exactly which parts of a bar or popup need blur,
without blurring transparent margins or shadows. This page is for client
developers; desktop users can use [window effects](/guides/window_effects).

The [protocol XML](https://wayland.app/protocols/ext-background-effect-v1)
is from Wayland's staging specifications.

## Submit a region

1. Create a `wl_region` in surface-local coordinates.
2. Set it as the effect's blur region.
3. Commit the surface with the matching buffer.

The compositor copies the region. Holes and disjoint rectangles are preserved;
the result is clipped to the surface size. Synchronised subsurfaces apply it
with their surface transaction.

## Clear or destroy

A null/empty region or destroyed effect removes blur on the next commit.
Destroying the manager does not destroy existing effects.

After a client sets a blur region through this protocol, Gnoblin uses that
region instead of guessing from pixel transparency. Sending an empty region
turns blur off; it does not return to automatic detection.

## Strength and fades

The protocol supplies shape, not strength.
Gnoblin defaults client requests to radius 24; a window rule can override it,
including `blur = 0`.

Foreground icons and text do not punch holes in the material region.
Client item animations use separate [blur-fade metadata](blur-fades.md).

## Disable the protocol

In `init.lua`:

```lua
gnoblin.configure {
    protocols = {
        ext_background_effect_v1 = false,
    },
}
```

Log out and back in. The protocol is only advertised in Gnoblin mode.

## Rendering

Layer surfaces in the same stack layer share a backdrop captured before the
layer paints. Adjacent panels therefore do not sample each other's tint.
Offscreen captures use the same grouping.

Bingux combines material regions per window during scene synchronisation.
Its standard regions exclude shadows; clients without protocol support retain
their fallback behavior.

## Tests

```sh
GNOBLIN_PREFIX="$PWD/install" \
GNOBLIN_TEST_DBUS_CLIENT="$PWD/tests/test-background-effect.sh" \
bash scripts/run-gnome-shell.sh
```

Covers negotiation, object errors, commit timing, holes, clipping, scale,
destruction and subsurfaces with rendered pixels.

Additional checks:

- `tests/test-blur-surface-joins.py`: adjoining surfaces and narrow panels.
- `tests/test-blur-detail-coverage.py`: icons, separators and borders.
- Bingux `tests/standard-background.py`: real dock menus.
- Bingux `tests/popup-shadow-blur.py`: shadows excluded from blur.

Use `GNOBLIN_TEST_STANDARD_BLUR=1` for standard-region paths where supported.
Provide the matching Bingux effects module through `QML_IMPORT_PATH`.
