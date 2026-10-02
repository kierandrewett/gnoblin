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

## Audited groups

| Patch group                                                                                                                                                                                                                                                                              | Current disposition                                                                                                                                                                                                                                                                                                      |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `10-tooling`                                                                                                                                                                                                                                                                             | Removed on 2026-10-02. Its patches only relaxed GNOME Shell extension version checks and added a Shell-only disable-extensions option. The standalone session has no GNOME Shell or extension loader.                                                                                                                    |
| `30-portal-policy`, `35-gnoblin-settings`, `36-notifications-toggle`, `37-pad-osd-toggle`, `38-input-source-switcher`, `39-orientation-lock`                                                                                                                                             | Shell-only tools, schemas, and UI hooks. The standalone configuration and compositor APIs own settings and input behavior; shell presentation belongs to separate clients. Retire these patches after confirming no supported external consumer depends on the old schema.                                               |
| `22-background-terminal`, `50-native-topbar`, `51-startup-animation`                                                                                                                                                                                                                     | Shell presentation and startup animation. The standalone session has no Shell panel or startup actor; clients own desktop UI. The `50-native-topbar` and `51-startup-animation` patches have been removed.                                                                                                               |
| `45-session-isolation`                                                                                                                                                                                                                                                                   | Shell's session-mode export is superseded by the standalone launcher and environment setup.                                                                                                                                                                                                                              |
| `53-layer-animation`, `56-layer-lifecycle`, `57-window-shaders`, `59-window-corners`, `60-permissions`, `65-no-extensions`, `66-external-chrome`, `71-window-frames`                                                                                                                     | Their responsibilities now live in Mutter, Gnoblin's Lua/native APIs, the portal backend, or client-owned UI. Keep compositor patches narrow and validate the native path before deleting corresponding historical patches.                                                                                              |
| `54-masked-background-blur`, `61-blur-shadow-mask`, `62-blur-corner-coverage`, `62-native-effect-geometry`, `63-buffer-fades`, `64-mask-uniform-cache`, `67-masked-blur-cache`, `70-blur-detail-coverage`, `72-background-effect`, `74-backdrop-kernel-padding`, `74z-blur-output-scale` | These modify the old Shell blur effect. The standalone renderer uses Mutter's background-effect path and the documented client-owned blur protocol. Exact visual parity still needs matched-scene evidence.                                                                                                              |
| `75-input-sources`, `76-keyboard-options`                                                                                                                                                                                                                                                | Native input configuration and Lua snapshots cover source selection and XKB options. Verify keyboard initialization and IBus disconnect behavior before retiring the remaining input patches.                                                                                                                            |
| `90-session-lock`                                                                                                                                                                                                                                                                        | The nested lifecycle runner now uses the supervised Gnoblin Lua runtime. It confirms protocol advertisement, then reports `SKIP` when lock-aware capture stops the devkit's virtual output from presenting later lock frames. Real-output lock, owner-death, takeover, and real-seat lifecycle verification remain open. |
| `91-notification-bridge`                                                                                                                                                                                                                                                                 | No Gnoblin notification UI is part of the compositor API; notification presentation belongs to shell clients. Treat this as a deliberate removal unless a supported client demonstrates a missing compositor service.                                                                                                    |
| `92-core-services`, `98-bridge-resources`                                                                                                                                                                                                                                                | Removed on 2026-10-02. These disabled patches only registered GJS bridge resources, and no current build consumed them.                                                                                                                                                                                                  |
| `94-animation-engine`, `95-workspace-animation`, `96-touchpad-gestures`, `96-workspace-resource`                                                                                                                                                                                         | Native Lua animation declarations, compositor animation controls, workspace operations, and input events exist. Verify the old event and gesture behavior against those public APIs before deleting these historical patches.                                                                                            |
| `99z-wallpaper-runtime` and wallpaper-host patches                                                                                                                                                                                                                                       | Wallpaper is owned by a shell client, as documented in `docs/guides/wallpapers.md`; it is outside the compositor Lua API.                                                                                                                                                                                                |

The patch directories `78-headless-testing`, `79-shutdown-order`,
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
- `68-dev-console`: the local `gnoblinctl lua` console now evaluates Lua in the
  terminal process and exposes the typed session API through `gnoblin.<area>.<method>`.
  It accepts a Lua file or one-line interactive input, prints table results as
  JSON, and never sends Lua source to the compositor. Shell-owned graphical
  console presentation remains outside the compositor contract.
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
- `80-optional-gnome-qr` and `81-brightness-follows-backlight`: confirm whether
  these are GNOME app features outside Gnoblin's session contract or still
  require a supported replacement.

Stale JavaScript tests that import removed Shell modules are not a migration
target. The unregistered `tests/gnome-shell-overlay-resources.test.py` was
removed after confirming the standalone source no longer has a Shell resource
manifest or build target. Remove other fixtures only after recording whether
their behavior is implemented, delegated to a shell client, or deliberately
dropped.
