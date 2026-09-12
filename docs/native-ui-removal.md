# Native UI removal

Gnoblin is moving desktop controls into the external shell. This change removes
GNOME extensions and the remaining general desktop popups from the Gnoblin
session. Stock GNOME retains its own behaviour.

QuickShell replacements are tracked in
[Bingux #12](https://github.com/kierandrewett/bingux/issues/12). Standard OSDs,
notifications and capture controls already exist in Bingux. The open issues
cover lock-mode OSD transport, tablet controls, interactive capture portal routing, monitor labels,
window menus, workspace feedback, the command launcher, welcome
guidance and wellbeing UI. Add an issue for each additional GUI or OSD removed;
first check the existing Bingux implementation to avoid duplicate work.

The official GNOME Extensions component is not constructed in Gnoblin. Its
management API, app and CLI are removed. GNOME extension compatibility is not
a QuickShell replacement requirement; Gnoblin user scripts remain supported.

The native Run dialog is also gone. `Alt+F2` now opens the compositor's
developer console for JavaScript inspection and live compositor changes;
[Bingux #4](https://github.com/kierandrewett/bingux/issues/4) remains open for
the separate external command launcher.

Gnoblin retains a native desktop right-click menu with Open Terminal. An
independent session component shows a recovery panel after eight seconds
without a visible layer surface. These tools remain available when the external
shell cannot start. They are disabled on the lock screen. See
[desktop recovery](bring-your-own-shell.md).

Implementation checklist:

- [x] Remove extension loading, installation and reload entry points.
- [x] Remove native OSD actors and keep external OSD events.
- [x] Remove screenshot controls, Run dialog, monitor labels and
  workspace popup. Keep workspace operations and non-interactive capture.
- [x] Stop building the extension app, preferences service and command-line tool.
- [x] Verify the private Gnoblin session and stock session isolation.

Verified against the rebuilt private prefix on 2026-09-12: native-chrome,
D-Bus, notification ownership, stock protocol isolation and `gnome-verify`
all passed. The lifecycle probe waits for lock-mode components to settle
before checking widget absence, then waits for the control component to return.
All 36 Shell patches also replayed cleanly against the pinned upstream source.

OSD and screenshot feature ids are accepted as disabled compatibility settings.
They cannot restore removed UI and no longer appear as feature switches.
Notifications and the native keyboard-layout switcher remain opt-in controls.

The stock unlock-dialog mode disables the control component and its OSD
forwarding. [Bingux #8](https://github.com/kierandrewett/bingux/issues/8) tracks
carrying those events to the existing renderer while locked. It does not
require rebuilding the OSD widgets.

Remaining native services need replacements before removal: screen locking,
authentication and keyring prompts, network credentials, mount prompts,
accessibility controls, and portal permission and selection dialogs. The
optional GNOME Settings build remains available for hardware configuration.

Installing a rebuilt runtime requires a new login to replace in-process shell
code. A private headless test does not verify the installed login session.
