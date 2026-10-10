# Add the files a distribution package ships outside Gnoblin's private prefix.
#
# ./build.sh installs the whole runtime below one private prefix, for example /usr/lib/gnoblin. A system package also
# needs a few entries where the desktop looks for them: /usr/bin, the display manager session list, the systemd user
# unit directory, the portal configuration and the polkit actions. This script makes those entries from the private
# tree. Every package recipe gets the same layout from the build, and none of them repeats these steps.
#
# usage: cmake -DGNOBLIN_PRIVATE_PREFIX=DIR [-DGNOBLIN_SYSTEM_PREFIX=/usr] -P cmake/system-layout.cmake
#
#   GNOBLIN_PRIVATE_PREFIX   where the runtime is installed on the target system (for example /usr/lib/gnoblin)
#   GNOBLIN_SYSTEM_PREFIX    where public entries go on the target system (default /usr)
#
# Environment:
#   GNOBLIN_STAGE_ROOT       a root, like DESTDIR, under which both prefixes are written (default: none)
#   GNOBLIN_SYSCONFDIR       the configuration directory on the target system (default /etc)
#   GNOBLIN_LAYOUT_GEOCLUE   set to 1 to add the GeoClue agent authorisation, for a package that ships it separately
include("${CMAKE_CURRENT_LIST_DIR}/public-entries.cmake")

if(NOT GNOBLIN_SYSTEM_PREFIX)
    set(GNOBLIN_SYSTEM_PREFIX /usr)
endif()
set(root "${CMAKE_CURRENT_LIST_DIR}/..")
set(stage "$ENV{GNOBLIN_STAGE_ROOT}")
set(sysconfdir "/etc")
if(DEFINED ENV{GNOBLIN_SYSCONFDIR} AND NOT "$ENV{GNOBLIN_SYSCONFDIR}" STREQUAL "")
    set(sysconfdir "$ENV{GNOBLIN_SYSCONFDIR}")
endif()

string(REGEX REPLACE "/+$" "" private "${GNOBLIN_PRIVATE_PREFIX}")
string(REGEX REPLACE "/+$" "" system "${GNOBLIN_SYSTEM_PREFIX}")
if(NOT private MATCHES "^/")
    message(FATAL_ERROR "[layout] the private prefix must be an absolute path: ${GNOBLIN_PRIVATE_PREFIX}")
endif()
if(private STREQUAL system)
    message(FATAL_ERROR "[layout] the private prefix must differ from the system prefix")
endif()

set(private_dir "${stage}${private}")
set(system_dir "${stage}${system}")
if(NOT EXISTS "${private_dir}/bin/gnoblin")
    message(FATAL_ERROR "[layout] no Gnoblin runtime in ${private_dir}. Build it first with ./build.sh.")
endif()

set(mode_644 OWNER_READ OWNER_WRITE GROUP_READ WORLD_READ)

# publish(SOURCE DESTINATION_DIRECTORY): copy one file to a public place and report it.
function(publish source destination_dir)
    file(INSTALL "${source}" DESTINATION "${destination_dir}" PERMISSIONS ${mode_644} MESSAGE_NEVER)
    get_filename_component(name "${source}" NAME)
    string(REPLACE "${stage}" "" shown "${destination_dir}/${name}")
    message(STATUS "[layout] ${shown}")
endfunction()

file(MAKE_DIRECTORY "${system_dir}/bin")
file(GLOB tools "${private_dir}/bin/gnoblin*")
foreach(tool IN LISTS tools)
    get_filename_component(name "${tool}" NAME)
    file(REMOVE "${system_dir}/bin/${name}")
    file(CREATE_LINK "${private}/bin/${name}" "${system_dir}/bin/${name}" SYMBOLIC)
    message(STATUS "[layout] ${system}/bin/${name} -> ${private}/bin/${name}")
endforeach()

# Every public file the build made, found in the private tree.
gnoblin_public_entries(entries "${private_dir}")
foreach(entry IN LISTS entries)
    get_filename_component(entry_dir "${entry}" DIRECTORY)
    publish("${private_dir}/${entry}" "${system_dir}/${entry_dir}")
    # The session file starts the private compositor and names the Gnoblin desktop.
    if(entry MATCHES "^share/wayland-sessions/.*\\.desktop$")
        file(READ "${system_dir}/${entry}" desktop)
        string(REGEX REPLACE "(^|\n)Exec=[^\n]*" "\\1Exec=${private}/bin/gnoblin" desktop "${desktop}")
        string(REGEX REPLACE "(^|\n)DesktopNames=[^\n]*" "\\1DesktopNames=Gnoblin;" desktop "${desktop}")
        file(WRITE "${system_dir}/${entry}" "${desktop}")
    endif()
endforeach()

# Mutter's backlight helper ships a polkit action under a GNOME name. A system package must not clash with GNOME's own
# copy, so Gnoblin publishes the action under its own name.
set(backlight_policy share/polkit-1/actions/org.gnome.mutter.backlight-helper.policy)
if(EXISTS "${private_dir}/${backlight_policy}")
    set(renamed share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy)
    file(READ "${private_dir}/${backlight_policy}" policy)
    string(REPLACE "org.gnome.mutter.backlight-helper" "org.gnoblin.mutter.backlight-helper" policy "${policy}")
    file(WRITE "${system_dir}/${renamed}" "${policy}")
    file(CHMOD "${system_dir}/${renamed}" PERMISSIONS ${mode_644})
    message(STATUS "[layout] ${system}/${renamed}")
endif()

# The optional GeoClue agent authorisation. Only a recipe with a separate package for it asks for the file, because a
# package that does not list it would fail as having unpackaged files.
if("$ENV{GNOBLIN_LAYOUT_GEOCLUE}" STREQUAL "1")
    publish("${root}/packaging/geoclue/50-gnoblin.conf" "${stage}${sysconfdir}/geoclue/conf.d")
endif()

# The schema cache is generated on the target, after the package is installed, so the package must not carry one.
file(REMOVE "${private_dir}/share/glib-2.0/schemas/gschemas.compiled")
