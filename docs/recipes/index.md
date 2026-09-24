# Recipes

Short Lua examples for common desktop configuration tasks. Add a recipe to
`~/.config/gnoblin/init.lua` after any `gnoblin.load(...)` lines so your choices
override settings loaded from other files.

After editing the file, apply the changes with `gnoblinctl config reload`.

## Start and organize a config

- [Complete starter config](/recipes/complete-starter-config): combine a shell
  setting, shortcut and window rule.
- [Split a config into files](/recipes/split-config-into-files): load separate
  files for appearance and shortcuts.

## Shortcuts and keyboards

- [Add a shortcut](/recipes/add-a-shortcut): add or change one named shortcut
  without replacing others.
- [Rebind close window](/recipes/rebind-close-window): override a built-in
  window-manager keybinding.
- [Make Caps Lock an Escape key](/recipes/caps-lock-as-escape): set a common
  XKB keyboard option.
- [Use a different keyboard layout per window](/recipes/per-window-keyboard-layouts):
  keep the active input source with each window.

## Pointing devices

- [Enable touchpad tap-to-click](/recipes/touchpad-tap-to-click): turn on taps
  and two-finger scrolling.
- [Set up a left-handed mouse](/recipes/left-handed-mouse): swap buttons and
  adjust pointer speed.
- [Keep tablet drawing proportions](/recipes/tablet-keep-aspect): map a tablet
  absolutely and preserve its aspect ratio.
- [Switch monitors with a stylus button](/recipes/stylus-switch-monitor): map a
  supported stylus button to the monitor-switch action.
- [Make the cursor larger](/recipes/larger-cursor): set a larger compositor
  cursor size.

## Windows and compositor

- [Dim unfocused windows](/recipes/dim-unfocused-windows): lower the opacity of
  windows that are not focused.
- [Round application windows](/recipes/round-application-windows): set window
  corner radius and smoothing.
- [Highlight the focused window](/recipes/accent-focused-windows): draw a
  border only around the active window.
- [Fade one application's windows](/recipes/fade-one-application): animate
  matching windows without changing other apps.
- [Blur one application's background](/recipes/blur-one-application): blur
  translucent windows from a matching app.
- [Tint windows with a shader](/recipes/tint-window-with-shader): apply a small
  GLSL effect with a configurable uniform.
- [Turn off layer animations](/recipes/turn-off-layer-animations): disable
  compositor animations for layer surfaces.
- [Request a Gnoblin titlebar](/recipes/request-a-titlebar): provide a server
  titlebar when an application requests one.
- [Center windows and attach dialogs](/recipes/center-windows-and-dialogs):
  set initial placement and modal-dialog behavior.
- [Name fixed workspaces](/recipes/name-fixed-workspaces): use a stable set of
  numbered workspaces with names.
- [Combine window rules](/recipes/combine-window-rules): apply defaults and
  override selected values for matching windows.
- [Disable compositor animations](/recipes/disable-compositor-animations):
  turn off compositor-managed transitions.

## Permissions and feedback

- [Ask before undecided portal access](/recipes/ask-before-portal-access): make
  the fallback permission policy ask for consent.
- [Use a visual bell](/recipes/visual-bell): replace the audible bell with a
  brief frame flash.

See the [configuration reference](/config/configure) for available options and
the [configuration guides](/guides/window_rules) for behavior and tradeoffs.
