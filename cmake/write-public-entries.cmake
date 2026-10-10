# Write the list of public files into the prefix, so that "make install" can register exactly what was built.
#
# usage: cmake -DGNOBLIN_PREFIX=DIR [-DGNOBLIN_STAGE_ROOT=DIR] -P cmake/write-public-entries.cmake
include("${CMAKE_CURRENT_LIST_DIR}/public-entries.cmake")

set(prefix_dir "${GNOBLIN_STAGE_ROOT}${GNOBLIN_PREFIX}")
gnoblin_public_entries(entries "${prefix_dir}")
list(JOIN entries "\n" body)
file(MAKE_DIRECTORY "${prefix_dir}/share/gnoblin")
file(WRITE "${prefix_dir}/share/gnoblin/public-entries.txt" "${body}\n")
list(LENGTH entries count)
message(STATUS "[entries] ${count} public files")
