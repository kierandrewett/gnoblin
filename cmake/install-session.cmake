# Install Gnoblin's session data into a prefix. Run in script mode:
#
#   cmake -DGNOBLIN_PREFIX=<prefix> [-DGNOBLIN_BINARY_DIR=<dir>] -P cmake/install-session.cmake
#
# The step installs:
#   - the wayland-session .desktop entry (shown at the login manager)
#   - gnoblin-session.target and the idle service for the standalone login
#   - gnoblinctl, the man pages, the default config and the schema override
#
# This step is additive: everything lands under <prefix> and can be removed by
# deleting it. No system files are touched, and nothing here registers with
# your live login manager or systemd --user instance. That is handled by
# `./build.sh --register-session` (see docs/install-source.md), a separate
# explicit step because it changes state outside <prefix>.
#
# Inputs from the environment (set by the gnoblin-session target):
#   GNOBLIN_LIBDIR, GNOBLIN_STAGE_ROOT, GNOBLIN_VECTOR_CURSORS,
#   GNOBLIN_IDLE_BINARY, GNOBLINCTL_BINARY, GNOBLIN_IDENTITY_FILE,
#   GNOBLIN_VERSION_METADATA_FILE, ADWAITA_CURSOR_FALLBACK (optional)
cmake_minimum_required(VERSION 3.22)

get_filename_component(ROOT "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)
set(SRC "${ROOT}/src/data/session")

if(NOT GNOBLIN_PREFIX)
  message(FATAL_ERROR "usage: cmake -DGNOBLIN_PREFIX=<prefix> -P install-session.cmake")
endif()
set(PREFIX "${GNOBLIN_PREFIX}")

# Installation must never merge a Gnoblin runtime into a shared GNOME prefix.
get_filename_component(real_prefix "${PREFIX}" REALPATH)
if(real_prefix MATCHES "^(/|/usr|/usr/local|/bin|/sbin|/lib|/lib64)$")
  message(FATAL_ERROR
    "Refusing shared system prefix ${real_prefix}; use a private directory such as ./install or /usr/lib/gnoblin.")
endif()

if(DEFINED ENV{GNOBLIN_LIBDIR} AND NOT "$ENV{GNOBLIN_LIBDIR}" STREQUAL "")
  set(LIBDIR "$ENV{GNOBLIN_LIBDIR}")
else()
  set(LIBDIR "lib64")
endif()
if(LIBDIR MATCHES "^(\\.\\.|/.*|\\.\\./.*|.*/\\.\\./.*|.*/\\.\\.)$")
  message(FATAL_ERROR "invalid GNOBLIN_LIBDIR (must stay below the prefix): ${LIBDIR}")
endif()

foreach(name GNOBLIN_IDLE_BINARY GNOBLINCTL_BINARY GNOBLIN_IDENTITY_FILE GNOBLIN_VERSION_METADATA_FILE)
  if("$ENV{${name}}" STREQUAL "")
    message(FATAL_ERROR "${name} is not set. Build the session with ./build.sh")
  endif()
  set(${name} "$ENV{${name}}")
endforeach()

set(stage_root "$ENV{GNOBLIN_STAGE_ROOT}")
file(MAKE_DIRECTORY "${stage_root}${PREFIX}")
get_filename_component(INSTALL_PREFIX "${stage_root}${PREFIX}" REALPATH)

# The monolithic compositor is installed by the pinned Mutter build before
# this session payload runs.
if(NOT EXISTS "${INSTALL_PREFIX}/bin/gnoblin")
  message(FATAL_ERROR "Missing ${INSTALL_PREFIX}/bin/gnoblin -- build the Mutter component first")
endif()

set(MODE_644 OWNER_READ OWNER_WRITE GROUP_READ WORLD_READ)
set(MODE_755 OWNER_READ OWNER_WRITE OWNER_EXECUTE GROUP_READ GROUP_EXECUTE WORLD_READ WORLD_EXECUTE)

# Copy one file to an exact path and set its mode. The destination directory is
# created when it is missing.
function(gnoblin_install_file source destination mode)
  get_filename_component(directory "${destination}" DIRECTORY)
  file(MAKE_DIRECTORY "${directory}")
  configure_file("${source}" "${destination}" COPYONLY)
  file(CHMOD "${destination}" PERMISSIONS ${${mode}})
endfunction()

# Remove files below the install prefix. Missing files are not an error.
function(gnoblin_remove)
  foreach(relative IN LISTS ARGN)
    file(REMOVE "${INSTALL_PREFIX}/${relative}")
  endforeach()
endfunction()

# A prior development install may have included GNOME's extension manager,
# captive-network portal helper, calendar server, or test tools.
# Remove only the old Gnoblin-prefix artefacts. System GNOME files are never
# considered by this script.
gnoblin_remove(
  bin/gnome-shell
  bin/gnome-extensions
  bin/gnome-extensions-app
  share/applications/org.gnome.Extensions.desktop
  share/dbus-1/services/org.gnome.Extensions.service
  share/glib-2.0/schemas/org.gnome.Extensions.gschema.xml
  share/metainfo/org.gnome.Extensions.metainfo.xml
  share/gnome-shell/org.gnome.Extensions
  share/gnome-shell/org.gnome.Extensions.data.gresource
  share/gnome-shell/org.gnome.Extensions.src.gresource
  share/gnome-shell/org.gnome.Shell.Extensions
  share/gnome-shell/org.gnome.Shell.Extensions.src.gresource
  share/gnome-shell/modes/gnoblin.json
  share/gnome-shell/gnome-shell-dbus-interfaces.gresource
  share/gnome-shell/gnome-shell-icons.gresource
  share/gnome-shell/gnome-shell-osk-layouts.gresource
  share/gnome-shell/gnome-shell-theme.gresource
  share/gnome-shell/org.gnome.ScreenSaver
  share/gnome-shell/org.gnome.ScreenSaver.src.gresource
  share/gnome-shell/org.gnome.Shell.Notifications
  share/gnome-shell/org.gnome.Shell.Notifications.src.gresource
  share/gnome-shell/org.gnome.Shell.Screencast
  share/gnome-shell/org.gnome.Shell.Screencast.src.gresource
  share/gnome-shell/perf-background.xml
  share/bash-completion/completions/gnome-extensions
  share/applications/org.gnome.Shell.Extensions.desktop
  share/dbus-1/services/org.gnome.Shell.Extensions.service
  lib/systemd/user/org.gnome.Shell-disable-extensions.service
  libexec/gnoblin-runtime
  libexec/gnome-shell-hotplug-sniffer
  libexec/gnome-shell-portal-helper
  share/applications/org.gnome.Shell.PortalHelper.desktop
  share/dbus-1/services/org.gnome.Shell.PortalHelper.service
  libexec/gnome-shell-calendar-server
  share/dbus-1/services/org.gnome.Shell.CalendarServer.service
  bin/gnome-shell-test-tool
  libexec/gnome-shell-perf-helper)

# Remove only now-empty directories left by the old Shell payload. Unknown
# files under this private prefix are preserved.
foreach(relative share/gnome-shell/modes share/gnome-shell)
  set(directory "${INSTALL_PREFIX}/${relative}")
  if(IS_DIRECTORY "${directory}" AND NOT IS_SYMLINK "${directory}")
    file(GLOB entries "${directory}/*" "${directory}/.[!.]*" "${directory}/..?*")
    if(NOT entries)
      file(REMOVE_RECURSE "${directory}")
    endif()
  endif()
endforeach()

# Old icon names from the Extensions application. Symlinks are kept.
if(IS_DIRECTORY "${INSTALL_PREFIX}/share/icons/hicolor")
  file(GLOB_RECURSE old_icons LIST_DIRECTORIES false
    "${INSTALL_PREFIX}/share/icons/hicolor/org.gnome.Extensions*"
    "${INSTALL_PREFIX}/share/icons/hicolor/org.gnome.Shell.Extensions*")
  foreach(icon IN LISTS old_icons)
    if(NOT IS_SYMLINK "${icon}" AND NOT IS_DIRECTORY "${icon}")
      file(REMOVE "${icon}")
    endif()
  endforeach()
endif()

# Retain the shared environment helper for developer tools using this prefix.
gnoblin_install_file("${ROOT}/src/tools/gnoblin-env.sh" "${INSTALL_PREFIX}/libexec/gnoblin-env.sh" MODE_644)
file(WRITE "${INSTALL_PREFIX}/libexec/gnoblin-libdir" "${LIBDIR}\n")
file(CHMOD "${INSTALL_PREFIX}/libexec/gnoblin-libdir" PERMISSIONS ${MODE_644})
set(old_link "${INSTALL_PREFIX}/bin/gnoblin-session")
if(IS_SYMLINK "${old_link}")
  file(READ_SYMLINK "${old_link}" old_target)
  if(old_target STREQUAL "gnoblin")
    file(REMOVE "${old_link}")
  endif()
endif()

# Recovery and developer UI are compositor-resident, and the compositor itself
# is now named gnoblin. Remove only obsolete executable names from earlier
# source builds in this private prefix.
gnoblin_remove(
  bin/gnoblin-recovery
  bin/gnoblin-console
  bin/gnoblin-mutter
  bin/mutter
  lib/systemd/user/gnoblin-recovery.service
  ${LIBDIR}/mutter-51/plugins/libgnoblin.so)

gnoblin_install_file("${ROOT}/src/tools/gnoblin-seed-config" "${INSTALL_PREFIX}/libexec/gnoblin-seed-config" MODE_755)
gnoblin_install_file("${ROOT}/src/data/init.lua.example" "${INSTALL_PREFIX}/share/gnoblin/init.lua.example" MODE_644)
file(COPY "${ROOT}/src/data/default-config/" DESTINATION "${INSTALL_PREFIX}/share/gnoblin/default-config"
  USE_SOURCE_PERMISSIONS)
gnoblin_install_file("${ROOT}/src/data/gnoblin-portals.conf"
  "${INSTALL_PREFIX}/share/xdg-desktop-portal/gnoblin-portals.conf" MODE_644)

# The login manager runs the Exec line, so it must name the installed binary.
file(READ "${SRC}/gnoblin.desktop" desktop_entry)
string(REPLACE "\\" "\\\\" exec_prefix "${PREFIX}")
string(REGEX REPLACE "(^|\n)Exec=[^\n]*" "\\1Exec=${exec_prefix}/bin/gnoblin" desktop_entry "${desktop_entry}")
file(WRITE "${INSTALL_PREFIX}/share/wayland-sessions/gnoblin.desktop" "${desktop_entry}")
file(CHMOD "${INSTALL_PREFIX}/share/wayland-sessions/gnoblin.desktop" PERMISSIONS ${MODE_644})

# Normal Xcursor themes work without compiling extra artwork. Keep the custom
# vector theme available to users who explicitly request it from build.sh.
set(vector_cursors "$ENV{GNOBLIN_VECTOR_CURSORS}")
if(vector_cursors STREQUAL "")
  set(vector_cursors OFF)
endif()
if(vector_cursors MATCHES "^(ON|TRUE|true|1)$")
  set(vector_cursors_enabled TRUE)
elseif(vector_cursors MATCHES "^(OFF|FALSE|false|0)$")
  set(vector_cursors_enabled FALSE)
else()
  message(FATAL_ERROR "invalid vector cursor setting: ${vector_cursors}")
endif()
if(vector_cursors_enabled)
  if(GNOBLIN_BINARY_DIR)
    set(work_parent "${GNOBLIN_BINARY_DIR}")
  else()
    set(work_parent "${INSTALL_PREFIX}")
  endif()
  set(theme_build "${work_parent}/adwaita-hyprcursor-build")
  file(REMOVE_RECURSE "${theme_build}")
  if(DEFINED ENV{ADWAITA_CURSOR_FALLBACK} AND NOT "$ENV{ADWAITA_CURSOR_FALLBACK}" STREQUAL "")
    set(cursor_fallback "$ENV{ADWAITA_CURSOR_FALLBACK}")
  else()
    set(cursor_fallback "/usr/share/icons/Adwaita")
  endif()
  execute_process(
    COMMAND python3 "${ROOT}/scripts/build-adwaita-hyprcursor.py"
      --output "${theme_build}/Adwaita-Hyprcursor" --fallback "${cursor_fallback}"
    RESULT_VARIABLE status)
  if(NOT status EQUAL 0)
    file(REMOVE_RECURSE "${theme_build}")
    message(FATAL_ERROR "build-adwaita-hyprcursor.py failed with status ${status}")
  endif()
  file(MAKE_DIRECTORY "${INSTALL_PREFIX}/share/icons/Adwaita-Hyprcursor")
  file(COPY "${theme_build}/Adwaita-Hyprcursor/" DESTINATION "${INSTALL_PREFIX}/share/icons/Adwaita-Hyprcursor"
    USE_SOURCE_PERMISSIONS)
  file(REMOVE_RECURSE "${theme_build}")
endif()

# Standalone session services. Recovery and developer UI are compositor-resident.
gnoblin_install_file("${SRC}/systemd-user/gnoblin-session.target"
  "${INSTALL_PREFIX}/lib/systemd/user/gnoblin-session.target" MODE_644)
gnoblin_install_file("${GNOBLIN_IDLE_BINARY}" "${INSTALL_PREFIX}/libexec/gnoblin-idle" MODE_755)
# The unit file names the installed binary, so @PREFIX@ is replaced with the prefix.
configure_file("${SRC}/systemd-user/gnoblin-idle.service.in"
  "${INSTALL_PREFIX}/lib/systemd/user/gnoblin-idle.service" @ONLY)
file(CHMOD "${INSTALL_PREFIX}/lib/systemd/user/gnoblin-idle.service" PERMISSIONS ${MODE_644})

# Manual pages. gnoblinctl(1) is generated from the table that the command line
# help reads, so the two agree.
execute_process(
  COMMAND python3 "${ROOT}/scripts/build-man-pages.py" "${INSTALL_PREFIX}/share/man/man1"
  RESULT_VARIABLE status)
if(NOT status EQUAL 0)
  message(FATAL_ERROR "build-man-pages.py failed with status ${status}")
endif()

# Desktop-specific schema defaults. This runs after Mutter has installed its
# schemas, so the override is compiled into the prefix used by Gnoblin's
# session (XDG_CURRENT_DESKTOP=Gnoblin).
gnoblin_install_file("${SRC}/schemas/00_org.gnoblin.mutter.gschema.override"
  "${INSTALL_PREFIX}/share/glib-2.0/schemas/00_org.gnoblin.mutter.gschema.override" MODE_644)
execute_process(
  COMMAND glib-compile-schemas "${INSTALL_PREFIX}/share/glib-2.0/schemas"
  RESULT_VARIABLE status)
if(NOT status EQUAL 0)
  message(FATAL_ERROR "glib-compile-schemas failed with status ${status}")
endif()

# The gnoblinctl CLI for controlling the active Gnoblin session.
gnoblin_install_file("${GNOBLINCTL_BINARY}" "${INSTALL_PREFIX}/bin/gnoblinctl" MODE_755)
gnoblin_install_file("${GNOBLIN_IDENTITY_FILE}" "${INSTALL_PREFIX}/share/gnoblin/version.json" MODE_644)
gnoblin_install_file("${GNOBLIN_VERSION_METADATA_FILE}" "${INSTALL_PREFIX}/share/gnoblin/version.ini" MODE_644)

message(STATUS "Session data installed in ${PREFIX}")
