# GNOME Shell patch retirement audit

This is an internal migration record. It does not describe a supported
GNOME Shell build or compatibility layer.

## Build boundary

The source build prepares and builds Mutter and `xdg-desktop-portal-gnome`.
`scripts/prepare-build-sources.sh` and `scripts/apply-patches.sh` accept only
those projects. The GNOME Shell patch tree is not applied, built, packaged, or
included in the standalone session. Shell patches that remain in the checkout
are migration history, not active runtime behavior.

Package-isolation checks that install stock GNOME are intentional: they prove
that installing Gnoblin does not replace a user's GNOME packages. The old
Shell files removed by `scripts/install-session.sh` are upgrade cleanup for a
private prefix and should remain until that cleanup is no longer needed.

The unbuilt `src/gnome-shell-overlay/shell-gnoblin-shader.{c,h}` helper has
been removed. It was only included by retired Shell shader patches. Shader
file watching now belongs to Gnoblin's native control service, and Mutter
compiles the effect; the old patch references remain historical artifacts.

The unregistered `tests/test-native-chrome.py` probe has been removed. It
started the retired GNOME Shell session and asserted details of `Main`, panel
actors, and Shell D-Bus objects. Those are not part of the standalone session
contract; compositor state is verified through the Lua-backed devkit tests,
and shell presentation belongs to external shell clients.

## Audited groups

| Patch group                                                                                                                                                                                                                                                                              | Current disposition                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `10-tooling`                                                                                                                                                                                                                                                                             | Removed on 2026-10-02. Its patches only relaxed GNOME Shell extension version checks and added a Shell-only disable-extensions option. The standalone session has no GNOME Shell or extension loader.                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `30-portal-policy`                                                                                                                                                                                                                                                                       | Removed on 2026-10-02. Its only patch fixed the GJS `AccessDialog.CloseAsync` callback in GNOME Shell. The standalone portal backend handles Gnoblin permissions in native code and does not register that Shell dialog.                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `39-orientation-lock`                                                                                                                                                                                                                                                                    | Removed on 2026-10-02. Native control and the shared Lua API now expose orientation-lock availability, state, updates, and a setter; shell clients own toggle presentation. The standalone session has no GNOME Shell system-actions UI.                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `35-gnoblin-settings`                                                                                                                                                                                                                                                                    | Removed on 2026-10-02. No standalone component consumes the old Shell schema; current session configuration and compositor settings use the Lua-backed API.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `36-notifications-toggle`                                                                                                                                                                                                                                                                | Removed on 2026-10-02. The standalone session has no GNOME Shell notification daemon; notification presentation belongs to shell clients.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `37-pad-osd-toggle`                                                                                                                                                                                                                                                                      | Removed on 2026-10-02. The standalone session has no GNOME Shell tablet-pad OSD; shell clients own visible OSD presentation.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `38-input-source-switcher`                                                                                                                                                                                                                                                               | Removed on 2026-10-02. Input-source state and selection are available through the Lua-backed native input API; popup presentation belongs to shell clients.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `22-background-terminal`, `50-native-topbar`, `51-startup-animation`                                                                                                                                                                                                                     | Shell presentation and startup animation. The standalone session has no Shell panel or startup actor; clients own desktop UI. The `50-native-topbar` and `51-startup-animation` patches have been removed.                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `45-session-isolation`                                                                                                                                                                                                                                                                   | Shell's session-mode export is superseded by the standalone launcher and environment setup.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `53-layer-animation`, `56-layer-lifecycle`, `57-window-shaders`, `59-window-corners`, `60-permissions`, `66-external-chrome`, `71-window-frames`                                                                                                                                         | Their responsibilities now live in Mutter, Gnoblin's Lua/native APIs, the portal backend, or client-owned UI. Keep compositor patches narrow and validate the native path before deleting corresponding historical patches.                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `65-no-extensions`                                                                                                                                                                                                                                                                       | Removed on 2026-10-02. The standalone session does not launch GNOME Shell or provide its extension manager, so these Shell mode and build patches had no active target.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `54-masked-background-blur`, `61-blur-shadow-mask`, `62-blur-corner-coverage`, `62-native-effect-geometry`, `63-buffer-fades`, `64-mask-uniform-cache`, `67-masked-blur-cache`, `70-blur-detail-coverage`, `72-background-effect`, `74-backdrop-kernel-padding`, `74z-blur-output-scale` | These modify the old Shell blur effect. The standalone renderer uses Mutter's background-effect path and the documented client-owned blur protocol. Exact visual parity still needs matched-scene evidence.                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `75-input-sources`, `76-keyboard-options`                                                                                                                                                                                                                                                | Native input configuration and Lua snapshots cover source selection and XKB options. Verify keyboard initialization and IBus disconnect behavior before retiring the remaining input patches.                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `90-session-lock`                                                                                                                                                                                                                                                                        | Removed on 2026-10-02. The patch only integrates Gnoblin lock requests into GNOME Shell's GJS system actions and screenshot path. The standalone contract is `gnoblin.session.lock()` plus compositor lock-state events for external shell clients. Real-output lock, owner-death, takeover, and real-seat lifecycle verification remain open.                                                                                                                                                                                                                                                                                                                     |
| `91-notification-bridge`                                                                                                                                                                                                                                                                 | Removed on 2026-10-02. It only registered a GNOME Shell `MessageTray` bridge. Gnoblin has no notification UI or compositor notification API; independent shell clients own notification presentation through the standard desktop notification interface.                                                                                                                                                                                                                                                                                                                                                                                                          |
| `92-core-services`, `98-bridge-resources`                                                                                                                                                                                                                                                | Removed on 2026-10-02. These disabled patches only registered GJS bridge resources, and no current build consumed them.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `94-animation-engine`, `95-workspace-animation`                                                                                                                                                                                                                                          | Native open/close and normal-window minimize/restore use Mutter plugin hooks. Workspace switches use Mutter's switch and interruption hooks with `workspace-switch` progress. Resize uses Mutter size-change hooks and `resize` progress to interpolate old-to-new buffer geometry; build and visual proof remain outstanding. Dialog dimming and tile-preview actors were Shell presentation. Their event IDs have been removed from the Lua and Mutter animation registries; the old Shell patch is retained as migration history and is not built or applied. Verify workspace and resize transitions visually; keep this audit active until that proof exists. |
| `99z-wallpaper-runtime` and wallpaper-host patches                                                                                                                                                                                                                                       | Wallpaper is owned by a shell client, as documented in `docs/guides/wallpapers.md`; it is outside the compositor Lua API.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

The `78-headless-testing` patch was removed on 2026-10-03. It only added a
GNOME Shell test override for `loginManager.haveSystemd()`; the standalone
session does not read `GNOBLIN_TEST_NO_LOGIND`.

The patch directories `79-shutdown-order`,
`79z-background-manager-shutdown`, `99zzzz-session-mode-startup`,
`99zzzzz-no-session-presence`, `99zzzzzz-logind-actions`,
`99zzzzzzz-session-ready`, `99zzzzzzzz-no-session-dialog`,
`99zzzzzzzzzz-standalone-idle`, `99zzzzzzzzzzz-native-scope`,
`99zzzzzzzzzzzz-native-xkb`, `99zzzzzzzzzzzzz-timezone`,
`99zzzzzzzzzzzzzz-native-slideshow`, and
`99zzzzzzzzzzzzzzzz-wallpaper-host-defaults` still need individual review.
The input-specific unresolved patches are listed below.

## Still needs a decision or stronger evidence

- `58-menu-backdrop-redraw`: establish whether native damage handling covers
  the same redraw lifecycle.
- `60-blur-cache` and `74-0002-share-shell-layer-backdrops`: the native blur
  path does not currently demonstrate cross-surface backdrop sharing. Compare
  frame cost and output before claiming parity or removing the old design.
- `68-dev-console`: the obsolete GJS entrypoint and console stylesheet patches
  were removed on 2026-10-02. `gnoblinctl lua` evaluates Lua locally in the
  terminal process and exposes the typed session API through
  `gnoblin.<area>.<method>`. It accepts a Lua file or one-line interactive
  input, prints table results as JSON, and never sends Lua source to the
  compositor. Shell-owned graphical console presentation remains outside the
  compositor contract.
- `69-desktop-recovery`: the nested devkit test kills the Lua worker, confirms
  the supervisor starts a replacement, and checks the same Mutter process
  remains alive and serves the config API. The recovery UI remains shell-client
  presentation; its behavior still needs verification with an external shell.
- `73-location-indicator`: the native GeoClue agent exposes availability,
  in-use state, and authorization requests to Lua and shell clients. The
  compositor provides data and policy; location-indicator UI belongs to shell
  clients. GeoClue must allow the `gnoblin` agent ID. Packaging that allowlist
  without replacing distro defaults is tracked in `gnoblin-mc6`.
- `77-keymap-initialization`, `78-ibus-disconnect-guard`,
  `zzzzzzz-on-demand-ibus`, and shutdown-order patches: verify native input
  initialization, IBus recovery, and teardown on a real seat.
- `96-touchpad-gestures`: removed on 2026-10-02. The patch only connected
  GNOME Shell swipe trackers for its Overview, app grid, emoji pager, and lock
  screen to the old GJS configuration bridge. The standalone runtime exposes
  `gnoblin.input.gesture` to Lua and routes configured gestures through native
  control; shell clients own their own gesture-driven presentation.
- `80-optional-gnome-qr`: removed on 2026-10-02. It only makes GNOME Shell's
  login-dialog QR rendering optional; Gnoblin does not provide that dialog.
  `81-brightness-follows-backlight` was removed on 2026-10-02. It synchronized
  GNOME Shell's private brightness sliders after external backlight changes;
  Gnoblin has no such UI state. Brightness shortcuts remain command bindings
  such as `brightnessctl`, as described in the user guide.

The `96-workspace-resource` patch was removed on 2026-10-02. It registered
`gnoblinWorkspaces.js`, which no longer exists in the checkout; workspace
operations are part of the native compositor API.

The stock-mode test harness `tests/test-stock-protocol-isolation.sh`, its
`tests/test-shell-security-policy.py` helper, and both extension fixtures were
removed on 2026-10-02. They tested GNOME Shell Eval policy, extension
registration and version checks, notification ownership, and cancellation of
the Shell-owned Access dialog. Gnoblin does not expose GNOME Shell modes or
extension APIs. External shell clients own notification presentation, while
native portal tests retain Access allow, deny, and caller-identity coverage.
The Shell-specific probes were dropped with the retired Shell integration.

Stale JavaScript tests that import removed Shell modules are not a migration
target. The unregistered `tests/gnome-shell-overlay-resources.test.py` was
removed after confirming the standalone source no longer has a Shell resource
manifest or build target. Remove other fixtures only after recording whether
their behavior is implemented, delegated to a shell client, or deliberately
dropped.
