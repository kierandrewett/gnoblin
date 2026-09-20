# Native and external UI

Gnoblin keeps GNOME's window management and core desktop services.
An external shell owns the bar, dock, launcher and desktop popups.

## Provided by your shell

- Bar, dock and application launcher
- OSDs and workspace feedback
- Capture controls
- Window-management menu
- Notifications, unless GNOME's service is explicitly enabled

See [choose a shell](bring-your-own-shell.md) and [native feature settings](session-settings.md#native-features).

## Kept in Gnoblin

- Screen locking and authentication
- Keyring, network and mount prompts
- Accessibility services
- Portal permission and selection dialogs
- Desktop right-click recovery menu
- Recovery panel when no shell surface is visible

## Removed from this session

GNOME's Overview, dash, app grid, extension loader and extension management UI
are unavailable. User scripts remain supported.

Alt+F2 opens the [developer console](developer-console.md), not the old Run dialog.
Legacy OSD/screenshot config keys cannot restore removed widgets.

Stock GNOME retains its normal behavior. Test both modes when changing shared
code; see [testing](testing.md).
