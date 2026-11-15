# Session settings

Layer-shell focus, the window drag boundary and the Wayland protocol list are compositor settings that `init.ts`
cannot set.

What each one controls, and where it is documented:

| Setting                                    | What it controls                                                                                  | Reference                                                                    |
| ------------------------------------------ | ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `layerShell.preserveActiveWindow`          | Whether the window under an exclusive layer, such as a launcher, keeps its active appearance.     | [Dropped settings](/js-settings#dropped-settings)                            |
| `windowManagement.constrainDragToWorkArea` | How far a dragged window may leave the work area. The compositor accepts it and does not read it. | [Window management](/config/configure/window_management#keys-the-shell-owns) |
| `protocols.<name>`                         | Which Wayland interfaces the compositor advertises, such as `wlrScreencopy`.                      | [Dropped settings](/js-settings#dropped-settings)                            |
| `GNOBLIN_DRM_MODESET=1` (environment)      | Whether a mode change on a hardware display is allowed. It is off unless the session sets it.     | [What each backend applies](/shell-api/outputs#what-each-backend-applies)    |

The compositor's defaults stay in force: exclusive layers keep the application beneath them active, a drag stays in the
work area, every protocol is advertised, and a hardware display keeps its mode.

The last one is an environment variable and not a setting. A config file or a script cannot allow a mode change. See the [protocol catalog](/wayland-protocols) for the interface names and
the related guides.

Disabling a protocol does not block all screen sharing, because portal backends decide what apps may do; see
[portals](/config/configure/portals).
