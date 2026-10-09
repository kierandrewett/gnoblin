# Source preparation for the Meson components.
#
# This file holds the functions. cmake/source-step.cmake is the command line entry. Together they replace the shell
# scripts apply-patches, copy-overlay, subproject-state, ensure-release-subprojects and prepare-build-sources. The
# behaviour is the same, including the state files in build/subproject-state/.
#
# Rules that must stay true:
#   * A submodule is reset only when it is clean at the pinned tag, or when it matches the state the last
#     successful run recorded. Anything else stops the build.
#   * The reset target is always the pinned tag. Overlay files and patches are applied after the reset, in order.
#   * Submodule fetches go through scripts/checkout-submodules-with-retry.sh, which retries remote failures.

get_filename_component(GNOBLIN_ROOT "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)
set(GNOBLIN_SOURCE_PROJECTS mutter xdg-desktop-portal-gnome)
set(GNOBLIN_STATE_DIR "${GNOBLIN_ROOT}/build/subproject-state")

# Print one line on standard output, as a progress message.
function(gnoblin_say text)
  execute_process(COMMAND "${CMAKE_COMMAND}" -E echo "${text}")
endfunction()

# Run git and keep its output. Errors stay quiet: the caller decides what a failure means.
function(gnoblin_git out rc)
  execute_process(COMMAND git ${ARGN}
    OUTPUT_VARIABLE output ERROR_QUIET RESULT_VARIABLE result
    OUTPUT_STRIP_TRAILING_WHITESPACE)
  set(${out} "${output}" PARENT_SCOPE)
  set(${rc} "${result}" PARENT_SCOPE)
endfunction()

# Run a command that must succeed.
function(gnoblin_run)
  execute_process(COMMAND ${ARGN} COMMAND_ERROR_IS_FATAL ANY)
endfunction()

# Read a field from gnome-versions.json through the one script that owns the pins.
function(gnoblin_pin out project field)
  execute_process(COMMAND "${GNOBLIN_ROOT}/scripts/gnome-versions.py" get "${project}" "${field}"
    OUTPUT_VARIABLE value RESULT_VARIABLE result OUTPUT_STRIP_TRAILING_WHITESPACE)
  if(result)
    message(FATAL_ERROR "gnome-versions.py could not read ${field} for ${project}")
  endif()
  set(${out} "${value}" PARENT_SCOPE)
endfunction()

function(gnoblin_require_source_project project)
  if(NOT project IN_LIST GNOBLIN_SOURCE_PROJECTS)
    message(FATAL_ERROR "unsupported source project: ${project}")
  endif()
endfunction()

# --- Submodule state --------------------------------------------------------------------------------------------

# Set problem to why a state check cannot run, or to the empty string.
function(gnoblin_state_precheck problem project tag)
  set(text "")
  if(NOT IS_DIRECTORY "${GNOBLIN_ROOT}/subprojects/${project}")
    set(text "subproject not found: ${project}")
  else()
    gnoblin_git(ignored rc -C "${GNOBLIN_ROOT}/subprojects/${project}" rev-parse --verify "${tag}^{commit}")
    if(rc)
      set(text "subproject ${project} has no tag ${tag}; run 'git submodule update --init --recursive'")
    endif()
  endif()
  set(${problem} "${text}" PARENT_SCOPE)
endfunction()

# SHA-256 of: the HEAD commit, the binary diff against HEAD, and every untracked file name with its blob hash. This is
# the byte stream the old shell script hashed, so a state that script recorded still matches. The pieces go to files
# because a CMake string cannot hold the NUL bytes or a binary diff.
function(gnoblin_snapshot out project)
  set(subproject "${GNOBLIN_ROOT}/subprojects/${project}")
  string(RANDOM LENGTH 12 suffix)
  set(work "${GNOBLIN_STATE_DIR}/.snapshot.${suffix}")
  file(MAKE_DIRECTORY "${work}")
  execute_process(COMMAND git -C "${subproject}" rev-parse HEAD
    OUTPUT_FILE "${work}/head" COMMAND_ERROR_IS_FATAL ANY)
  execute_process(COMMAND git -C "${subproject}" diff --binary --no-ext-diff HEAD --
    OUTPUT_FILE "${work}/diff" COMMAND_ERROR_IS_FATAL ANY)
  execute_process(COMMAND git -c core.quotePath=false -C "${subproject}" ls-files --others --exclude-standard
      -- . ":(exclude)subprojects/.wraplock"
    OUTPUT_VARIABLE names COMMAND_ERROR_IS_FATAL ANY)
  set(pieces "${work}/head" "${work}/diff")
  set(index 0)
  while(NOT names STREQUAL "")
    string(FIND "${names}" "\n" cut)
    if(cut EQUAL -1)
      set(name "${names}")
      set(names "")
    else()
      string(SUBSTRING "${names}" 0 ${cut} name)
      math(EXPR rest "${cut} + 1")
      string(SUBSTRING "${names}" ${rest} -1 names)
    endif()
    if(name STREQUAL "")
      continue()
    endif()
    # Git quotes unusual names. Refuse them, so the check fails closed instead of hashing a wrong name.
    if(name MATCHES "^\"")
      file(REMOVE_RECURSE "${work}")
      message(FATAL_ERROR "refusing to hash an untracked file with an unusual name in ${project}: ${name}")
    endif()
    execute_process(COMMAND git -C "${subproject}" ls-files --others --exclude-standard -z -- ":(literal)${name}"
      OUTPUT_FILE "${work}/name${index}" COMMAND_ERROR_IS_FATAL ANY)
    execute_process(COMMAND git -C "${subproject}" hash-object -- "${name}"
      OUTPUT_FILE "${work}/hash${index}" COMMAND_ERROR_IS_FATAL ANY)
    list(APPEND pieces "${work}/name${index}" "${work}/hash${index}")
    math(EXPR index "${index} + 1")
  endwhile()
  execute_process(COMMAND "${CMAKE_COMMAND}" -E cat ${pieces}
    OUTPUT_FILE "${work}/all" COMMAND_ERROR_IS_FATAL ANY)
  file(SHA256 "${work}/all" digest)
  file(REMOVE_RECURSE "${work}")
  set(${out} "${digest}" PARENT_SCOPE)
endfunction()

function(gnoblin_worktree_status out project)
  gnoblin_git(status rc -C "${GNOBLIN_ROOT}/subprojects/${project}" status --porcelain --untracked-files=all
    -- . ":(exclude)subprojects/.wraplock")
  set(${out} "${status}" PARENT_SCOPE)
endfunction()

# A subproject is pristine when it has no changes and sits on the pinned tag. Mutter may also sit on the clean revision
# of the Gnoblin fork that the superproject pins.
function(gnoblin_is_pristine out project tag)
  set(subproject "${GNOBLIN_ROOT}/subprojects/${project}")
  set(${out} FALSE PARENT_SCOPE)
  gnoblin_worktree_status(status "${project}")
  if(NOT status STREQUAL "")
    return()
  endif()
  gnoblin_git(head rc -C "${subproject}" rev-parse HEAD)
  gnoblin_git(pinned rc -C "${subproject}" rev-parse "${tag}^{commit}")
  if(head STREQUAL pinned)
    set(${out} TRUE PARENT_SCOPE)
    return()
  endif()
  if(NOT project STREQUAL "mutter")
    return()
  endif()
  gnoblin_pin(expected_url mutter source-url)
  gnoblin_git(configured_url rc -C "${GNOBLIN_ROOT}" config -f "${GNOBLIN_ROOT}/.gitmodules"
    --get submodule.subprojects/mutter.url)
  gnoblin_git(origin_url rc -C "${subproject}" remote get-url origin)
  gnoblin_git(expected_commit rc -C "${GNOBLIN_ROOT}" rev-parse "HEAD:subprojects/mutter")
  if(configured_url STREQUAL expected_url AND origin_url STREQUAL expected_url AND head STREQUAL expected_commit)
    set(${out} TRUE PARENT_SCOPE)
  endif()
endfunction()

# Set ok to TRUE when a reset of the subproject is allowed. It is allowed when the checkout is pristine, or matches the
# state the last successful run recorded, or the caller forces it.
function(gnoblin_state_allows ok project tag)
  set(${ok} FALSE PARENT_SCOPE)
  gnoblin_state_precheck(problem "${project}" "${tag}")
  if(NOT problem STREQUAL "")
    return()
  endif()
  if("$ENV{GNOBLIN_FORCE_RESET}" STREQUAL "1")
    set(${ok} TRUE PARENT_SCOPE)
    return()
  endif()
  gnoblin_is_pristine(pristine "${project}" "${tag}")
  if(pristine)
    set(${ok} TRUE PARENT_SCOPE)
    return()
  endif()
  set(state_file "${GNOBLIN_STATE_DIR}/${project}.sha256")
  if(EXISTS "${state_file}")
    file(READ "${state_file}" recorded)
    string(STRIP "${recorded}" recorded)
    gnoblin_snapshot(current "${project}")
    if(recorded STREQUAL current)
      set(${ok} TRUE PARENT_SCOPE)
    endif()
  endif()
endfunction()

# Stop the build unless a reset of the subproject is allowed.
function(gnoblin_state_require project tag)
  gnoblin_state_precheck(problem "${project}" "${tag}")
  if(NOT problem STREQUAL "")
    message(FATAL_ERROR "${problem}")
  endif()
  if("$ENV{GNOBLIN_FORCE_RESET}" STREQUAL "1")
    message(NOTICE ">> WARNING: forcing destructive reset of ${project}")
    return()
  endif()
  gnoblin_state_allows(ok "${project}" "${tag}")
  if(NOT ok)
    gnoblin_worktree_status(status "${project}")
    message(FATAL_ERROR
      "refusing to reset subproject ${project}: checkout contains unrecognised work\n${status}\n"
      "Move source changes into gnoblin overlays/patches, or review them and rerun with\n"
      "GNOBLIN_FORCE_RESET=1 only when discarding this checkout is intentional.")
  endif()
endfunction()

function(gnoblin_state_record project tag)
  gnoblin_state_precheck(problem "${project}" "${tag}")
  if(NOT problem STREQUAL "")
    message(FATAL_ERROR "${problem}")
  endif()
  file(MAKE_DIRECTORY "${GNOBLIN_STATE_DIR}")
  gnoblin_snapshot(digest "${project}")
  set(state_file "${GNOBLIN_STATE_DIR}/${project}.sha256")
  file(WRITE "${state_file}.tmp" "${digest}\n")
  file(RENAME "${state_file}.tmp" "${state_file}")
endfunction()

# --- Overlay ----------------------------------------------------------------------------------------------------

# Copy, list or remove the overlay files that src/**/manifest maps into a subproject. Each manifest line is
# "<project> <source> <destination>". The source is relative to the manifest. A copied file is added to the git exclude
# file of a checkout, so the submodule stays clean. A release archive has no git metadata and needs no entry.
#
# action is copy, list or remove. For list, the destinations go to standard output, one per line.
function(gnoblin_overlay project subproject_dir action)
  gnoblin_require_source_project("${project}")
  if(NOT action MATCHES "^(copy|list|remove)$")
    message(FATAL_ERROR "unknown overlay action: ${action}")
  endif()
  file(GLOB_RECURSE manifests "${GNOBLIN_ROOT}/src/manifest")
  list(SORT manifests)
  set(count 0)
  set(listed "")
  foreach(manifest IN LISTS manifests)
    get_filename_component(feature_dir "${manifest}" DIRECTORY)
    file(READ "${manifest}" text)
    string(REGEX MATCHALL "[^\n]+" lines "${text}")
    foreach(line IN LISTS lines)
      string(REGEX MATCHALL "[^ \t]+" fields "${line}")
      list(LENGTH fields field_count)
      if(field_count EQUAL 0)
        continue()
      endif()
      list(GET fields 0 entry_project)
      if(entry_project MATCHES "^#" OR NOT entry_project STREQUAL project)
        continue()
      endif()
      set(source "")
      set(destination "")
      if(field_count GREATER 1)
        list(GET fields 1 source)
      endif()
      if(field_count GREATER 2)
        list(GET fields 2 destination)
      endif()
      if(source STREQUAL "" OR NOT EXISTS "${feature_dir}/${source}" OR IS_DIRECTORY "${feature_dir}/${source}")
        message(FATAL_ERROR "overlay: missing ${feature_dir}/${source}")
      endif()
      if(destination STREQUAL "" OR destination STREQUAL ".." OR destination MATCHES "^/"
          OR destination MATCHES "^\\.\\./" OR destination MATCHES "/\\.\\./" OR destination MATCHES "/\\.\\.$")
        message(FATAL_ERROR "overlay: invalid destination outside subproject: ${destination}")
      endif()
      if(action STREQUAL "list")
        string(APPEND listed "${destination}\n")
      elseif(action STREQUAL "remove")
        file(REMOVE "${subproject_dir}/${destination}")
        get_filename_component(parent "${destination}" DIRECTORY)
        if(NOT parent STREQUAL "" AND IS_DIRECTORY "${subproject_dir}/${parent}")
          file(GLOB children "${subproject_dir}/${parent}/*")
          if(NOT children)
            file(REMOVE_RECURSE "${subproject_dir}/${parent}")
          endif()
        endif()
        math(EXPR count "${count} + 1")
      else()
        get_filename_component(parent "${destination}" DIRECTORY)
        file(MAKE_DIRECTORY "${subproject_dir}/${parent}")
        file(COPY_FILE "${feature_dir}/${source}" "${subproject_dir}/${destination}")
        gnoblin_git(inside rc -C "${subproject_dir}" rev-parse --is-inside-work-tree)
        if(rc EQUAL 0)
          gnoblin_git(git_dir rc -C "${subproject_dir}" rev-parse --absolute-git-dir)
          set(exclude "${git_dir}/info/exclude")
          if(EXISTS "${exclude}")
            file(READ "${exclude}" excluded)
            string(FIND "\n${excluded}\n" "\n/${destination}\n" found)
            if(found EQUAL -1)
              file(APPEND "${exclude}" "/${destination}\n")
            endif()
          endif()
        endif()
        math(EXPR count "${count} + 1")
      endif()
    endforeach()
  endforeach()
  if(action STREQUAL "list")
    execute_process(COMMAND "${CMAKE_COMMAND}" -E echo_append "${listed}")
  elseif(action STREQUAL "copy")
    gnoblin_say(">> copied ${count} overlay file(s) into ${project}")
  else()
    gnoblin_say(">> removed ${count} overlay file(s) from ${project}")
  endif()
endfunction()

# --- Patches ----------------------------------------------------------------------------------------------------

# Reset a submodule to its pinned tag, copy the overlay, and apply patches/<project>/ in name order. The run is
# idempotent: the reset comes first, so no state builds up.
function(gnoblin_apply_patches project)
  gnoblin_require_source_project("${project}")
  set(subproject "${GNOBLIN_ROOT}/subprojects/${project}")
  gnoblin_pin(tag "${project}" version)
  gnoblin_git(ignored rc -C "${subproject}" rev-parse --git-dir)
  if(rc)
    message(FATAL_ERROR "submodule ${project} not initialised; run 'git submodule update --init --recursive'")
  endif()

  gnoblin_state_require("${project}" "${tag}")
  gnoblin_run("${GNOBLIN_ROOT}/scripts/manage-patches.py" check)
  gnoblin_overlay("${project}" "${subproject}" remove)

  gnoblin_say(">> resetting ${project} to pristine tag ${tag}")
  execute_process(COMMAND git -C "${subproject}" am --abort OUTPUT_QUIET ERROR_QUIET)
  gnoblin_run(git -C "${subproject}" checkout -qf "${tag}")
  gnoblin_run(git -C "${subproject}" reset -q --hard "${tag}")
  gnoblin_run(git -C "${subproject}" clean -qfd)

  gnoblin_overlay("${project}" "${subproject}" copy)

  file(GLOB_RECURSE patches "${GNOBLIN_ROOT}/patches/${project}/*.patch")
  list(SORT patches)
  list(LENGTH patches patch_count)
  gnoblin_say(">> applying ${patch_count} patch(es) to ${project}")
  if(patch_count GREATER 0)
    foreach(patch IN LISTS patches)
      string(REPLACE "${GNOBLIN_ROOT}/" "" shown "${patch}")
      gnoblin_say("   ${shown}")
    endforeach()
    # Keep the patch author from each From: header. A clean build environment may have no committer, so supply a stable
    # build identity only when git cannot find one. The .patch files stay the source of truth. These commits are a
    # staging step before git archive and are never pushed.
    gnoblin_git(ignored rc -C "${subproject}" var GIT_COMMITTER_IDENT)
    if(rc EQUAL 0)
      gnoblin_run(git -C "${subproject}" am ${patches})
    else()
      gnoblin_run(git -C "${subproject}" -c "user.name=Gnoblin Build" -c "user.email=builds@gnoblin.invalid"
        am ${patches})
    endif()
  endif()

  gnoblin_state_record("${project}" "${tag}")
  gnoblin_git(short rc -C "${subproject}" rev-parse --short HEAD)
  gnoblin_git(ahead rc -C "${subproject}" rev-list --count "${tag}..HEAD")
  gnoblin_say(">> ${project} now at ${short} (${ahead} patches on top of ${tag})")
endfunction()

# --- Release pins -----------------------------------------------------------------------------------------------

# Check that each source submodule matches the public GNOME release tag. Nothing is overwritten.
function(gnoblin_ensure_release_subprojects)
  set(projects ${ARGN})
  if(NOT projects)
    set(projects ${GNOBLIN_SOURCE_PROJECTS})
  endif()
  foreach(project IN LISTS projects)
    if(NOT project IN_LIST GNOBLIN_SOURCE_PROJECTS)
      message(FATAL_ERROR "Unknown source project: ${project}")
    endif()
    set(subproject "${GNOBLIN_ROOT}/subprojects/${project}")
    gnoblin_pin(tag "${project}" version)
    gnoblin_git(ignored rc -C "${subproject}" rev-parse --git-dir)
    if(rc)
      message(FATAL_ERROR
        "subproject ${project} is not initialised; run 'git submodule update --init --recursive'")
    endif()

    # A checkout build writes the reviewed overlay and patch series into this worktree. Accept that exact recorded
    # state on later build stages. The state check rejects any edit outside the patch pipeline.
    gnoblin_state_allows(recorded "${project}" "${tag}")
    if(recorded)
      continue()
    endif()

    gnoblin_git(ignored rc -C "${subproject}" rev-parse --verify "${tag}^{commit}")
    if(rc)
      gnoblin_run(git -C "${subproject}" fetch --quiet --depth=1 origin "refs/tags/${tag}:refs/tags/${tag}")
    endif()
    gnoblin_git(expected rc -C "${subproject}" rev-parse "${tag}^{commit}")
    gnoblin_git(actual rc -C "${GNOBLIN_ROOT}" rev-parse "HEAD:subprojects/${project}")
    if(actual STREQUAL expected)
      continue()
    endif()

    if(project STREQUAL "mutter")
      gnoblin_pin(expected_url mutter source-url)
      gnoblin_git(configured_url rc -C "${GNOBLIN_ROOT}" config -f "${GNOBLIN_ROOT}/.gitmodules"
        --get submodule.subprojects/mutter.url)
      gnoblin_git(origin_url rc -C "${subproject}" remote get-url origin)
      gnoblin_git(checkout rc -C "${subproject}" rev-parse HEAD)
      gnoblin_worktree_status(status mutter)
      if(configured_url STREQUAL expected_url AND origin_url STREQUAL expected_url
          AND checkout STREQUAL actual AND status STREQUAL "")
        continue()
      endif()
      message(FATAL_ERROR "subproject ${project} is pinned at ${actual}, but release ${tag} names ${expected}\n"
        "Mutter must use either the GNOME release tag or the clean Gnoblin fork revision pinned by this checkout.")
    endif()
    message(FATAL_ERROR "subproject ${project} is pinned at ${actual}, but release ${tag} names ${expected}\n"
      "The superproject pin and release tag must agree; no checkout was overwritten.")
  endforeach()
endfunction()
