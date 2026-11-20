# Verify a desktop installation

For contributors and package maintainers: use this checklist to verify a
release on a real desktop. It covers login, input and visible behaviour that
headless tests cannot prove. Ordinary installation ends with the steps in the
[install guide](installation.md).

## 1. Log in

Use the [package guide](installation.md) or complete
[source session registration](install-source.md#login-session).
Choose **Gnoblin** at the login screen.

Record the build you installed, distribution, GPU, displays and scale factors.

## 2. Check the hardware backend

A login from a display manager has a seat and a GPU and no host compositor, so Gnoblin picks the `drm` backend by
itself. See [backends](backends.md) for the rule.

1. Ask the session which backend it runs on:

    ```sh
    gnoblinctl session list
    gnoblinctl session get ID backend
    ```

    It prints `"drm"`. The compositor's log names the reason: `backend selected ... reason="auto: a login session with a
seat and a DRM node"`. A `"headless"` or `"nested"` here is a finding: record the environment (`WAYLAND_DISPLAY`,
    `XDG_SEAT`, `ls /dev/dri`).

2. Check every display lights, the pointer follows the mouse, and the keyboard and any touchscreen work.
3. Switch to another virtual terminal (Ctrl+Alt+F3) and back. The desktop repaints, no key or button stays held, and
   the session does not freeze.
4. Unplug a monitor and plug it back in. Its output goes and returns, and the windows that were on it move to a
   display that is left.
5. Use night light, or whatever sets a colour temperature, and switch a display off and on. The tint changes and the
   screen blanks: the gamma and power writes have only been tested without hardware.
6. From a terminal inside the session, start a second compositor on the same seat:

    ```sh
    GNOBLIN_BACKEND=drm gnoblin --role compositor
    ```

    It must stop with `the drm backend cannot start`, the step that failed and what to do, and the running session must
    be untouched.

Record the GPU driver and whether the GLES context came up. A GPU that cannot give one is refused by name, and the
`nested` and `headless` backends are the way out on that machine.

## 3. Check your shell

Verify the bar, dock, launcher and notifications appear and respond to input.
Open and close menus near each screen edge. Test every display.

If the shell fails, read `gnoblinctl log --level WARN` for the reason before restarting it.

## 4. Check windows and keys

Test focus, typing, minimise/restore, maximise, fullscreen, drag and resize.
Check app switching, launcher shortcuts and any custom bindings.

For SSD windows, check all buttons, titlebar drag and edge resize.
A correct screenshot alone does not prove input works.

## 5. Reload

Change one visible config setting, then:

```sh
gnoblinctl config reload
```

Confirm the setting changed and existing windows survived.

Then check a device setting, which only a real device can prove:

1. Set `Touchpad.tapToClick = true` in `shell.ts`, reload, and tap the touchpad. It should click.
2. Delete the line, reload, and tap again. It should not click, because tap-to-click is off by default on most
   touchpads.
3. Do the same with `Mouse.sensitivity` or `Mouse.naturalScroll` for a mouse.

Another program's `libinput list-devices` shows its own libinput context, so it cannot confirm this. A device that does
not take a setting is named once in the log (`gnoblinctl log --level WARN`, area `input`). See
[backends](backends.md#input-device-settings).

## 6. Lock and unlock

Confirm the lock screen receives input, desktop controls are unavailable while
locked, and the shell recovers after unlock.

## 7. Screen sharing and remote input

With the desktop portal backend installed:

1. Start a share and complete the backend's consent dialog.
2. Confirm the selected monitor is the one actually shared.
3. Test remote input and confirm the backend's device prompts.
4. Stop sharing and confirm the stream ends.

Gnoblin only routes the portal request. The selected backend decides access.

On the hardware backend a screenshot or a screen share has nothing to read: it draws straight to the display and keeps
no frame to read back, so the request is refused. That is expected today and is listed in [backends](backends.md); it
is not a finding.

## 8. Check a mode change

A mode change on a hardware display is off unless the session starts with `GNOBLIN_DRM_MODESET=1`. See
[outputs](shell-api/outputs#what-each-backend-applies).

1. With the variable unset, run `gnoblinctl output call <id> apply mode=<another mode>`. Confirm it is refused with an
   error that names the variable, and that the display keeps its mode.
2. Start the session with `GNOBLIN_DRM_MODESET=1`. On a monitor that lists two modes, switch to the other and back. The
   screen blanks briefly, then shows the desktop at the new size.
3. Ask for a mode the monitor does not list. Confirm it is refused.

Record the GPU driver. A driver that refuses a commit its own test passed is a finding.

## 9. Return to GNOME

::: details Check the GNOME session

Log out and select GNOME. Confirm its desktop still works.

:::

## 10. Remove Gnoblin

For package-release testing, remove Gnoblin using its documented uninstall
path and check that the Gnoblin login entry disappears.

Record failures separately from skipped checks. Include the exact commands,
logs and visible outcome; a successful build is not a successful desktop test.
