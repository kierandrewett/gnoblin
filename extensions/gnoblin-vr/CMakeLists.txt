cmake_minimum_required(VERSION 3.24)
project(gnoblin-vr VERSION 0.1.0 LANGUAGES C)

include(CTest)
find_package(PkgConfig REQUIRED)
pkg_check_modules(GLIB REQUIRED IMPORTED_TARGET gio-2.0)
pkg_check_modules(LIBDRM IMPORTED_TARGET libdrm)
pkg_check_modules(LIBUDEV IMPORTED_TARGET libudev)
find_path(OPENXR_INCLUDE_DIR openxr/openxr.h REQUIRED)
find_library(OPENXR_LOADER_LIBRARY NAMES openxr_loader REQUIRED)
find_package(OpenGL REQUIRED)
find_package(X11 REQUIRED)
add_executable(gnoblin-vr src/gnoblin-vr.c src/gnoblin-vr-common.c src/gnoblin-vr-gl.c
  src/gnoblin-vr-devices.c src/gnoblin-vr-session.c)
target_include_directories(gnoblin-vr PRIVATE "${OPENXR_INCLUDE_DIR}" src)
target_link_libraries(gnoblin-vr PRIVATE PkgConfig::GLIB "${OPENXR_LOADER_LIBRARY}" OpenGL::GL X11::X11 m)
if(LIBDRM_FOUND)
  target_compile_definitions(gnoblin-vr PRIVATE HAVE_LIBDRM)
  target_link_libraries(gnoblin-vr PRIVATE PkgConfig::LIBDRM)
endif()
if(LIBUDEV_FOUND)
  target_compile_definitions(gnoblin-vr PRIVATE HAVE_LIBUDEV)
  target_link_libraries(gnoblin-vr PRIVATE PkgConfig::LIBUDEV)
endif()
install(TARGETS gnoblin-vr RUNTIME DESTINATION bin)
foreach(unit gnoblin-vr.service gnoblin-vr@.service gnoblin-vr-runtime@.service
        gnoblin-vr-watch.service)
  configure_file(systemd/${unit}.in ${unit} @ONLY)
  install(FILES "${CMAKE_CURRENT_BINARY_DIR}/${unit}" DESTINATION lib/systemd/user)
endforeach()
install(FILES lua/gnoblin-vr.lua lua/gnoblin-vr-shell.lua lua/gnoblin-vr-session.lua
        DESTINATION share/gnoblin-vr/lua)

if(BUILD_TESTING)
  find_package(Python3 REQUIRED COMPONENTS Interpreter)
  add_test(NAME gnoblin-vr-devices
    COMMAND "${Python3_EXECUTABLE}" "${CMAKE_CURRENT_SOURCE_DIR}/tests/vr-devices.test.py"
            "$<TARGET_FILE:gnoblin-vr>")
  set_tests_properties(gnoblin-vr-devices PROPERTIES TIMEOUT 30)
  add_test(NAME gnoblin-vr-status-absent
    COMMAND "${Python3_EXECUTABLE}" "${CMAKE_CURRENT_SOURCE_DIR}/tests/vr-status-absent.test.py"
            "$<TARGET_FILE:gnoblin-vr>")
  set_tests_properties(gnoblin-vr-status-absent PROPERTIES TIMEOUT 15)
endif()
