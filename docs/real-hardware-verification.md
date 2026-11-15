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

## 2. Check your shell

Verify the bar, dock, launcher and notifications appear and respond to input.
Open and close menus near each screen edge. Test every display.

If the shell fails, confirm desktop right-click opens a terminal and the recovery
panel appears. Check the shell's log before restarting it.

## 3. Check windows and keys

Test focus, typing, minimise/restore, maximise, fullscreen, drag and resize.
Check app switching, launcher shortcuts and any custom bindings.

For SSD windows, check all buttons, titlebar drag and edge resize.
A correct screenshot alone does not prove input works.

## 4. Reload

Change one visible config setting, then:

```sh
gnoblinctl config reload
```

Confirm the setting changed and existing windows survived.

## 5. Lock and unlock

Confirm the lock screen receives input, desktop controls are unavailable while
locked, and the shell recovers after unlock.

## 6. Screen sharing and remote input

With the desktop portal backend installed:

1. Start a share and complete the backend's consent dialog.
2. Confirm the selected monitor is the one actually shared.
3. Test remote input and confirm the backend's device prompts.
4. Stop sharing and confirm the stream ends.

Gnoblin only routes the portal request. The selected backend decides access.

## 7. Check a mode change

A mode change on a hardware display is off unless the session starts with `GNOBLIN_DRM_MODESET=1`. See
[outputs](shell-api/outputs#what-each-backend-applies).

1. With the variable unset, run `gnoblinctl output call <id> apply mode=<another mode>`. Confirm it is refused with an
   error that names the variable, and that the display keeps its mode.
2. Start the session with `GNOBLIN_DRM_MODESET=1`. On a monitor that lists two modes, switch to the other and back. The
   screen blanks briefly, then shows the desktop at the new size.
3. Ask for a mode the monitor does not list. Confirm it is refused.

Record the GPU driver. A driver that refuses a commit its own test passed is a finding.

## 8. Return to GNOME

::: details Check the GNOME session

Log out and select GNOME. Confirm its desktop still works.

:::

## 9. Remove Gnoblin

For package-release testing, remove Gnoblin using its documented uninstall
path and check that the Gnoblin login entry disappears.

Record failures separately from skipped checks. Include the exact commands,
logs and visible outcome; a successful build is not a successful desktop test.
