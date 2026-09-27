cmake_minimum_required(VERSION 3.22)
project(GnoblinBuild C)

# Ninja owns the source-build graph; Meson remains the upstream project builder.
if(NOT CMAKE_GENERATOR STREQUAL "Ninja")
  message(FATAL_ERROR "Gnoblin source builds require -G Ninja")
endif()
include(ExternalProject)
find_package(Python3 3.11 REQUIRED COMPONENTS Interpreter)
find_package(PkgConfig REQUIRED)
pkg_check_modules(GNOBLIN_GIO REQUIRED IMPORTED_TARGET gio-2.0 gio-unix-2.0)
pkg_check_modules(GNOBLIN_CONTROL REQUIRED IMPORTED_TARGET gio-2.0 gio-unix-2.0 json-glib-1.0)
add_executable(gnoblin-idle src/session/gnoblin-idle.c)
target_link_libraries(gnoblin-idle PRIVATE PkgConfig::GNOBLIN_GIO)
add_executable(gnoblinctl src/tools/gnoblinctl.c)
target_link_libraries(gnoblinctl PRIVATE PkgConfig::GNOBLIN_CONTROL)
add_custom_target(gnoblinctl-identity
  COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/scripts/build-identity.py"
    "${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
  BYPRODUCTS "${CMAKE_BINARY_DIR}/gnoblinctl-identity.json")
add_dependencies(gnoblinctl gnoblinctl-identity)
target_compile_definitions(gnoblinctl PRIVATE
  "GNOBLIN_IDENTITY_FALLBACK=\"${CMAKE_BINARY_DIR}/gnoblinctl-identity.json\"")

set(GNOBLIN_PREFIX "${PROJECT_SOURCE_DIR}/install" CACHE PATH "Private Gnoblin install prefix")
set(GNOBLIN_LIBDIR "lib64" CACHE STRING "Library directory under the prefix")
set(GNOBLIN_BUILD_TYPE "debugoptimized" CACHE STRING "Meson build type")
set(GNOBLIN_DEVKIT "disabled" CACHE STRING "Build the Mutter devkit")
option(GNOBLIN_XWAYLAND "Support X11 applications through XWayland" ON)
option(GNOBLIN_VECTOR_CURSORS "Build the optional Adwaita vector cursor theme" OFF)
set(GNOBLIN_SOURCE_MODE "checkout" CACHE STRING "checkout or release-archive")
set(GNOBLIN_STAGE_ROOT "" CACHE PATH "Optional package staging root")
set_property(CACHE GNOBLIN_SOURCE_MODE PROPERTY STRINGS checkout release-archive)
set(GNOBLIN_JOBS "4" CACHE STRING "Parallel jobs inside each Meson build")

get_filename_component(GNOBLIN_PREFIX "${GNOBLIN_PREFIX}" ABSOLUTE)
if(GNOBLIN_PREFIX MATCHES "^(/|/usr|/usr/local|/bin|/sbin|/lib|/lib64)$")
  message(FATAL_ERROR "Use a private install prefix, such as ./install")
endif()
if(GNOBLIN_LIBDIR MATCHES "^/|(^|/)\.\.(/|$)" OR GNOBLIN_LIBDIR STREQUAL "")
  message(FATAL_ERROR "GNOBLIN_LIBDIR must stay below GNOBLIN_PREFIX")
endif()
if(NOT GNOBLIN_JOBS MATCHES "^[1-9][0-9]*$")
  message(FATAL_ERROR "GNOBLIN_JOBS must be a positive integer")
endif()

set(GNOBLIN_REQUIREMENTS_OPTIONS)
if(NOT GNOBLIN_XWAYLAND)
  list(APPEND GNOBLIN_REQUIREMENTS_OPTIONS --without-xwayland)
endif()
if(GNOBLIN_VECTOR_CURSORS)
  list(APPEND GNOBLIN_REQUIREMENTS_OPTIONS --with-vector-cursors)
endif()

foreach(name IN ITEMS mutter gnome-shell xdg-desktop-portal-gnome)
  add_custom_target("prepare-${name}-source"
    COMMAND "${PROJECT_SOURCE_DIR}/scripts/prepare-build-sources.sh" "${GNOBLIN_SOURCE_MODE}" "${name}"
    WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}"
    USES_TERMINAL
    COMMENT "Prepare pinned ${name} source")
  add_custom_target("check-${name}-requirements"
    COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/scripts/check-build-deps.py" "${GNOBLIN_SOURCE_MODE}" "${name}" ${GNOBLIN_REQUIREMENTS_OPTIONS}
    DEPENDS "prepare-${name}-source"
    USES_TERMINAL
    COMMENT "Check ${name} development libraries")
endforeach()
add_custom_target(prepare-sources DEPENDS
  prepare-mutter-source prepare-gnome-shell-source
  prepare-xdg-desktop-portal-gnome-source)

function(gnoblin_component name)
  ExternalProject_Add("${name}"
    SOURCE_DIR "${PROJECT_SOURCE_DIR}/subprojects/${name}"
    BINARY_DIR "${CMAKE_BINARY_DIR}/${name}"
    DOWNLOAD_COMMAND ""
    UPDATE_COMMAND ""
    PATCH_COMMAND ""
    CONFIGURE_COMMAND ""
    BUILD_ALWAYS TRUE
    BUILD_COMMAND "${PROJECT_SOURCE_DIR}/scripts/build-component.sh" "${name}"
      "${PROJECT_SOURCE_DIR}" "${CMAKE_BINARY_DIR}" "${GNOBLIN_PREFIX}"
      "${GNOBLIN_LIBDIR}" "${GNOBLIN_BUILD_TYPE}" "${GNOBLIN_DEVKIT}" "${GNOBLIN_JOBS}" "${GNOBLIN_SOURCE_MODE}"
      "${GNOBLIN_STAGE_ROOT}" "${GNOBLIN_XWAYLAND}"
      "${GNOBLIN_VECTOR_CURSORS}"
    INSTALL_COMMAND ""
    DEPENDS "check-${name}-requirements" ${ARGN}
    USES_TERMINAL_BUILD TRUE)
endfunction()

gnoblin_component(mutter)
gnoblin_component(gnome-shell mutter)
gnoblin_component(xdg-desktop-portal-gnome)

add_custom_target(session
  COMMAND "${CMAKE_COMMAND}" -E env "GNOBLIN_LIBDIR=${GNOBLIN_LIBDIR}"
    "GNOBLIN_STAGE_ROOT=${GNOBLIN_STAGE_ROOT}"
    "GNOBLIN_VECTOR_CURSORS=${GNOBLIN_VECTOR_CURSORS}"
    "GNOBLIN_IDLE_BINARY=$<TARGET_FILE:gnoblin-idle>"
    "GNOBLINCTL_BINARY=$<TARGET_FILE:gnoblinctl>"
    "GNOBLIN_IDENTITY_FILE=${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
    "${PROJECT_SOURCE_DIR}/scripts/install-session.sh" "${GNOBLIN_PREFIX}"
  DEPENDS gnome-shell xdg-desktop-portal-gnome gnoblin-idle gnoblinctl
  USES_TERMINAL
  COMMENT "Install Gnoblin session data")
add_custom_target(gnoblin ALL DEPENDS session)
