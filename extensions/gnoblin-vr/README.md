# Gnoblin VR extension

`gnoblin-vr` is an optional OpenXR presentation client. It runs outside the
compositor and does not render scene content in this phase. Install the
extension package alongside Gnoblin, then opt in for the current user with:

```sh
systemctl --user enable --now gnoblin-vr.service
```

The service is not enabled by package installation and is not pulled into the
stock session target. Disable it with `systemctl --user disable --now
gnoblin-vr.service`.

The client uses the Khronos OpenXR loader. If
`$XDG_RUNTIME_DIR/monado_comp_ipc` is a Monado service socket and
`XR_RUNTIME_JSON` is unset, it selects an installed Monado runtime manifest.
Without an available runtime or headset, it exits successfully after writing
an `absent` status and a diagnostic under
`$XDG_RUNTIME_DIR/gnoblin-vr/status.ini`.

The extension Lua module is shipped at
`/usr/share/gnoblin-vr/lua/gnoblin-vr.lua`. Copy it into the configuration's
`lua/` module directory and assign `require("gnoblin-vr")` to
`gnoblin.outputs.xr`; the extension supplies `start`, `stop`, and `status`.
The optional extension also installs `gnoblin.spatial` through Gnoblin's Lua
extension installer hook. Its `anchor`, `place`, `attach`, `move`, `raise`,
and `update_pose` methods validate extension-owned scene arguments and submit
asynchronous `vr.spatial.*` actions from the supervised Lua runtime. The
`surface` and `snapshot` methods are likewise typed extension actions.

The compositor keeps a spatial scene store behind these calls. Anchors, free and viewport-docked surface placements, stacking ranks and pose updates are validated and applied there, and `snapshot` and `surface` return the stored records with a scene revision. A headless test covers the store. It does not yet import window textures, project quad layers, hit test controller rays, or route pointer and focus events; those consume the store in later phases.

The stock output list remains owned by Mutter's monitor manager. This
extension package does not add an XR record to that list; Mutter currently has
no external driver registration interface that this process can use. The
headset-as-output integration therefore remains a separate extension/driver
boundary to implement before `gnoblinctl output list` can show an XR output.
The extension Lua helper supplies client lifecycle and status only. It does
not publish an XR output record into the compositor snapshot.

Build and test the extension independently with CMake and CTest from this
directory. The build requires GLib/GIO and the OpenXR loader development files.
