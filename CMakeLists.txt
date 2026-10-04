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
pkg_check_modules(GNOBLIN_XKBCOMMON REQUIRED IMPORTED_TARGET xkbcommon)
# The cached pkg-config result can outlive a removed Lua development package.
# Recheck it so a stale build tree fails here instead of at the C compiler.
unset(GNOBLIN_LUA_FOUND CACHE)
pkg_search_module(GNOBLIN_LUA QUIET IMPORTED_TARGET
  lua5.4>=5.4 lua-5.4>=5.4 lua54>=5.4 lua>=5.4)
if(NOT GNOBLIN_LUA_FOUND)
  message(FATAL_ERROR
    "Lua 5.4 or newer development files are required for the Gnoblin runtime. Install the development package (for example: Fedora lua-devel, Debian/Ubuntu liblua5.4-dev, or Arch lua) and make its pkg-config metadata and headers available. The Lua command-line interpreter alone is not sufficient.")
endif()
file(STRINGS "${PROJECT_SOURCE_DIR}/src/native-control/gnoblin-native-control.h"
  GNOBLIN_NATIVE_CONTROL_API_MAJOR_LINE
  REGEX "^#define GNOBLIN_NATIVE_CONTROL_API_MAJOR [0-9]+$")
file(STRINGS "${PROJECT_SOURCE_DIR}/src/native-control/gnoblin-native-control.h"
  GNOBLIN_NATIVE_CONTROL_API_MINOR_LINE
  REGEX "^#define GNOBLIN_NATIVE_CONTROL_API_MINOR [0-9]+$")
if(NOT GNOBLIN_NATIVE_CONTROL_API_MAJOR_LINE OR NOT GNOBLIN_NATIVE_CONTROL_API_MINOR_LINE)
  message(FATAL_ERROR "Could not read the native API version from gnoblin-native-control.h")
endif()
string(REGEX REPLACE "^#define GNOBLIN_NATIVE_CONTROL_API_MAJOR " ""
  GNOBLIN_NATIVE_CONTROL_API_MAJOR "${GNOBLIN_NATIVE_CONTROL_API_MAJOR_LINE}")
string(REGEX REPLACE "^#define GNOBLIN_NATIVE_CONTROL_API_MINOR " ""
  GNOBLIN_NATIVE_CONTROL_API_MINOR "${GNOBLIN_NATIVE_CONTROL_API_MINOR_LINE}")
set(GNOBLIN_PREFIX "${PROJECT_SOURCE_DIR}/install" CACHE PATH "Private Gnoblin install prefix")
set(GNOBLIN_LIBDIR "lib64" CACHE STRING "Library directory under the prefix")
enable_testing()
add_executable(native-touchpad-router-test
  tests/native-touchpad-router-test.c
  src/native-control/gnoblin-touchpad-router.c)
target_link_libraries(native-touchpad-router-test PRIVATE PkgConfig::GNOBLIN_GIO m)
add_test(NAME native-touchpad-router COMMAND native-touchpad-router-test)
add_executable(runtime-protocol-test
  tests/runtime-protocol-test.c
  src/native-control/gnoblin-runtime-protocol.c)
target_link_libraries(runtime-protocol-test PRIVATE PkgConfig::GNOBLIN_GIO)
add_test(NAME runtime-protocol COMMAND runtime-protocol-test)
set_tests_properties(runtime-protocol PROPERTIES TIMEOUT 10)
add_executable(runtime-spawn-test
  tests/runtime-spawn-test.c
  src/session/gnoblin-runtime-spawn.c)
target_link_libraries(runtime-spawn-test PRIVATE PkgConfig::GNOBLIN_GIO)
add_test(NAME runtime-spawn COMMAND runtime-spawn-test)
add_executable(autostart-protocol-test
  tests/autostart-protocol-test.c
  src/session/gnoblin-autostart.c
  src/native-control/gnoblin-runtime-protocol.c)
target_link_libraries(autostart-protocol-test PRIVATE PkgConfig::GNOBLIN_GIO)
add_test(NAME autostart-protocol COMMAND autostart-protocol-test)
add_executable(runtime-cache-test
  tests/runtime-cache-test.c
  src/native-control/gnoblin-runtime-cache.c)
target_link_libraries(runtime-cache-test PRIVATE PkgConfig::GNOBLIN_GIO)
add_test(NAME runtime-cache COMMAND runtime-cache-test)
add_executable(lua-pattern-test
  tests/lua-pattern-test.c
  src/config/gnoblin-lua-pattern.c)
target_include_directories(lua-pattern-test PRIVATE src/config)
target_link_libraries(lua-pattern-test PRIVATE PkgConfig::GNOBLIN_GIO)
add_test(NAME lua-pattern COMMAND lua-pattern-test)
if(GNOBLIN_LUA_FOUND)
  set(GNOBLIN_CONFIG_SOURCES
    src/config/gnoblin-config.c
    src/native-control/gnoblin-input-config.c
    src/config/gnoblin-lua.c
    src/config/gnoblin-lua-pattern.c
    src/config/gnoblin-glob.c
    src/permissions/gnoblin-portal-identity.c
    src/permissions/gnoblin-portal-policy.c)
  add_library(gnoblin-config-core STATIC ${GNOBLIN_CONFIG_SOURCES})
  target_include_directories(gnoblin-config-core PUBLIC
    src/config src/permissions src/native-control)
  target_compile_definitions(gnoblin-config-core PUBLIC
    GNOBLIN_NATIVE_CONTROL_API_MAJOR=${GNOBLIN_NATIVE_CONTROL_API_MAJOR}
    GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR})
  target_link_libraries(gnoblin-config-core PUBLIC
    PkgConfig::GNOBLIN_GIO PkgConfig::GNOBLIN_CONTROL PkgConfig::GNOBLIN_XKBCOMMON
    PkgConfig::GNOBLIN_LUA m)

  foreach(test_name IN ITEMS lua-config lua-api glob-config)
    add_executable(${test_name}-test tests/${test_name}-test.c)
    target_link_libraries(${test_name}-test PRIVATE gnoblin-config-core)
    add_test(NAME ${test_name} COMMAND ${test_name}-test)
    set_tests_properties(${test_name} PROPERTIES TIMEOUT 20)
  endforeach()
set_tests_properties(lua-config PROPERTIES
  ENVIRONMENT
    "GNOBLIN_TEST_SOURCE_ROOT=${PROJECT_SOURCE_DIR};GNOBLIN_PREFIX=${CMAKE_CURRENT_BINARY_DIR}/gnoblin-config-test-prefix")
set(GNOBLIN_CONFIG_TEST_CATALOG_DIR
  "${CMAKE_CURRENT_BINARY_DIR}/gnoblin-config-test-prefix/share/gnoblin")
file(MAKE_DIRECTORY "${GNOBLIN_CONFIG_TEST_CATALOG_DIR}")
configure_file(
  "${PROJECT_SOURCE_DIR}/tests/fixtures/native-keybindings.json"
  "${GNOBLIN_CONFIG_TEST_CATALOG_DIR}/native-keybindings.json"
  COPYONLY)

  add_executable(gnoblin
    src/session/gnoblin-runtime.c
    src/session/gnoblin-autostart.c
    src/native-control/gnoblin-runtime-protocol.c
    src/session/gnoblin-runtime-spawn.c)
  target_include_directories(gnoblin PRIVATE src/config src/permissions src/native-control)
  target_link_libraries(gnoblin PRIVATE
    gnoblin-config-core PkgConfig::GNOBLIN_GIO PkgConfig::GNOBLIN_CONTROL)
  set(GNOBLIN_BINARY "$<TARGET_FILE:gnoblin>")
endif()
add_executable(gnoblin-idle src/session/gnoblin-idle.c)
target_link_libraries(gnoblin-idle PRIVATE PkgConfig::GNOBLIN_GIO)
add_executable(gnoblinctl src/tools/gnoblinctl.c)
target_link_libraries(gnoblinctl PRIVATE PkgConfig::GNOBLIN_CONTROL PkgConfig::GNOBLIN_LUA)
add_test(NAME gnoblinctl-cli
  COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/tests/gnoblinctl-cli.test.py"
    "$<TARGET_FILE:gnoblinctl>" "${CMAKE_BINARY_DIR}")
set_tests_properties(gnoblinctl-cli PROPERTIES TIMEOUT 15)
add_test(NAME permission-contract
  COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/tests/permission-contract.test.py")
add_test(NAME cursor-config-contract
  COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/tests/cursor-config-contract.test.py")
add_test(NAME window-button-layout-contract
  COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/tests/window-button-layout-contract.test.py")
add_custom_target(gnoblinctl-identity
  COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/scripts/build-identity.py"
    "${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
    --lua-version "${GNOBLIN_LUA_VERSION}"
    --ini-output "${CMAKE_BINARY_DIR}/gnoblin-version.ini"
  BYPRODUCTS "${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
    "${CMAKE_BINARY_DIR}/gnoblin-version.ini")
add_dependencies(gnoblinctl gnoblinctl-identity)
target_compile_definitions(gnoblinctl PRIVATE
  GNOBLIN_NATIVE_CONTROL_API_MAJOR=${GNOBLIN_NATIVE_CONTROL_API_MAJOR}
  GNOBLIN_NATIVE_CONTROL_API_MINOR=${GNOBLIN_NATIVE_CONTROL_API_MINOR}
  "GNOBLIN_IDENTITY_FALLBACK=\"${CMAKE_BINARY_DIR}/gnoblinctl-identity.json\"")

set(GNOBLIN_BUILD_TYPE "debugoptimized" CACHE STRING "Meson build type")
set(GNOBLIN_DEVKIT "disabled" CACHE STRING "Build the Mutter devkit")
option(GNOBLIN_XWAYLAND "Support X11 applications through XWayland" ON)
option(GNOBLIN_VECTOR_CURSORS "Build the optional Adwaita vector cursor theme" OFF)
set(GNOBLIN_SOURCE_MODE "checkout" CACHE STRING "checkout or release-archive")
set(GNOBLIN_STAGE_ROOT "" CACHE PATH "Optional package staging root")
set_property(CACHE GNOBLIN_SOURCE_MODE PROPERTY STRINGS checkout release-archive)
set(GNOBLIN_JOBS "4" CACHE STRING "Parallel jobs inside each Meson build")

get_filename_component(GNOBLIN_PREFIX "${GNOBLIN_PREFIX}" ABSOLUTE)
target_compile_definitions(gnoblin PRIVATE
  "GNOBLIN_DEFAULT_COMPOSITOR=\"${GNOBLIN_PREFIX}/bin/gnoblin-mutter\"")
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

foreach(name IN ITEMS mutter xdg-desktop-portal-gnome)
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
  prepare-mutter-source prepare-xdg-desktop-portal-gnome-source)

function(gnoblin_component name)
  cmake_parse_arguments(COMPONENT "EXCLUDE_FROM_ALL" "" "" ${ARGN})
  set(exclude_from_all FALSE)
  if(COMPONENT_EXCLUDE_FROM_ALL)
    set(exclude_from_all TRUE)
  endif()
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
    DEPENDS "check-${name}-requirements" ${COMPONENT_UNPARSED_ARGUMENTS}
    EXCLUDE_FROM_ALL "${exclude_from_all}"
    USES_TERMINAL_BUILD TRUE)
endfunction()

gnoblin_component(mutter)

add_custom_target(gnoblin-session ALL
  COMMAND "${CMAKE_COMMAND}" -E env "GNOBLIN_LIBDIR=${GNOBLIN_LIBDIR}"
    "GNOBLIN_STAGE_ROOT=${GNOBLIN_STAGE_ROOT}"
    "GNOBLIN_VECTOR_CURSORS=${GNOBLIN_VECTOR_CURSORS}"
    "GNOBLIN_IDLE_BINARY=$<TARGET_FILE:gnoblin-idle>"
    "GNOBLINCTL_BINARY=$<TARGET_FILE:gnoblinctl>"
    "GNOBLIN_IDENTITY_FILE=${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
    "GNOBLIN_VERSION_METADATA_FILE=${CMAKE_BINARY_DIR}/gnoblin-version.ini"
    "GNOBLIN_BINARY=${GNOBLIN_BINARY}"
    "${PROJECT_SOURCE_DIR}/scripts/install-session.sh" "${GNOBLIN_PREFIX}"
  DEPENDS mutter gnoblin-idle gnoblinctl gnoblin
  USES_TERMINAL
  COMMENT "Install the standalone Gnoblin session")

# The GTK-based portal backend is a separate component. Keep a full-session
# target for package builds and users who explicitly want Gnoblin's backend,
# while the default source build remains usable with any installed backend.
gnoblin_component(xdg-desktop-portal-gnome EXCLUDE_FROM_ALL gnoblin-session)
add_custom_target(standalone-session DEPENDS gnoblin-session xdg-desktop-portal-gnome)
