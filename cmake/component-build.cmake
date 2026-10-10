# Build and install one upstream Meson project (Mutter or the portal backend). Run in script mode from the superbuild:
#
#   cmake -DNAME=<project> -DROOT=<source root> -DBUILD_ROOT=<build dir> -DPREFIX=<prefix> -DLIBDIR=<libdir>
#     -DBUILDTYPE=<meson buildtype> -DDEVKIT=<enabled|disabled> -DJOBS=<n> [-DSTAGE_ROOT=<dir>]
#     [-DXWAYLAND=ON] [-DVECTOR_CURSORS=OFF] [-DPYTHON=<python3>] -P cmake/component-build.cmake
#
# The patches are already applied by the gnoblin-sources step (cmake/source-step.cmake). This script runs Meson, and it
# sets the environment at build time, because the search paths depend on what the caller has in the environment.
cmake_minimum_required(VERSION 3.22)

foreach(required IN ITEMS NAME ROOT BUILD_ROOT PREFIX LIBDIR BUILDTYPE DEVKIT JOBS)
  if(NOT DEFINED ${required} OR "${${required}}" STREQUAL "")
    message(FATAL_ERROR "component-build: ${required} is required")
  endif()
endforeach()
if(NOT DEFINED XWAYLAND)
  set(XWAYLAND ON)
endif()
if(NOT DEFINED VECTOR_CURSORS)
  set(VECTOR_CURSORS OFF)
endif()
if(NOT DEFINED PYTHON)
  set(PYTHON python3)
endif()

# Read a pin from gnome-versions.py, which owns them.
function(component_pin out project field)
  execute_process(COMMAND "${ROOT}/scripts/gnome-versions.py" get "${project}" "${field}"
    OUTPUT_VARIABLE value RESULT_VARIABLE result OUTPUT_STRIP_TRAILING_WHITESPACE)
  if(result)
    message(FATAL_ERROR "gnome-versions.py could not read ${field} for ${project}")
  endif()
  set(${out} "${value}" PARENT_SCOPE)
endfunction()

# Put a value in front of a search path. A path that is not set yet gets only the value.
function(component_env_prepend variable value)
  if("$ENV{${variable}}" STREQUAL "")
    set(ENV{${variable}} "${value}")
  else()
    set(ENV{${variable}} "${value}:$ENV{${variable}}")
  endif()
endfunction()

# Map the many spellings of a CMake boolean to true or false.
function(component_boolean out setting label)
  string(TOUPPER "${setting}" upper)
  if(upper MATCHES "^(ON|TRUE|1)$")
    set(${out} true PARENT_SCOPE)
  elseif(upper MATCHES "^(OFF|FALSE|0)$")
    set(${out} false PARENT_SCOPE)
  else()
    message(FATAL_ERROR "invalid ${label} setting: ${setting}")
  endif()
endfunction()

component_pin(mutter_version mutter version)
string(REGEX REPLACE "\\..*$" "" mutter_api "${mutter_version}")
set(source_dir "${ROOT}/subprojects/${NAME}")
set(build_dir "${BUILD_ROOT}/${NAME}")
set(installed_prefix "${STAGE_ROOT}${PREFIX}")

if(NAME STREQUAL "xdg-desktop-portal-gnome")
  set(options "-Ddbus_service_dir=${PREFIX}/share/dbus-1/services"
    "-Dsystemduserunitdir=${PREFIX}/lib/systemd/user")
elseif(NAME STREQUAL "mutter")
  component_boolean(xwayland_option "${XWAYLAND}" XWayland)
  component_boolean(vector_cursor_option "${VECTOR_CURSORS}" "vector cursor")
  if(vector_cursor_option)
    set(hyprcursor enabled)
  else()
    set(hyprcursor disabled)
  endif()
  set(options "-Ddevkit=${DEVKIT}" "-Dxwayland=${xwayland_option}" -Dlibgnome_desktop=false -Dtests=disabled
    -Ddocs=false -Dprofiler=false -Dbash_completion=false "-Dudev_dir=${PREFIX}/lib/udev"
    "-Dhyprcursor=${hyprcursor}")
else()
  set(options)
endif()

if(STAGE_ROOT)
  set(private_pc "${BUILD_ROOT}/pkgconfig")
  file(MAKE_DIRECTORY "${private_pc}")
  component_env_prepend(PKG_CONFIG_PATH "${private_pc}")
else()
  component_env_prepend(PKG_CONFIG_PATH "${PREFIX}/${LIBDIR}/pkgconfig:${PREFIX}/share/pkgconfig")
endif()
set(ENV{GNOBLIN_SOURCE_ROOT} "${ROOT}")
set(ENV{GNOBLIN_IMGUI_SOURCE} "${ROOT}/subprojects/imgui")
set(ENV{GNOBLIN_PREFIX} "${PREFIX}")
component_env_prepend(GI_GIR_PATH "${installed_prefix}/share/gir-1.0")
component_env_prepend(GI_TYPELIB_PATH
  "${installed_prefix}/${LIBDIR}/girepository-1.0:${installed_prefix}/${LIBDIR}/mutter-${mutter_api}")
component_env_prepend(LD_LIBRARY_PATH "${installed_prefix}/${LIBDIR}:${installed_prefix}/${LIBDIR}/mutter-${mutter_api}")
component_env_prepend(PATH "${installed_prefix}/bin")

function(component_meson)
  execute_process(COMMAND meson ${ARGN} COMMAND_ERROR_IS_FATAL ANY)
endfunction()

set(coredata "${build_dir}/meson-private/coredata.dat")
set(setup_arguments "${build_dir}" "${source_dir}" "--prefix=${PREFIX}" "--libdir=${LIBDIR}"
  "--buildtype=${BUILDTYPE}" ${options})
if(EXISTS "${coredata}" AND EXISTS "${source_dir}/meson.options" AND "${source_dir}/meson.options" IS_NEWER_THAN "${coredata}")
  # Meson checks command line options against cached coredata before it finds newly added project options.
  # A reconfigure cannot add them, so start again.
  component_meson(setup --wipe ${setup_arguments})
elseif(EXISTS "${coredata}")
  component_meson(setup --reconfigure ${setup_arguments})
else()
  component_meson(setup ${setup_arguments})
endif()
component_meson(compile -C "${build_dir}" -j "${JOBS}")
if(STAGE_ROOT)
  component_meson(install -C "${build_dir}" --destdir "${STAGE_ROOT}" --no-rebuild)
  # Later components find the staged libraries through pkg-config files that point into the stage.
  file(GLOB staged_pc "${installed_prefix}/${LIBDIR}/pkgconfig/*.pc" "${installed_prefix}/share/pkgconfig/*.pc")
  foreach(pc IN LISTS staged_pc)
    get_filename_component(pc_name "${pc}" NAME)
    file(READ "${pc}" pc_text)
    string(REPLACE "${PREFIX}" "${installed_prefix}" pc_text "${pc_text}")
    file(WRITE "${private_pc}/${pc_name}" "${pc_text}")
  endforeach()
else()
  component_meson(install -C "${build_dir}" --no-rebuild)
endif()

if(NAME STREQUAL "mutter")
  set(schemas_source_dir "${ROOT}/build/source-inputs/gsettings-desktop-schemas-pinned")
  component_pin(expected_schemas_revision gsettings-desktop-schemas commit)
  execute_process(COMMAND git -C "${schemas_source_dir}" rev-parse HEAD
    OUTPUT_VARIABLE actual_schemas_revision ERROR_QUIET RESULT_VARIABLE git_result OUTPUT_STRIP_TRAILING_WHITESPACE)
  if(git_result)
    set(actual_schemas_revision "")
    if(EXISTS "${schemas_source_dir}/GNOBLIN_SOURCE_REVISION")
      file(READ "${schemas_source_dir}/GNOBLIN_SOURCE_REVISION" actual_schemas_revision)
      string(STRIP "${actual_schemas_revision}" actual_schemas_revision)
    endif()
  endif()
  if(NOT actual_schemas_revision STREQUAL expected_schemas_revision)
    message(FATAL_ERROR "pinned gsettings-desktop-schemas source is missing or mismatched; prepare it with: "
      "cmake -DACTION=prepare -DSOURCE_MODE=<mode> -DPROJECTS=gsettings-desktop-schemas -P cmake/source-step.cmake")
  endif()
  execute_process(COMMAND "${PYTHON}" "${ROOT}/scripts/generate-mutter-keybinding-catalog.py"
    "${source_dir}" "${schemas_source_dir}" "${installed_prefix}/share/gnoblin/native-keybindings.json"
    COMMAND_ERROR_IS_FATAL ANY)
  set(devkit_marker "${installed_prefix}/share/gnoblin/mutter-devkit-enabled")
  if(DEVKIT MATCHES "^(enabled|true|TRUE|1)$")
    file(WRITE "${devkit_marker}" "")
  else()
    file(REMOVE "${devkit_marker}")
  endif()
endif()
if(NAME STREQUAL "xdg-desktop-portal-gnome" AND IS_DIRECTORY "${installed_prefix}/share/glib-2.0/schemas")
  execute_process(COMMAND glib-compile-schemas "${installed_prefix}/share/glib-2.0/schemas"
    COMMAND_ERROR_IS_FATAL ANY)
endif()
