cmake_minimum_required(VERSION 3.24)
project(gnoblin-vr VERSION 0.1.0 LANGUAGES C)

include(CTest)
find_package(PkgConfig REQUIRED)
pkg_check_modules(GLIB REQUIRED IMPORTED_TARGET gio-2.0)
find_path(OPENXR_INCLUDE_DIR openxr/openxr.h REQUIRED)
find_library(OPENXR_LOADER_LIBRARY NAMES openxr_loader REQUIRED)
find_package(OpenGL REQUIRED)
find_package(X11 REQUIRED)
add_executable(gnoblin-vr src/gnoblin-vr.c src/gnoblin-vr-common.c src/gnoblin-vr-gl.c)
target_include_directories(gnoblin-vr PRIVATE "${OPENXR_INCLUDE_DIR}" src)
target_link_libraries(gnoblin-vr PRIVATE PkgConfig::GLIB "${OPENXR_LOADER_LIBRARY}" OpenGL::GL X11::X11 m)
install(TARGETS gnoblin-vr RUNTIME DESTINATION bin)
configure_file(systemd/gnoblin-vr.service.in gnoblin-vr.service @ONLY)
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/gnoblin-vr.service"
        DESTINATION lib/systemd/user)
install(FILES lua/gnoblin-vr.lua DESTINATION share/gnoblin-vr/lua)

if(BUILD_TESTING)
  find_package(Python3 REQUIRED COMPONENTS Interpreter)
  add_test(NAME gnoblin-vr-status-absent
    COMMAND "${Python3_EXECUTABLE}" "${CMAKE_CURRENT_SOURCE_DIR}/tests/vr-status-absent.test.py"
            "$<TARGET_FILE:gnoblin-vr>")
  set_tests_properties(gnoblin-vr-status-absent PROPERTIES TIMEOUT 15)
endif()
