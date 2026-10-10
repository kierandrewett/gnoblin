# Find the files that Gnoblin shares with the desktop.
#
# The desktop finds these files by directory, and Gnoblin owns them by name. A file with gnoblin in its name, under one of
# these directories, is public. The install step, the system layout and the login registration all read this one list from
# the finished prefix. A build that adds such a file needs no change to a script.
set(GNOBLIN_PUBLIC_DIRECTORIES
    share/wayland-sessions
    share/xdg-desktop-portal
    share/dbus-1/services
    lib/systemd/user
    share/man/man1)

# gnoblin_public_entries(OUT_VARIABLE PREFIX_DIRECTORY)
# Set OUT_VARIABLE to the public files under PREFIX_DIRECTORY, relative to it and sorted.
function(gnoblin_public_entries out prefix_dir)
    set(entries "")
    foreach(dir IN LISTS GNOBLIN_PUBLIC_DIRECTORIES)
        file(GLOB_RECURSE found LIST_DIRECTORIES false RELATIVE "${prefix_dir}" "${prefix_dir}/${dir}/*")
        foreach(entry IN LISTS found)
            get_filename_component(name "${entry}" NAME)
            if(name MATCHES "gnoblin")
                list(APPEND entries "${entry}")
            endif()
        endforeach()
    endforeach()
    list(SORT entries)
    set(${out} "${entries}" PARENT_SCOPE)
endfunction()
