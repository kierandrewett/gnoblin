# Contributor commands for the standalone Gnoblin source build.

set shell := ["bash", "-uc"]

prefix := env_var_or_default("GNOBLIN_PREFIX", justfile_directory() / "install")
libdir := env_var_or_default("GNOBLIN_LIBDIR", "lib64")
dev_buildtype := env_var_or_default("GNOBLIN_BUILD_TYPE", "debugoptimized")
mutter_test_opts := "--prefix=" + prefix + " --libdir=" + libdir + " --buildtype=" + dev_buildtype + " -Ddevkit=enabled -Dtests=enabled -Dmutter_tests=true -Dclutter_tests=false -Dcogl_tests=false -Ddocs=false -Dprofiler=false -Dudev_dir=" + prefix + "/lib/udev"
mutter_test_suites := "--suite mutter:mutter/unit --suite mutter:mutter/wayland --suite mutter:mutter/backends/native"
mutter_focus_tests := "mutter:focus-default-window-globally-active-input mutter:click-to-focus-and-raise mutter:overview-focus mutter:sloppy-focus mutter:sloppy-focus-pointer-rest mutter:sloppy-focus-auto-raise mutter:popup-focus"
export GNOBLIN_PREFIX := prefix
export GNOBLIN_LIBDIR := libdir

_default:
    @just --list

# Fetch and prepare the pinned source trees.
setup:
    ./build.sh --target prepare-sources

# Build the standalone compositor, runtime, portal backend, and session files.
build-source:
    ./build.sh

# Register this prefix as a selectable login session.
register-session:
    ./build.sh --register-session

# Open a nested Gnoblin development session.
preview *TERMINAL:
    ./scripts/run-gnoblin-devkit.sh {{TERMINAL}}

# Check the native control connection from a nested development session.
test-preview:
    ./tests/test-gnoblin-devkit.sh

# Verify compositor-native reconstruction of client-drawn transparent corners.
test-window-csd:
    GNOBLIN_DEVKIT_EXEC='python3 {{justfile_directory()}}/tests/test-window-csd-reconstruction.py' ./scripts/run-gnoblin-devkit.sh

# Verify Lua window rules render native borders in the standalone session.
test-window-borders:
    GNOBLIN_DEVKIT_EXEC='python3 {{justfile_directory()}}/tests/test-window-native-borders.py' ./scripts/run-gnoblin-devkit.sh

# Verify repeated Lua window-rule reloads preserve a live compositor window.
test-window-rule-lifecycle:
    GNOBLIN_DEVKIT_CONFIG_SOURCE='{{justfile_directory()}}/tests/configs/window-rule-lifecycle' GNOBLIN_DEVKIT_EXEC='python3 {{justfile_directory()}}/tests/test-window-rule-lifecycle.py' ./scripts/run-gnoblin-devkit.sh

# Verify Lua replacement shadows and transparent-window compositing in the standalone session.
test-window-shadows:
    GNOBLIN_DEVKIT_EXEC='python3 {{justfile_directory()}}/tests/test-window-native-shadows.py' ./scripts/run-gnoblin-devkit.sh

# Install the published Fedora packages from COPR.
install-fedora:
    ./scripts/install-system.sh

# Run deterministic parser, runtime, packaging, and source checks.
check: verify-fast

# Run the native runtime and configuration test suites.
test-runtime: verify-runtime

# Build the source tree and run all checks that do not require a live login.
test-all: verify

# Run the release verification gate.
test-release: verify-release

# Run the real-host Mutter unit, Wayland, backend, and focus suites.
test-window-manager: test-mutter

# --- implementation recipes --------------------------------------------------

[private]
prepare-tarball-sources:
    for p in mutter xdg-desktop-portal-gnome; do ./scripts/list-tarball-sources.sh "$p" --prepare >/dev/null || exit; done

[private]
patch PROJ:
    ./scripts/apply-patches.sh {{PROJ}}

[private]
patch-all:
    for p in mutter xdg-desktop-portal-gnome; do ./scripts/apply-patches.sh "$p" || exit; done

[private]
reset PROJ:
    #!/usr/bin/env bash
    set -euo pipefail
    t="$(./scripts/gnome-versions.py get "{{PROJ}}" version)"
    ./scripts/subproject-state.sh check "{{PROJ}}" "$t"
    ./scripts/copy-overlay.sh "{{PROJ}}" "subprojects/{{PROJ}}" --remove-destinations
    git -C subprojects/{{PROJ}} am --abort 2>/dev/null || true
    git -C subprojects/{{PROJ}} checkout -qf "$t"
    git -C subprojects/{{PROJ}} reset -q --hard "$t"
    git -C subprojects/{{PROJ}} clean -qfd
    ./scripts/subproject-state.sh record "{{PROJ}}" "$t"
    echo "{{PROJ}} reset to $t"

[private]
reset-all:
    for p in mutter xdg-desktop-portal-gnome; do just reset "$p" || exit; done

[private]
build PROJ: (patch PROJ)
    ./build.sh --target {{PROJ}}

[private]
check-install-prefix:
    source ./src/tools/gnoblin-env.sh; gnoblin_env_validate_install_prefix "{{prefix}}"

[private]
tarball PROJ:
    ./scripts/make-tarball.sh {{PROJ}}

[private]
arch PROJ:
    @echo "Arch packaging is maintained in packaging/arch/README.md"

[private]
test-config:
    ./tests/test-config.sh

[private]
test-mutter: (patch "mutter")
    meson setup --reconfigure build/mutter-tests subprojects/mutter {{mutter_test_opts}} || meson setup build/mutter-tests subprojects/mutter {{mutter_test_opts}}
    meson compile -C build/mutter-tests
    count="$(meson test -C build/mutter-tests {{mutter_test_suites}} --list | sed '/^$/d' | wc -l)"; if [ "$count" -le 0 ]; then echo "FAIL: no Mutter tests selected"; exit 1; fi; echo ">> running $count Mutter unit/Wayland/native tests"
    meson test -C build/mutter-tests --no-rebuild --num-processes 1 --print-errorlogs {{mutter_test_suites}}
    focus_count="$(meson test -C build/mutter-tests {{mutter_focus_tests}} --list | sed '/^$/d' | wc -l)"; if [ "$focus_count" -le 0 ]; then echo "FAIL: no Mutter focus tests selected"; exit 1; fi; echo ">> running $focus_count Mutter focus/stacking tests"
    meson test -C build/mutter-tests --no-rebuild --num-processes 1 --print-errorlogs {{mutter_focus_tests}}

[private]
verify-fast:
    ./scripts/gnome-versions.py check
    ./scripts/manage-patches.py check
    for file in scripts/*.sh tests/*.sh src/tools/*.sh; do bash -n "$file" || exit; done
    tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT; PYTHONPYCACHEPREFIX="$tmp" python3 -m py_compile scripts/*.py tests/*.py
    ./tests/test-log-diagnostics.sh
    ./tests/test-secure-state.sh
    ./tests/test-rpm-sources.sh
    ./tests/config-seed.test.sh
    python3 tests/frame-renderer-policy.test.py
    python3 tests/session-environment.test.py
    python3 tests/package-isolation.test.py
    python3 tests/check-build-deps.test.py
    just test-config

[private]
verify-runtime:
    ctest --test-dir build/ninja --output-on-failure
    just test-config

[private]
verify: verify-fast
    cmake --build build/ninja --parallel
    just verify-runtime

[private]
verify-release:
    just verify
    just test-mutter

[private]
test: verify-fast
    @echo ">> deterministic checks passed; compositor and session runtime checks were not run."

[private]
clean:
    rm -rf build

[private]
srpm PROJECT SOURCES OUTPUT:
    ./scripts/build-srpm.sh "{{PROJECT}}" "{{SOURCES}}" "{{OUTPUT}}"

[private]
release-assets OUTPUT="dist/release" TAG="":
    ./scripts/build-release-assets.sh "{{OUTPUT}}" "{{TAG}}"

[private]
copr PROJECT MUTTER_SRPM PORTAL_SRPM GNOBLIN_SRPM:
    ./scripts/publish-copr.sh "{{PROJECT}}" "{{MUTTER_SRPM}}" "{{PORTAL_SRPM}}" "{{GNOBLIN_SRPM}}"

[private]
check-gnome-version:
    ./scripts/gnome-versions.py check --upstream

[private]
package-manifest COMMAND="check":
    ./scripts/sync-package-manifest.py {{COMMAND}}

lint *args:
    ./scripts/quality.sh lint {{args}}

format *args:
    ./scripts/quality.sh format {{args}}
