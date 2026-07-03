# gnoblin control-center panel — additive gnoblin overlay (src/control-center/).
# Registered by adding 'gnoblin' to panels/meson.build's list (one-line patch) and
# a matching PANEL_TYPE row in shell/cc-panel-loader.c. Uses plain GtkBuilder .ui
# (compiled straight into a gresource) so it needs no blueprint-compiler step.

panels_list += cappletname
desktop = 'gnome-@0@-panel.desktop'.format(cappletname)

i18n.merge_file(
  type: 'desktop',
  input: desktop + '.in',
  output: desktop,
  po_dir: po_dir,
  install: true,
  install_dir: control_center_desktopdir
)

sources = files(
  'cc-gnoblin-panel.c',
)

sources += gnome.compile_resources(
  'cc-' + cappletname + '-resources',
  cappletname + '.gresource.xml',
  c_name: 'cc_' + cappletname,
  export: true,
)

panels_libs += static_library(
  cappletname,
  sources: sources,
  include_directories: [ top_inc, shell_inc, common_inc ],
  dependencies: common_deps,
  c_args: cflags
)
