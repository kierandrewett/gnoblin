# Touchpad settings

Set only the touchpad behaviour you want to override. Unspecified fields keep their current system values.

These settings are accepted and checked, and no device reads them: the compositor logs `not applied: ...` for the
`input.touchpad` group. This example enables tap-to-click and two-finger scrolling:

```ts
import { defineConfig } from "gnoblin";

export default defineConfig({
    input: {
        touchpad: {
            tapToClick: true,
            middleClickEmulation: true,
            twoFingerScrollingEnabled: true,
            disableWhileTypingTimeout: 500,
        },
    },
});
```

## Pointer behaviour

| Field           | Accepted values                                    | What it changes                                                                                                      |
| --------------- | -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `speed`         | Number from `-1` to `1`                            | `-1` is unaccelerated, `1` is fast, and `0` uses the system default.                                                 |
| `scrollSpeed`   | Number from `0` to `2`                             | Scales two-finger and touchpad scrolling. `1` is the default speed, `0.5` is half speed, and `2` is twice the speed. |
| `accelProfile`  | `"default"`, `"flat"`, `"adaptive"`, or `"custom"` | Selects the system curve or a custom profile.                                                                        |
| `accelCurve`    | `{ step: number, points: number[] }`               | Defines custom pointer acceleration. Needs `accelProfile: "custom"`; it affects pointer motion, not scrolling.       |
| `leftHanded`    | `"right"`, `"left"`, or `"follow-mouse"`           | Selects the touchpad button order; `"follow-mouse"` uses the mouse setting.                                          |
| `naturalScroll` | Boolean                                            | Reverses the scroll direction.                                                                                       |

## Gestures

| Field                       | Accepted values | Meaning                                                          |
| --------------------------- | --------------- | ---------------------------------------------------------------- |
| `tapToClick`                | Boolean         | Click by tapping the touchpad.                                   |
| `tapAndDrag`                | Boolean         | Drag by tapping, then moving a finger.                           |
| `tapAndDragLock`            | Boolean         | Keep a tap-and-drag active briefly after lifting your finger.    |
| `disableWhileTyping`        | Boolean         | Helps prevent pointer movement from a resting palm while typing. |
| `disableWhileTypingTimeout` | 100–5000 ms     | Wait this long after typing before enabling the touchpad again.  |
| `edgeScrollingEnabled`      | Boolean         | Scroll along the touchpad edge, if supported.                    |
| `twoFingerScrollingEnabled` | Boolean         | Scroll by moving two fingers, if supported.                      |

Set `disableWhileTypingTimeout` to an integer from 100 to 5000 milliseconds. Omit it or set it to `"inherit"` to keep
the system value when available. An explicit value works even when the installed settings schema does not define this
key. The touchpad must support disable-while-typing, and the timeout takes effect only while that behaviour is
enabled.

## Click mapping

| Field                  | Accepted values                                              | Meaning                                                                                                              |
| ---------------------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------- |
| `tapButtonMap`         | `"default"`, `"left-right-middle"`, or `"left-middle-right"` | `"left-right-middle"` maps taps to left, right, middle. `"left-middle-right"` swaps two- and three-finger taps.      |
| `clickMethod`          | `"default"`, `"none"`, `"areas"`, or `"fingers"`             | Keep hardware behaviour, disable software-emulated clicks, or emulate clicks using click areas or finger counts.     |
| `middleClickEmulation` | Boolean                                                      | `true` enables middle-click when both buttons are pressed; omitted uses the current device preference, normally off. |

Each field defaults to the current device preference when omitted. The `"default"` value asks libinput to
choose the device behaviour. Every field also accepts `"inherit"` to say that you want the system preference.

A config is a fixed description, so it cannot change the scroll speed for one application's windows. `scrollSpeed`
applies to the whole touchpad.

For `accelCurve`, use a positive `step` and at least two finite, non-negative `points`. The points define output speed
at evenly spaced input speeds in device units per millisecond. libinput interpolates between points and extrapolates
beyond the last one. Its implementation sets the maximum number of points. When the custom curve is active, libinput
ignores `speed` for pointer behaviour.

The available gestures depend on the touchpad hardware. GNOME's
[touchpad guide](https://help.gnome.org/gnome-help/mouse-touchpad-click.html) explains tap, click, and scroll
behaviour. The [libinput acceleration guide](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html)
explains the curve units and behaviour.
