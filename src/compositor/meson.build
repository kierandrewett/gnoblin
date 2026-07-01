# gnoblin-shell — a minimal libmutter compositor.
#
# Unlike the protocol overlays under src/<feature>/ (which are copied INTO the
# mutter submodule and built as part of libmutter), gnoblin-shell is a
# first-class, standalone program. It links the *installed* libmutter-17 — the
# same one gnoblin's patched mutter ships — via pkg-config. Concrete, owned
# source; no submodule, no patches.
#
# The shell is C++; the shared config parser (../config/gnoblin-config.c) stays
# C because the libmutter protocol overlays also use it.
#
# Build standalone:
#     meson setup build/gnoblin-shell src/compositor
#     meson compile -C build/gnoblin-shell

project('gnoblin-shell', ['c', 'cpp'],
  version : '0.1',
  default_options : ['c_std=gnu11', 'cpp_std=c++20', 'warning_level=2'],
)

libmutter_dep = dependency('libmutter-17')
gio_unix_dep = dependency('gio-unix-2.0')

# GLib/Clutter vtable callbacks have fixed signatures with unavoidably-unused
# parameters; keep warning_level=2 otherwise.
cc = meson.get_compiler('c')
cxx = meson.get_compiler('cpp')
add_project_arguments(cc.get_supported_arguments('-Wno-unused-parameter'), language : 'c')
add_project_arguments(cxx.get_supported_arguments('-Wno-unused-parameter'), language : 'cpp')

# PAM for the lock screen's password check.
pam_dep = cxx.find_library('pam')
# cairo for the self-screenshot PNG writer.
cairo_dep = dependency('cairo')

# Bake gnoblin.defaults.conf into the compositor as a C string so the config
# parser can layer it UNDER the user's gnoblin.conf (shipped defaults live in
# the config file, not hardcoded in code — same contract the Rust shell honours
# via include_str!). The mutter protocol overlays compile the same config.c
# WITHOUT this header; the parser guards the embed behind GNOBLIN_EMBED_DEFAULTS.
defaults_conf_h = custom_target('gnoblin-defaults-conf-h',
  input : '../data/gnoblin.defaults.conf',
  output : 'gnoblin-defaults-conf.h',
  command : [
    find_program('python3'),
    files('../../scripts/stringify-file.py'),
    '@INPUT@', '@OUTPUT@', 'GNOBLIN_DEFAULTS_CONF',
  ],
)

executable('gnoblin-shell',
  sources : [
    defaults_conf_h,
    'gnoblin-shell.cpp',
    'gnoblin-shell-plugin.cpp',
    'gnoblin-actions.cpp',
    'gnoblin-actions-spec.c',
    'gnoblin-anim.cpp',
    'gnoblin-anim-spec.c',
    'gnoblin-blur.cpp',
    'gnoblin-color-spec.c',
    'gnoblin-control.cpp',
    'gnoblin-gestures.cpp',
    'gnoblin-input.cpp',
    'gnoblin-input-spec.c',
    'gnoblin-lock.cpp',
    'gnoblin-output.cpp',
    'gnoblin-output-spec.c',
    'gnoblin-overview.cpp',
    'gnoblin-roles.cpp',
    'gnoblin-rounded.cpp',
    'gnoblin-rules.cpp',
    'gnoblin-rules-spec.c',
    'gnoblin-shadow.cpp',
    'gnoblin-shadow-spec.c',
    'gnoblin-spec-util.c',
    'gnoblin-switcher.cpp',
    '../config/gnoblin-config.c',
  ],
  include_directories : include_directories('../config'),
  dependencies : [libmutter_dep, gio_unix_dep, pam_dep],
  c_args : [
    '-DGNOBLIN_EMBED_DEFAULTS',
  ],
  cpp_args : [
    '-DGNOBLIN_SHELL_VERSION="@0@"'.format(meson.project_version()),
  ],
  install : true,
)
