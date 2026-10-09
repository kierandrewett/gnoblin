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
add_executable(native-input-disposition-test
  tests/native-input-disposition-test.c
  src/native-control/gnoblin-input-disposition.c)
target_link_libraries(native-input-disposition-test PRIVATE PkgConfig::GNOBLIN_GIO)
add_test(NAME native-input-disposition COMMAND native-input-disposition-test)
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
  set(GNOBLIN_DEFAULT_CONFIG_SOURCE
    "${PROJECT_SOURCE_DIR}/src/data/default-config")
  file(GLOB_RECURSE GNOBLIN_DEFAULT_CONFIG_FILES CONFIGURE_DEPENDS
    "${GNOBLIN_DEFAULT_CONFIG_SOURCE}/*")
  set(GNOBLIN_DEFAULT_CONFIG_C
    "${CMAKE_CURRENT_BINARY_DIR}/generated/gnoblin-default-config.c")
  add_custom_command(
    OUTPUT "${GNOBLIN_DEFAULT_CONFIG_C}"
    COMMAND "${CMAKE_COMMAND}" -E make_directory
      "${CMAKE_CURRENT_BINARY_DIR}/generated"
    COMMAND "${Python3_EXECUTABLE}"
      "${PROJECT_SOURCE_DIR}/scripts/embed-config.py"
      "${GNOBLIN_DEFAULT_CONFIG_SOURCE}" "${GNOBLIN_DEFAULT_CONFIG_C}"
    DEPENDS ${GNOBLIN_DEFAULT_CONFIG_FILES}
      "${PROJECT_SOURCE_DIR}/scripts/embed-config.py"
    VERBATIM)
  set(GNOBLIN_CONFIG_SOURCES
    src/config/gnoblin-config.c
    "${GNOBLIN_DEFAULT_CONFIG_C}"
    src/native-control/gnoblin-input-config.c
    src/config/gnoblin-lua.c
    src/config/gnoblin-lua-pattern.c
    src/config/gnoblin-glob.c
    src/native-control/gnoblin-keybinding-catalog.c
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

endif()
add_executable(gnoblin-idle src/session/gnoblin-idle.c)
target_link_libraries(gnoblin-idle PRIVATE PkgConfig::GNOBLIN_CONTROL)
add_executable(gnoblinctl src/tools/gnoblinctl.c)
target_link_libraries(gnoblinctl PRIVATE
  gnoblin-config-core PkgConfig::GNOBLIN_CONTROL PkgConfig::GNOBLIN_LUA)
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
option(GNOBLIN_WITH_PORTAL "Build and report the GTK-based portal backend" ON)
add_custom_target(gnoblinctl-identity
  COMMAND "${Python3_EXECUTABLE}" "${PROJECT_SOURCE_DIR}/scripts/build-identity.py"
    "${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
    --lua-version "${GNOBLIN_LUA_VERSION}"
    $<$<NOT:$<BOOL:${GNOBLIN_WITH_PORTAL}>>:--without-portal>
    --ini-output "${CMAKE_BINARY_DIR}/gnoblin-version.ini"
  BYPRODUCTS "${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
    "${CMAKE_BINARY_DIR}/gnoblin-version.ini"
  COMMENT "Write the build identity")
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
set(GNOBLIN_LAYOUT "private" CACHE STRING
  "private: everything under GNOBLIN_PREFIX. system: also the public entries a package ships")
set_property(CACHE GNOBLIN_LAYOUT PROPERTY STRINGS private system)
set(GNOBLIN_SYSTEM_PREFIX "/usr" CACHE PATH "Where the system layout puts its public entries")
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
add_custom_target(prepare-gsettings-desktop-schemas-source
  COMMAND "${PROJECT_SOURCE_DIR}/scripts/prepare-build-sources.sh"
    "${GNOBLIN_SOURCE_MODE}" gsettings-desktop-schemas
  WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}"
  USES_TERMINAL
  COMMENT "Prepare pinned gsettings-desktop-schemas metadata source")
add_custom_target(prepare-sources DEPENDS
  prepare-mutter-source prepare-xdg-desktop-portal-gnome-source
  prepare-gsettings-desktop-schemas-source)

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

gnoblin_component(mutter prepare-gsettings-desktop-schemas-source)

add_custom_target(gnoblin-session
  COMMAND "${CMAKE_COMMAND}" -E env "GNOBLIN_LIBDIR=${GNOBLIN_LIBDIR}"
    "GNOBLIN_STAGE_ROOT=${GNOBLIN_STAGE_ROOT}"
    "GNOBLIN_VECTOR_CURSORS=${GNOBLIN_VECTOR_CURSORS}"
    "GNOBLIN_IDLE_BINARY=$<TARGET_FILE:gnoblin-idle>"
    "GNOBLINCTL_BINARY=$<TARGET_FILE:gnoblinctl>"
    "GNOBLIN_IDENTITY_FILE=${CMAKE_BINARY_DIR}/gnoblinctl-identity.json"
    "GNOBLIN_VERSION_METADATA_FILE=${CMAKE_BINARY_DIR}/gnoblin-version.ini"
    "${PROJECT_SOURCE_DIR}/scripts/install-session.sh" "${GNOBLIN_PREFIX}"
  DEPENDS mutter gnoblin-idle gnoblinctl
  USES_TERMINAL
  COMMENT "Install the standalone Gnoblin session")

# The GTK-based portal backend is a separate component, but is part of the
# default full-session build. Use gnoblin-session for the core without it.
gnoblin_component(xdg-desktop-portal-gnome EXCLUDE_FROM_ALL gnoblin-session)
add_custom_target(standalone-session ALL DEPENDS gnoblin-session xdg-desktop-portal-gnome)

# The system layout adds what a package ships outside the private prefix: command links, the session file, systemd
# units and the portal and polkit files. The step reads the finished private tree, so it runs after every component.
if(NOT GNOBLIN_LAYOUT MATCHES "^(private|system)$")
  message(FATAL_ERROR "GNOBLIN_LAYOUT must be private or system")
endif()
add_custom_target(gnoblin-system-layout
  COMMAND "${CMAKE_COMMAND}" -E env "GNOBLIN_STAGE_ROOT=${GNOBLIN_STAGE_ROOT}"
    "${PROJECT_SOURCE_DIR}/scripts/install-system-layout.sh" "${GNOBLIN_PREFIX}" "${GNOBLIN_SYSTEM_PREFIX}"
  USES_TERMINAL
  COMMENT "Install the system layout")
add_dependencies(gnoblin-system-layout gnoblin-session)
if(GNOBLIN_WITH_PORTAL)
  add_dependencies(gnoblin-system-layout xdg-desktop-portal-gnome)
endif()

# One target that builds every test program, plus gnoblinctl, which the CLI tests run. It collects the targets whose names
# end in -test, so a test added later is built without a change to any workflow. Run it, then ctest.
get_property(GNOBLIN_ALL_TARGETS DIRECTORY PROPERTY BUILDSYSTEM_TARGETS)
set(GNOBLIN_TEST_TARGETS gnoblinctl)
foreach(target IN LISTS GNOBLIN_ALL_TARGETS)
  if(target MATCHES "-test$")
    list(APPEND GNOBLIN_TEST_TARGETS ${target})
  endif()
endforeach()
add_custom_target(gnoblin-tests DEPENDS ${GNOBLIN_TEST_TARGETS})
