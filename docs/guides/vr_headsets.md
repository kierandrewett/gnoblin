# VR Headsets

Gnoblin can show your desktop in a VR headset. VR is an optional extension, `gnoblin-vr`, that you install next to Gnoblin. Plug a headset in and Gnoblin starts the right OpenXR runtime for it.

Two kinds of headset work, and each needs one runtime package:

| Headset                                                        | How it connects                                   | Runtime package   |
| -------------------------------------------------------------- | ------------------------------------------------- | ----------------- |
| Tethered: Valve Index, HTC Vive, Sony PSVR2, Pimax, Windows MR | A display cable (DisplayPort or HDMI) and USB     | Monado (`monado`) |
| Standalone: Meta Quest 1, 2, 3, 3S and Pro, Pico 4, Vive Focus | A USB cable or Wi-Fi, streaming from the computer | WiVRn (`wivrn`)   |

A tethered headset's display connector is leased to the runtime, so it never appears as a monitor on your desktop. A standalone headset runs a small app that receives the picture over the link.

## Install

Install `gnoblin-vr` and the runtime for your headset. For a Quest, Pico or Vive Focus over USB, also install `adb` (the `android-tools` package). On Fedora:

```sh
sudo dnf install gnoblin-vr monado wivrn android-tools
```

If your distribution has no WiVRn package, build the copy pinned in the Gnoblin source tree:

```sh
make vr-streamer INSTALL=1
```

## Turn it on

Enable the headset watcher, then load the default policy from your Lua configuration.

```sh
systemctl --user enable --now gnoblin-vr-watch.service
mkdir -p ~/.config/gnoblin/lua
cp /usr/share/gnoblin-vr/lua/gnoblin-vr*.lua ~/.config/gnoblin/lua/
```

```lua
-- ~/.config/gnoblin/config.lua
require("gnoblin-vr-session").auto()
require("gnoblin-vr-desktop").auto()
```

Reload with `gnoblinctl reload`. To keep the streamer off until a headset is plugged in by USB, pass `{ streamer = "usb" }`. The default `"always"` keeps it running so a headset can find the computer over Wi-Fi.

The desktop policy mirrors every normal window on a two-metre arc at eye height when the headset session enters `running`. It sizes each quad from the window's pixel dimensions. The policy requires a Gnoblin build with the VR spatial scene and input actions.

The optional settings below change the arc, eye height and pixel-to-metre scale. They take effect when the Lua configuration is loaded or reloaded. Defaults and accepted values are:

| Option                                                                    | Type and accepted values       | Default                | Effect                                                                                                    |
| ------------------------------------------------------------------------- | ------------------------------ | ---------------------- | --------------------------------------------------------------------------------------------------------- |
| `pixels_per_metre`                                                        | Positive number                | `1000`                 | Converts the window frame from pixels to metres.                                                          |
| `radius`                                                                  | Positive number in metres      | `2`                    | Distance from the scene origin to the arc.                                                                |
| `eye_height`                                                              | Number in metres               | `1.45`                 | Vertical centre of each window in the scene.                                                              |
| `arc_step`                                                                | Number in radians              | `math.rad(18)`         | Angular gap between adjacent placement slots.                                                             |
| `arc_center`                                                              | Number                         | `0`                    | Arc offset in slot units. Slots fan out from the centre in alternating directions.                        |
| `minimum_size`                                                            | Positive number in metres      | `0.2`                  | Smallest width or height during corner resizing and two-hand scaling.                                     |
| `resize_debounce`                                                         | Non-negative number in seconds | `0.25`                 | Delay before resize updates are sent while a corner is held; release applies the final size immediately.  |
| `environment`                                                             | `"opaque"` or `"passthrough"`  | `"opaque"`             | Initial headset background mode. The left menu button toggles the mode.                                   |
| `minimum_workspace_scale`                                                 | Positive number                | `0.25`                 | Lower limit for two-hand workspace scaling.                                                               |
| `maximum_workspace_scale`                                                 | Positive number                | `4`                    | Upper limit for two-hand workspace scaling.                                                               |
| `place`, `remove`, `reconcile`, `input`, `_point`, `_start_grip`, `_drag` | Lua function                   | Built-in policy method | Replaces the matching method. Use `require("gnoblin-vr-desktop").new(opts)` to manage callbacks directly. |

```lua
require("gnoblin-vr-desktop").auto({
    pixels_per_metre = 1000,
    radius = 2,
    eye_height = 1.45,
    arc_step = math.rad(18),
})
```

## Use the desktop in VR

| Action                         | Controller input                                                                                                   |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| Point at a window              | Aim either active controller at it; the pointer follows the ray.                                                   |
| Click                          | Press or release the trigger. It sends a left-button click.                                                        |
| Right-click                    | Press or release the right controller's menu button.                                                               |
| Scroll                         | Use the controller thumbstick; its scroll delta goes to the pointed window.                                        |
| Move a window                  | Grip a window and move the controller. The window follows and faces you.                                           |
| Resize a window                | Grip within 12% of a window corner and move. The opposite corner stays fixed; release applies the new window size. |
| Move the workspace             | Grip with the ray over empty space. Translate or turn the controller to move or rotate all windows together.       |
| Scale and rotate the workspace | Grip with both controllers and move them apart/together or around each other.                                      |
| Toggle passthrough             | Press the left controller's menu button. Press again to return to the opaque environment.                          |

The desktop policy is ordinary Lua and can be replaced. `auto(opts)` accepts `pixels_per_metre`, `radius`, `eye_height`, `arc_step`, `arc_center`, `minimum_size`, `resize_debounce`, `environment`, workspace scale limits, and replacement functions for `place`, `remove`, `reconcile`, `input`, `_point`, `_start_grip`, and `_drag`.

## Connect a headset

A tethered headset comes up when you plug in both cables. Nothing else is needed.

A standalone headset needs one-time setup in the headset:

1. Install the WiVRn app on the headset from its store, or install the APK with `adb install`.
2. For USB, turn on developer mode for the headset in its phone app, plug the cable in, and accept the USB debugging prompt inside the headset.
3. For Wi-Fi, start the WiVRn app. Gnoblin shows a pairing PIN in the `wivrn-server` log; `journalctl --user -u gnoblin-vr-runtime@wivrn` prints it.

After that, plugging the cable in starts the stream, and the WiVRn app finds the computer on its own over Wi-Fi.

## Check what Gnoblin sees

```sh
gnoblinctl output list
```

Each headset is an output with `kind = "xr"`. Its `state` says what to do next:

| State       | Meaning                                                                                |
| ----------- | -------------------------------------------------------------------------------------- |
| `waiting`   | Tethered USB is present but no display connector is leasable. Check the display cable. |
| `detected`  | A standalone headset is on USB and has no stream yet. The `diagnostic` field says why. |
| `ready`     | A tethered display can be opened.                                                      |
| `connected` | A standalone headset is streaming to the computer.                                     |
| `running`   | The OpenXR session is active and drawing to the headset.                               |

`gnoblin-vr --devices` prints the same list as the watcher sees it, and `gnoblin-vr --connect` retries the USB link by hand.
