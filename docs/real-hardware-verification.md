# Verify a desktop installation

For contributors and package maintainers: use this checklist to verify a
release on a real desktop. It covers login, input and visible behaviour that
headless tests cannot prove. Ordinary installation ends with the steps in the
[install guide](installation.md).

## 1. Log in

Use the [package guide](installation.md) or complete
[source session registration](install-source.md#3-add-a-login-session).
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
gnoblinctl reload
```

Confirm the setting changed and existing windows survived.
The second command also reloads theme and user scripts.

## 5. Lock and unlock

Confirm the lock screen receives input, desktop controls are unavailable while
locked, and the shell recovers after unlock.

## 6. Screen sharing and remote input

With the patched portal backend installed:

1. Under the default policy, start a share and complete the source picker.
2. Confirm the selected monitor is the one actually shared.
3. Test an explicit `ask` rule, then a `deny` rule.
4. For an `allow` rule, test both permitted and broader device requests.
5. Stop sharing and confirm the stream ends.

See [portal permissions](/guides/permissions). Old custom grants and
“remember forever” checkboxes are obsolete; do not use them as acceptance criteria.

## 7. Return to GNOME

Log out and select GNOME. Confirm its desktop still works.
For package-release testing, also remove Gnoblin using its documented uninstall
path and check that the Gnoblin login entry disappears.

Record failures separately from skipped checks. Include the exact commands,
logs and visible outcome; a successful build is not a successful desktop test.
