# One source-preparation step, run in script mode.
#
#   cmake -DACTION=<action> [-D<NAME>=<value> ...] -P cmake/source-step.cmake
#
# Actions and their arguments:
#   apply-patches        PROJECT                      Reset to the pinned tag, copy the overlay, apply patches/<PROJECT>/
#   overlay              PROJECT SOURCE_DIR MODE      MODE is copy, list or remove. SOURCE_DIR is the tree to change
#   state-check          PROJECT TAG                  Fail unless the submodule may be reset
#   state-record         PROJECT TAG                  Record the state of a patched submodule
#   ensure-release       [PROJECTS]                   Check the submodules against the release tags
#   prepare              SOURCE_MODE PROJECTS         Check out or unpack the pinned sources
#   component-sources    PROJECT BUILD_ROOT           Apply the patches for one component when its inputs changed
#
# PROJECTS is a list. Separate the items with semicolons.
cmake_minimum_required(VERSION 3.22)
include("${CMAKE_CURRENT_LIST_DIR}/source-lib.cmake")
include("${CMAKE_CURRENT_LIST_DIR}/source-prepare.cmake")

if(NOT DEFINED ACTION)
  message(FATAL_ERROR "ACTION is required; see the header of cmake/source-step.cmake")
endif()

function(gnoblin_require)
  foreach(name IN LISTS ARGN)
    if(NOT DEFINED ${name} OR "${${name}}" STREQUAL "")
      message(FATAL_ERROR "${ACTION}: ${name} is required")
    endif()
  endforeach()
endfunction()

if(ACTION STREQUAL "apply-patches")
  gnoblin_require(PROJECT)
  gnoblin_apply_patches("${PROJECT}")
elseif(ACTION STREQUAL "overlay")
  gnoblin_require(PROJECT SOURCE_DIR MODE)
  gnoblin_overlay("${PROJECT}" "${SOURCE_DIR}" "${MODE}")
elseif(ACTION STREQUAL "state-check")
  gnoblin_require(PROJECT TAG)
  gnoblin_state_require("${PROJECT}" "${TAG}")
elseif(ACTION STREQUAL "state-record")
  gnoblin_require(PROJECT TAG)
  gnoblin_state_record("${PROJECT}" "${TAG}")
elseif(ACTION STREQUAL "ensure-release")
  gnoblin_ensure_release_subprojects(${PROJECTS})
elseif(ACTION STREQUAL "prepare")
  gnoblin_require(SOURCE_MODE)
  gnoblin_prepare_sources("${SOURCE_MODE}" ${PROJECTS})
elseif(ACTION STREQUAL "component-sources")
  gnoblin_require(PROJECT BUILD_ROOT)
  gnoblin_component_sources("${PROJECT}" "${BUILD_ROOT}")
else()
  message(FATAL_ERROR "unknown ACTION: ${ACTION}")
endif()
