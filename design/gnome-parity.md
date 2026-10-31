# GNOME session parity gaps

This compares a stock GNOME 51 Wayland session (the Mutter and GNOME Shell versions pinned in `src/gnome-versions.json`) with Gnoblin plus the Bingux shell. “Gap” means a user-visible behavior or control surface that is missing or less integrated; it does not mean the underlying Linux service or Wayland protocol is absent.

Effort estimates are for a small, reviewable first implementation, not full GNOME feature completeness.

Gnoblin is the compositor and session integration layer. Bingux owns shell presentation and interaction. System daemons such as IBus, UPower, power-profiles-daemon, GNOME Online Accounts, and colord remain separate dependencies. The Mutter hook column names existing public mechanisms where known; where GNOME relies on private Shell-to-Mutter APIs or a system service, the row says so instead of inventing a protocol.

## User-visible gaps

### Unresponsive apps

- **Current Gnoblin + Bingux state and gap:** Covered. Gnoblin pings clients (`window_management.check_alive_timeout`), reports `unresponsive` on window records with a `gnoblin.window.changed` event, and offers `window.kill` (`gnoblinctl window kill`, `Window:kill()`). Bingux shows a Not Responding dialog with Wait and Force Quit. Remaining parity: GNOME also dims and groups the frozen window and offers the choice when the user clicks its close button, which Bingux does not.
- **Owner:** Bingux UI; Gnoblin detection and safe action
- **User impact:** A hung app can be ended from the desktop without a terminal.
- **First useful slice:** Done; S for the dimmed window and close-button prompt.
- **Mutter hook or protocol:** Mutter `MetaWindow` `is-alive` and `meta_window_kill`. No Wayland protocol reports this to arbitrary clients.

### Alt+Tab and app switcher

- **Current Gnoblin + Bingux state and gap:** Bingux has a window-based chooser with previews, reverse selection, cancellation, and a compositor fallback if its UI is unavailable. GNOME also offers application grouping and a Shell-integrated switcher/overview path.
- **Owner:** Bingux
- **User impact:** Switching works, but app grouping and GNOME’s presentation and keyboard conventions differ.
- **First useful slice:** S
- **Mutter hook or protocol:** Mutter keybindings and window/workspace enumeration through the existing Gnoblin bridge; Bingux’s persistent shortcut session. No standard switcher protocol.

### Window menu

- **Current Gnoblin + Bingux state and gap:** This is substantially covered: Gnoblin forwards native window-menu requests and Bingux implements a menu whose entries follow GNOME Shell, including move/resize and workspace actions. Remaining parity is validation of all entry paths and edge cases, not a missing feature.
- **Owner:** Bingux presentation; Gnoblin request and authorized window operations
- **User impact:** Inconsistent invocation or unavailable actions would make window management harder to discover.
- **First useful slice:** S validation
- **Mutter hook or protocol:** Mutter `show_window_menu` plugin hook; Gnoblin `gnoblin.window.menu-requested` bridge event and one-use menu context. xdg-shell has no standard compositor window-menu protocol.

### Workspaces overview

- **Current Gnoblin + Bingux state and gap:** Gnoblin has native workspaces, Lua actions and touchpad workspace swipes. There is no full-screen visual overview for browsing, moving, and creating workspaces and windows.
- **Owner:** Bingux
- **User impact:** Spatial navigation and cross-workspace window organization are less discoverable.
- **First useful slice:** L
- **Mutter hook or protocol:** Mutter workspace/window snapshots and activation/move operations through the bridge; Clutter stage/layer surfaces for the overview. GNOME Shell’s overview is not a Wayland protocol.

### Notifications and Do Not Disturb

- **Current Gnoblin + Bingux state and gap:** Bingux displays and retains notifications, groups cards, exposes history, and has a DND toggle that suppresses banners through the standard GNOME notifications setting. The remaining gap is fuller policy parity, such as scheduled DND and clear rules for which notifications remain visible.
- **Owner:** Bingux
- **User impact:** Users can silence banners now, but have fewer automatic and fine-grained interruption controls.
- **First useful slice:** S
- **Mutter hook or protocol:** Freedesktop Notifications D-Bus service (`org.freedesktop.Notifications`) and `org.gnome.desktop.notifications` GSettings; Bingux owns presentation and policy. There is no Mutter notification protocol.

### Screenshot and screen recording UI

- **Current Gnoblin + Bingux state and gap:** Bingux Capture provides region/window/screen modes, screenshots, recording, audio options, and recording status. This is substantially covered; gaps are GNOME’s native portal picker integration and polished system-level status/permission continuity where the portal/backend differs.
- **Owner:** Bingux UI; Gnoblin portal/backend
- **User impact:** Users can capture, but may encounter a different picker and permission flow in apps expecting GNOME portal behavior.
- **First useful slice:** M
- **Mutter hook or protocol:** `org.freedesktop.portal.Screenshot` and `org.freedesktop.portal.ScreenCast`; Mutter ScreenCast D-Bus API / PipeWire streams. `wlr_screencopy` is a separate compositor capture path, not a substitute for portal consent.

### Accessibility

- **Current Gnoblin + Bingux state and gap:** Bingux has keyboard navigation, accessible names, contrast-conscious controls, and reduced-motion behavior. The session does not provide a coherent Settings surface for Orca/screen-reader activation, high contrast, large text, magnification, and keyboard accessibility. Gnoblin already has configurable keyboard accessibility settings, but no parity UI.
- **Owner:** Bingux settings; Gnoblin exposes compositor settings and state
- **User impact:** Users who need assistive features must assemble settings across tools or cannot activate them easily.
- **First useful slice:** M
- **Mutter hook or protocol:** AT-SPI D-Bus and the `org.gnome.desktop.a11y` GSettings keys for assistive settings; Mutter input settings for sticky/bounce/slow keys and magnification integration. Do not treat a shell-only visual theme as screen-reader support.

### Input methods and on-screen keyboard

- **Current Gnoblin + Bingux state and gap:** Gnoblin includes IBus integration for text-input clients; Bingux has text-entry anchoring support. No general-purpose on-screen keyboard comparable to GNOME’s accessibility/touch keyboard is evident.
- **Owner:** Bingux UI; Gnoblin text-input and input-method plumbing
- **User impact:** Touch-only users and users unable to use a physical keyboard lack a dependable session keyboard.
- **First useful slice:** M
- **Mutter hook or protocol:** IBus D-Bus API; Wayland `text-input-v3` and `input-method-v2` where supported. On-screen keyboard is a shell client and must coordinate with text-input focus.

### Power profiles and battery

- **Current Gnoblin + Bingux state and gap:** Bingux already shows battery state and exposes available power profiles through its control centre. The gap is broader power-settings parity: detailed battery behavior and service capabilities beyond the profile picker are not established.
- **Owner:** Bingux UI; UPower and power-profiles-daemon own state and policy
- **User impact:** Users can see charge and choose an available profile, but have fewer battery and power controls in one place.
- **First useful slice:** S
- **Mutter hook or protocol:** UPower D-Bus for battery state; `net.hadess.PowerProfiles` D-Bus API for profile enumeration and selection. Mutter is not the policy owner.

### Night Light

- **Current Gnoblin + Bingux state and gap:** Bingux exposes a Night Light toggle backed by `org.gnome.settings-daemon.plugins.color` and reports scheduled/active state. Missing parity is richer schedule and color-temperature setup and reliable behavior where the settings daemon or Mutter color pipeline is unavailable.
- **Owner:** Bingux settings; GNOME color service and Gnoblin output pipeline
- **User impact:** Users can toggle the feature, but may not be able to tune its schedule or temperature from Bingux.
- **First useful slice:** S/M
- **Mutter hook or protocol:** GSettings schema `org.gnome.settings-daemon.plugins.color`; Mutter color-management and monitor color state. `zwlr_gamma_control_v1` alone is not equivalent to color-managed Night Light.

### Lock screen

- **Current Gnoblin + Bingux state and gap:** Bingux has an `ext-session-lock-v1` client. Gnoblin supplies the compositor-owned lock boundary; the remaining parity gap is a fully integrated idle, suspend, and lock-before-sleep policy plus a GNOME-like lock-screen shell experience.
- **Owner:** Gnoblin security/state; Bingux lock UI; session policy service
- **User impact:** Locking can be invoked, but automatic lock timing and suspend coordination are less turnkey.
- **First useful slice:** M
- **Mutter hook or protocol:** `ext-session-lock-v1`, `ext-idle-notify-v1`, logind `Lock`/`PrepareForSleep`. Gnoblin must enforce lock state and fail closed; Bingux authenticates and renders.

### Online accounts

- **Current Gnoblin + Bingux state and gap:** `docs/gnome-apps.md` says account support needs an account manager and GVfs backend; Gnoblin does not start GNOME Shell components. There is no integrated accounts setup/control surface in Bingux.
- **Owner:** Bingux integration/settings; GNOME Online Accounts and GVfs services
- **User impact:** Cloud account setup and account-backed file access require separately installed/configured GNOME components.
- **First useful slice:** M
- **Mutter hook or protocol:** GNOME Online Accounts D-Bus API and GVfs backends. No Mutter hook; launch the supported account manager and report missing services clearly.

### Color management

- **Current Gnoblin + Bingux state and gap:** Mutter 51 has Wayland color-management support, but Gnoblin/Bingux do not offer a user-facing display/profile workflow comparable to GNOME Settings. Protocol presence alone does not configure ICC profiles.
- **Owner:** Bingux settings; Gnoblin output pipeline
- **User impact:** Users cannot easily assign display profiles or inspect the active color state.
- **First useful slice:** L
- **Mutter hook or protocol:** `color-management-v1` Wayland protocol and Mutter’s color-management implementation; colord D-Bus for profile/device management. Keep policy in the shell/service and application in the compositor.

### Fractional scaling

- **Current Gnoblin + Bingux state and gap:** Gnoblin inherits Mutter scale/output support and fractional-scale protocol plumbing. The gap is discoverable per-monitor setup and clear handling of clients that do not scale sharply.
- **Owner:** Bingux settings; Gnoblin output configuration
- **User impact:** High-density displays may require a trade-off between usable size and sharpness, with no single obvious control surface.
- **First useful slice:** M
- **Mutter hook or protocol:** Mutter monitor configuration (`org.gnome.Mutter.DisplayConfig` D-Bus) and `wp_fractional_scale_manager_v1` plus `wp_viewport`; verify the exact advertised globals at runtime.

### Multi-monitor

- **Current Gnoblin + Bingux state and gap:** Gnoblin has output discovery/configuration hooks and stable monitor identity; Bingux provides per-monitor shell surfaces. A GNOME-like graphical display arrangement page, mirror/primary controls, and recovery from invalid layouts are not established as one workflow.
- **Owner:** Gnoblin output model and validation; Bingux UI
- **User impact:** Users have fewer safe controls for arranging displays, choosing scale/refresh, and recovering from bad settings.
- **First useful slice:** M
- **Mutter hook or protocol:** Mutter `org.gnome.Mutter.DisplayConfig` D-Bus (`GetCurrentState`, `ApplyMonitorsConfig`) and Mutter monitor manager. `zwlr_output_manager_v1` is explicitly not advertised by Gnoblin.

### Touch and gestures

- **Current Gnoblin + Bingux state and gap:** Gnoblin has a Lua input policy and configured touchpad workspace swipes. Bingux is primarily keyboard/pointer oriented; GNOME’s overview and shell gestures have no equivalent end-to-end touch flow.
- **Owner:** Gnoblin input routing; Bingux gesture presentation
- **User impact:** Touch users cannot rely on the same edge/swipe navigation and gesture feedback.
- **First useful slice:** M
- **Mutter hook or protocol:** Mutter/Clutter touch and gesture events routed through Gnoblin’s input policy; `zwp_pointer_gestures_v1` is client-facing and does not replace compositor gesture recognition.

### Drag and drop

- **Current Gnoblin + Bingux state and gap:** Wayland clients can use core data-device drag-and-drop, but the shell lacks GNOME’s overview/workspace drop targets and consistent dock-to-window workflows.
- **Owner:** Bingux
- **User impact:** Moving files between apps works where clients support it, while arranging windows and moving them between workspaces by dragging is less discoverable.
- **First useful slice:** M
- **Mutter hook or protocol:** Wayland `wl_data_device` / `wl_data_device_manager` for client transfers; Mutter window/workspace operations through the bridge for shell-level window drops.

### Clipboard

- **Current Gnoblin + Bingux state and gap:** Gnoblin advertises `ext_data_control` for clipboard managers and Bingux has a guarded paste helper. A persistent clipboard history, preview, and privacy controls comparable to an integrated desktop workflow are missing.
- **Owner:** Bingux UI/service; Gnoblin protocol and security policy
- **User impact:** Users cannot recover earlier clipboard items or easily see what is being retained.
- **First useful slice:** M
- **Mutter hook or protocol:** `ext-data-control-v1` for manager access; Wayland core selection for ordinary clients. Clipboard history belongs in a separate shell service with explicit persistence and sensitive-content policy.

### TTY switching

- **Current Gnoblin + Bingux state and gap:** This is not a clear parity gap: Mutter’s native backend and the Linux seat/logind handle VT switching; Gnoblin does not need a Shell UI for Ctrl+Alt+Fn. It still needs validation on supported seats and GPUs.
- **Owner:** Gnoblin/platform integration
- **User impact:** If the shortcut fails, users can lose the standard recovery path to another console.
- **First useful slice:** S validation; M if a backend defect is found
- **Mutter hook or protocol:** Mutter native backend, libseat/logind and kernel VT switch handling. No Wayland protocol or Bingux surface.

### Crash recovery

- **Current Gnoblin + Bingux state and gap:** Gnoblin has startup/config recovery work and Bingux clients reconnect to compositor services. There is no established GNOME-session-style recovery path that restarts a crashed shell and restores its panels without losing the desktop session; compositor failure itself normally ends the Wayland session.
- **Owner:** Bingux shell supervisor/restart; Gnoblin session lifecycle and compositor failure reporting
- **User impact:** A shell crash can remove desktop controls until manually restarted; a compositor crash can terminate all clients.
- **First useful slice:** M
- **Mutter hook or protocol:** Mutter `MetaPlugin`/display lifecycle and Gnoblin’s session supervisor/bridge reconnect events. No standard Wayland crash-recovery protocol; distinguish recoverable shell restart from compositor restart.

## Evidence and boundaries

Repository evidence includes `docs/gnome-apps.md`, `docs/wayland-protocols.md`, `docs/shell-integration.md`, `design/gnome-shell-retirement.md`, `design/lua-api.md`, and `design/lua-input-policy.md`. It also includes Gnoblin’s default input and window configuration, plus Bingux’s `docs/window-switcher.md`, `docs/capture.md`, `docs/locking.md`, `docs/desktop-shell.md`, `shell/bingux/README.md`, `shell/bingux/WindowMenu.qml`, `shell/bingux/ControlCentreOverview.qml`, and `shell/bingux/control-centre-services.py`.

The review describes source/documentation capability; it does not claim fresh-session runtime verification for every row.

The upstream comparison is GNOME 51 because `src/gnome-versions.json` pins Mutter 51.0. Relevant primary sources:

- [GNOME Shell 51.0 UI source](https://gitlab.gnome.org/GNOME/gnome-shell/-/tree/51.0/js/ui), including `altTab.js`, `overview.js`, `windowManager.js`, `notificationDaemon.js`, `screenshot.js`, and `screencast.js`.
- [Mutter 51.0 source](https://gitlab.gnome.org/GNOME/mutter/-/tree/51.0/src), including `core/`, `wayland/`, monitor configuration, window ping handling, input, and color management.
- [GNOME Shell 51.0](https://gitlab.gnome.org/GNOME/gnome-shell/-/tree/51.0) and [Mutter 51.0](https://gitlab.gnome.org/GNOME/mutter/-/tree/51.0) are the pinned upstream revision lines; Gnoblin carries a patched Mutter build, so upstream behavior is a reference rather than proof of a Gnoblin runtime feature.
- [Wayland protocols](https://gitlab.freedesktop.org/wayland/wayland-protocols), including `ext-session-lock-v1`, `ext-idle-notify-v1`, `ext-data-control-v1`, `color-management-v1`, `fractional-scale-v1`, `text-input-v3`, and `input-method-v2`.
- [XDG Desktop Portal ScreenCast](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html) and [Screenshot](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Screenshot.html).
- [Mutter DisplayConfig D-Bus API](https://gitlab.gnome.org/GNOME/mutter/-/blob/51.0/data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml), [GNOME Online Accounts](https://gnome.pages.gitlab.gnome.org/gnome-online-accounts/), [UPower](https://upower.freedesktop.org/docs/), and [power-profiles-daemon](https://gitlab.freedesktop.org/upower/power-profiles-daemon/-/blob/main/dbus/org.freedesktop.UPower.PowerProfiles.xml).

The table separates confirmed repository facilities from parity judgments. Effort is relative: S is a focused UI or wiring change, M crosses shell and compositor/service boundaries, and L needs a substantial interaction model or color/display pipeline work. The “hook” identifies an integration point, not a claim that the full feature is already implemented.

## Prioritized small-PR backlog

1. **DND policy completion (S, Bingux).** Keep the existing toggle and history behavior; add clear scheduling or automatic rules, and document which notifications remain visible.
2. **Window menu parity check (S, Bingux).** Verify titlebar and keyboard invocation, action availability, and stale-window handling against the Gnoblin menu contract; fix only observed gaps.
3. **Alt+Tab parity decisions (S, Bingux).** Decide and document app grouping versus per-window order, then implement one behavior behind the existing switcher rather than replacing its compositor fallback.
4. **Unresponsive-window event and prompt (M, Gnoblin + Bingux).** Bridge Mutter ping timeout as a typed event; add wait/force-quit UI and keep the destructive action explicit.
5. **Lock-before-suspend lifecycle (M, Gnoblin + Bingux).** Add a session policy service that waits for compositor-confirmed lock before suspend and reports failure instead of implying the screen is locked.
6. **Battery and power-profile controls (S/M, Bingux).** Connect UPower and power-profiles-daemon state to the existing power surface; show unavailable states when the service is absent.
7. **Display configuration starter (M, Gnoblin + Bingux).** Add a read/preview/apply/revert flow for monitor arrangement, scale and refresh using Mutter DisplayConfig with rollback on invalid or disconnected layouts.
8. **Accessibility settings starter (M, Bingux).** Provide direct screen-reader launch and controls for large text, contrast and keyboard accessibility, persisting the standard settings and exposing unsupported features clearly.
9. **Clipboard history (M, Bingux).** Build a bounded history service over `ext_data_control_v1` with clear/disable controls and a documented sensitive-data policy.
10. **Touch keyboard and gestures (M, Bingux + Gnoblin).** First deliver a text-input-aware on-screen keyboard, then add tested overview/workspace gestures through Gnoblin’s input routing.
11. **Overview (L, Bingux).** Prototype a keyboard-first overview over existing window/workspace snapshots; add drag-to-workspace only after activation and move operations are reliable.
12. **Night Light and color profile UI (M/L, Bingux + Gnoblin).** Start with a read-only report of active color state and hardware support; add scheduling/profile selection only where the Mutter pipeline provides correct color-managed application.
13. **Shell restart supervision (M, Bingux/session integration).** Restart Bingux after a process crash, reconnect its bridge clients, and report when a compositor crash requires a new session.
14. **TTY switch acceptance (S, Gnoblin/platform).** Record a seat-level regression check for switching away and back on each supported backend; file a focused fix only if Mutter’s native VT path fails.

The smallest high-value sequence is notification-policy completion and window-menu path validation, then clear error/recovery surfaces (hung applications and lock-before-suspend), then power and display settings. Overview, full accessibility settings, and color management are larger projects and should follow stable shell/compositor interfaces.
