# Build and test the patched Mutter, GNOME Shell and Gnoblin session.
# Owned changes live in patches/ and src/. Upstream sources live in subprojects/.

set shell := ["bash", "-uc"]

# Patched subprojects built by `just build-local`.
patch_projects := "mutter gnome-shell"
rpm_projects := "mutter gnome-shell"

# Local development layout. Override both together for distro-style prefixes,
# for example: GNOBLIN_PREFIX=/tmp/gnoblin GNOBLIN_LIBDIR=lib just build-local.
prefix := env_var_or_default("GNOBLIN_PREFIX", justfile_directory() / "install")
libdir := env_var_or_default("GNOBLIN_LIBDIR", "lib64")
# Retain debug symbols while optimising the compositor used by local sessions.
# Set GNOBLIN_BUILD_TYPE=debug for an unoptimised debugger build.
dev_buildtype := env_var_or_default("GNOBLIN_BUILD_TYPE", "debugoptimized")
export GNOBLIN_PREFIX := prefix
export GNOBLIN_LIBDIR := libdir

_default:
    @just --list

# The public interface. These are the commands contributors and source-build
# users should need. The implementation recipes below retain their precise
# names for CI and focused debugging, but are intentionally hidden from
# `just --list`.

# Fetch the pinned sources needed for a local build.
setup: init

# Build Gnoblin into ./install after private dependencies are available.
build-source: build-local

# Open Gnoblin in a nested window on the current Wayland desktop.
preview *TERMINAL:
    just gnome-devkit {{TERMINAL}}

# Fuzz real Wayland window lifetimes in a private Gnoblin session. Supply a seed
# to make a run reproducible, for example `just fuzz-lifecycle SEED=1738`.
fuzz-lifecycle SEED="random" STEPS="300":
    if [ "{{SEED}}" = random ]; then ./tests/window-lifecycle-fuzz.py --steps "{{STEPS}}"; else ./tests/window-lifecycle-fuzz.py --seed "{{SEED}}" --steps "{{STEPS}}"; fi

# Add this source build as a selectable login session.
register-session: dev-session-register

# Install the published Fedora package from COPR.
install-fedora:
    ./scripts/install-system.sh

# Run quick deterministic checks. This does not start a compositor.
check: verify-fast

# Test the installed compositor in isolated headless sessions.
test-session: verify-installed-headless

# Build the current source, then run all headless checks.
test-all: verify

# Run the release gate, including real-host Mutter tests and Fedora packages.
test-release: verify-release

# Test a nested preview session and its control connection.
test-preview: gnome-devkit-verify

# Test compositor startup and advertised Wayland protocols.
test-startup: gnome-verify

# Test that ordinary GNOME stays separate from Gnoblin.
test-stock-gnome: gnome-stock-protocol-isolation-verify

# Test the public control protocol.
test-control-api: gnome-dbus-verify

# Test the protocol contracts exposed to shell clients.
test-protocols: gnome-protocol-boundaries-verify

# Test Lua configuration loading and live reload.
test-configuration: gnome-config-verify

# Test that Gnoblin leaves native desktop chrome to an external shell.
test-native-chrome: gnome-native-chrome-verify

# Test the user scripting lifecycle.
test-scripting: gnome-scripting-verify

# Test notification ownership can move between Gnoblin and another daemon.
test-notifications: gnome-notifications-verify

# Test protocol visibility follows the configuration.
test-protocol-gating: gnome-protocol-gating-verify

# Test the Gnoblin developer console.
test-developer-console: gnome-developer-console-verify

# Test layer-shell resize and animation behaviour.
test-layer-animations: gnome-layer-animation-verify

# Test effects applied to ordinary windows.
test-window-effects: gnome-window-effects-verify

# Test desktop recovery controls with a real shell client.
test-desktop-recovery: gnome-desktop-recovery-verify

# Test dock minimisation against a real Quickshell dock.
test-dock-minimise: gnome-minimize-target-verify

# Test the real-host Mutter window-management suite.
test-window-manager: test-mutter

# Build a Debian or Ubuntu package in a disposable container.
package-deb: deb

# --- implementation recipes --------------------------------------------------

# Initialise / update the pinned source checkouts and mandatory Meson wraps.
[private]
init:
    git submodule sync --recursive
    git submodule update --init --recursive
    ./scripts/ensure-release-subprojects.sh
    just prepare-tarball-sources
    @echo "mutter               -> $(git -C subprojects/mutter               describe --tags --always)"
    @echo "gnome-shell          -> $(git -C subprojects/gnome-shell          describe --tags --always)"

# Materialise the pinned Meson subprojects required by no-download RPM builds.
[private]
prepare-tarball-sources:
    for p in {{rpm_projects}}; do ./scripts/list-tarball-sources.sh "$p" --prepare >/dev/null || exit; done

# Apply the patch series to a subproject (resets it to the pinned tag first).
[private]
patch PROJ:
    ./scripts/apply-patches.sh {{PROJ}}

# Apply patches to every subproject.
[private]
patch-all:
    for p in {{patch_projects}}; do ./scripts/apply-patches.sh "$p" || exit; done

# Reset a subproject back to its pristine pinned tag.
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
    for p in {{patch_projects}}; do just reset "$p" || exit; done

# Configure + compile a subproject with meson into build/<proj> (dev build).
[private]
build PROJ: (patch PROJ)
    if [ "{{PROJ}}" = gnome-shell ]; then options=(-Dextensions_tool=false); else options=(); fi; meson setup --reconfigure build/{{PROJ}} subprojects/{{PROJ}} --buildtype={{dev_buildtype}} "${options[@]}" || meson setup build/{{PROJ}} subprojects/{{PROJ}} --buildtype={{dev_buildtype}} "${options[@]}"
    meson compile -C build/{{PROJ}}

# --- dev stack: build the whole gnoblin stack into ./install and run it ------
#
#   just build-local    build+install patched mutter + patched gnome-shell + session
#                       data into ./install
#   just gnome-verify   headless: boot gnome-shell in gnoblin mode, check layer-shell
#   just gnome-dbus-verify  headless: org.gnoblin.* control protocol round-trip
#
devkit := env_var_or_default("GNOBLIN_DEVKIT", "enabled")
mutter_dev_opts := "--prefix=" + prefix + " --libdir=" + libdir + " --buildtype=" + dev_buildtype + " -Ddevkit=" + devkit + " -Dtests=disabled -Ddocs=false -Dprofiler=false -Dudev_dir=" + prefix + "/lib/udev"
mutter_test_opts := "--prefix=" + prefix + " --libdir=" + libdir + " -Ddevkit=enabled -Dtests=enabled -Dmutter_tests=true -Dclutter_tests=false -Dcogl_tests=false -Ddocs=false -Dprofiler=false -Dudev_dir=" + prefix + "/lib/udev"
mutter_test_suites := "--suite mutter:mutter/unit --suite mutter:mutter/wayland --suite mutter:mutter/backends/native"
mutter_focus_tests := "mutter:focus-default-window-globally-active-input mutter:click-to-focus-and-raise mutter:overview-focus mutter:sloppy-focus mutter:sloppy-focus-pointer-rest mutter:sloppy-focus-auto-raise mutter:popup-focus"
mutter_test_run_opts := "--no-rebuild --num-processes 1 --print-errorlogs"
gnome_shell_dev_opts := "--prefix=" + prefix + " --libdir=" + libdir + " --buildtype=" + dev_buildtype + " -Dextensions_tool=false -Dtests=false -Dman=false -Dgtk_doc=false"
private_pkg_config_path := prefix + "/" + libdir + "/pkgconfig:" + prefix + "/share/pkgconfig:" + env_var_or_default("PKG_CONFIG_PATH", "")
private_gir_path := prefix + "/share/gir-1.0:" + env_var_or_default("GI_GIR_PATH", "")
private_typelib_path := prefix + "/" + libdir + "/girepository-1.0:" + env_var_or_default("GI_TYPELIB_PATH", "")

# Build + install the pinned GNOME schemas into the private development prefix.
# Mutter 51 consumes schema enums while configuring, so this must precede Mutter
# even when the host already has an older, distro-supported GNOME installation.
[private]
dev-schemas: check-install-prefix
    #!/usr/bin/env bash
    set -euo pipefail
    sources="build/release-sources"
    source_dir="build/gsettings-desktop-schemas-source"
    build_dir="build/gsettings-desktop-schemas"
    archive="$(./scripts/make-tarball.sh gsettings-desktop-schemas "$sources")"
    rm -rf -- "$source_dir" "$build_dir"
    mkdir -p -- "$source_dir"
    tar -xf "$archive" -C "$source_dir" --strip-components=1
    meson setup "$build_dir" "$source_dir" \
      --prefix={{prefix}} --libdir={{libdir}} --buildtype={{dev_buildtype}}
    meson install -C "$build_dir"

# Build + install patched mutter (incl. the Mutter Devkit viewer) into ./install.
[private]
dev-mutter: dev-schemas check-install-prefix (patch "mutter")
    PKG_CONFIG_PATH={{private_pkg_config_path}} GI_GIR_PATH={{private_gir_path}} GI_TYPELIB_PATH={{private_typelib_path}} meson setup --wipe build/mutter subprojects/mutter {{mutter_dev_opts}} || PKG_CONFIG_PATH={{private_pkg_config_path}} GI_GIR_PATH={{private_gir_path}} GI_TYPELIB_PATH={{private_typelib_path}} meson setup build/mutter subprojects/mutter {{mutter_dev_opts}}
    PKG_CONFIG_PATH={{private_pkg_config_path}} GI_GIR_PATH={{private_gir_path}} GI_TYPELIB_PATH={{private_typelib_path}} meson install -C build/mutter

# Build + install patched gnome-shell against the freshly built mutter in ./install.
# gnome-shell is the compositor+shell again; its stock UI (panel/overview/dash) is
# stripped via the `gnoblin` session mode + a minimal native-topbar patch, and its
# subsystems are toggled live over org.gnoblin.* — bring-your-own chrome draws the UI.
[private]
dev-gnome-shell: dev-mutter (patch "gnome-shell")
    # ALWAYS build clean: `patch gnome-shell` resets the submodule (git clean/checkout)
    # and re-copies the overlay every run, which resets source mtimes underneath the
    # build dir. Reusing it yields a half-stale libshell/libst (observed: duplicate
    # g_boxed_type registration → GJS boxed-prototype crash at boot). A fresh build dir
    # is the only reliably-correct option here.
    rm -rf build/gnome-shell
    PKG_CONFIG_PATH={{private_pkg_config_path}} GI_GIR_PATH={{private_gir_path}} GI_TYPELIB_PATH={{private_typelib_path}}:{{prefix}}/{{libdir}}/mutter-51 meson setup build/gnome-shell subprojects/gnome-shell {{gnome_shell_dev_opts}}
    PKG_CONFIG_PATH={{private_pkg_config_path}} GI_GIR_PATH={{private_gir_path}} GI_TYPELIB_PATH={{private_typelib_path}}:{{prefix}}/{{libdir}}/mutter-51 meson install -C build/gnome-shell
    rm -f {{prefix}}/lib/systemd/user/org.gnome.Shell-disable-extensions.service

# --- optional: unattended screen-share portal backend -----------------------
#
# xdg-desktop-portal-gnome is the org.freedesktop.impl.portal.desktop.gnome
# backend that shows the ScreenCast source-picker and RemoteDesktop consent
# dialogs. Gnoblin can remember the exact approved monitor, input-device, and
# clipboard capabilities for a verified requester. Grants are scoped by portal
# kind under $XDG_DATA_HOME/gnoblin/portal-grants/ and are written only after
# the session starts successfully. List/revoke them with `gnoblinctl
# portal-grants` and `gnoblinctl revoke-grant <kind> <id>`. It is not part of
# `just build-local`; build it explicitly:
#
#   just dev-portal
#
# then (re)start the backend so it owns the impl portal, e.g.:
#
#   ./install/libexec/xdg-desktop-portal-gnome -r
#
portal_dev_opts := "--prefix=" + prefix + " --libdir=" + libdir

# Build + install the patched xdg-desktop-portal-gnome backend into ./install.
[private]
dev-portal: check-install-prefix (patch "xdg-desktop-portal-gnome")
    meson setup --reconfigure build/xdg-desktop-portal-gnome subprojects/xdg-desktop-portal-gnome {{portal_dev_opts}} || meson setup build/xdg-desktop-portal-gnome subprojects/xdg-desktop-portal-gnome {{portal_dev_opts}}
    meson install -C build/xdg-desktop-portal-gnome

# Build the whole gnoblin stack (patched mutter + patched gnome-shell) into ./install.
[private]
build-local: dev-gnome-shell dev-session
    @echo ">> gnoblin stack (mutter + gnome-shell) installed in {{prefix}} — run 'just gnome-verify'"

# Install the gnoblin session data (session mode, gnome-session, .desktop) into ./install.
[private]
dev-session:
    ./scripts/install-session.sh {{prefix}}

# Reject prefixes that would overwrite an existing GNOME installation.
[private]
check-install-prefix:
    source ./src/tools/gnoblin-env.sh; gnoblin_env_validate_install_prefix "{{prefix}}"

# Register the gnoblin session with your live systemd --user instance (links
# org.gnoblin.Shell.target/@wayland.service -- gnoblin-specific unit names,
# does NOT touch org.gnome.Shell*) and print the (root) command to make
# "Gnoblin" appear at your login manager's session picker. NOT run by
# `just build-local`/`dev-session` -- it's the one step that touches state outside
# ./install. See docs/installation.md.
[private]
dev-session-register:
    ./scripts/register-session.sh {{prefix}}

# Install Gnoblin from the official COPR alongside the system's GNOME packages.
# The local RPM path is an explicit maintainer escape hatch only.
#   just install-session            # prompts, shows the transaction
#   just install-session dry        # validate + print only, changes nothing
#   just install-session yes        # no prompt
#   just install-session reinstall  # re-apply a rebuild at the same version
#   just install-session local       # maintainer-only local RPM install
# PRODUCTION: install the official COPR packages onto THIS host.
[private]
install-session MODE="":
    ./scripts/install-system.sh {{ if MODE == "dry" { "--dry-run" } else if MODE == "yes" { "--yes" } else if MODE == "reinstall" { "--reinstall" } else if MODE == "local" { "--local-rpms" } else { "" } }}

# Devkit: open a VISIBLE nested gnoblin session (a window in your current Wayland
# session) + a terminal already wired to it — so you can launch your own chrome
# against gnoblin without vendoring anything here. In the terminal, run e.g.
#   qs -p ~/dev/kobel-shell
# and your bar appears inside the nested gnoblin. Optional arg picks the terminal.
#   just gnome-devkit          # foot/kitty/alacritty auto-detected
#   just gnome-devkit kitty
[private]
gnome-devkit *TERMINAL:
    ./scripts/run-gnome-devkit.sh {{TERMINAL}}

# Headless regression test for the devkit: boot a nested gnoblin and confirm the
# spawned-terminal env (isolated bus + gnoblinctl) can drive org.gnoblin.Shell.
[private]
gnome-devkit-verify:
    ./tests/test-gnome-devkit.sh

# Headless: boot patched gnome-shell in the `gnoblin` session mode and verify it
# starts, advertises wlr-layer-shell, applies Gnoblin-only panel and extension
# policy, and keeps privileged Shell APIs restricted.
[private]
gnome-verify:
    GNOBLIN_EXPECT_EXTENSION_SCOPE=1 GNOBLIN_TEST_EXTENSION_ROOT="{{justfile_directory()}}/tests/shell-extensions" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-shell-security-policy.py" ./scripts/run-gnome-shell.sh

# Headless: boot stock GNOME mode from the patched packages and prove Gnoblin's
# protocols and control API remain unavailable while the native panel,
# extension validation, and notification ownership keep upstream behaviour.
[private]
gnome-stock-protocol-isolation-verify:
    ./tests/test-stock-protocol-isolation.sh

# Headless: exercise the org.gnoblin.* control protocol end-to-end over D-Bus
# (Ping / GetVersion / Reload → soft in-process reload).
[private]
gnome-dbus-verify:
    ./tests/test-gnome-dbus.sh

# Headless: performance smoke test — idle memory budget + growth, soft-reload
# leak bound and window-churn leak bound. Real-session follow-up is tracked in
# TODO.md and the real-hardware verification guide.
[private]
perf-smoke:
    ./tests/perf-smoke.sh

# Headless: verify external chrome owns the removed native desktop UI.
[private]
gnome-native-chrome-verify:
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-native-chrome.py" ./scripts/run-gnome-shell.sh

# Headless: verify the Gnoblin developer console replacement, evaluator and
# lock/unlock lifecycle against the installed patched Shell.
[private]
gnome-developer-console-verify:
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-developer-console.py" ./scripts/run-gnome-shell.sh

# Headless: real recovery-menu/panel clicks, terminal launch and layer-client
# failure. Requires foot and a Quickshell build matching the host Qt.
[private]
gnome-desktop-recovery-verify:
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_UNSAFE_MODE=1 QS_TEST_BIN="${QS_TEST_BIN:-qs}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-desktop-recovery.py" ./scripts/run-gnome-shell.sh

# Headless: prove Lua configuration watching, named autostart and live window behaviour.
[private]
gnome-config-verify:
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-live-shell-config.py" ./scripts/run-gnome-shell.sh

# Optional real Quickshell dock test; requires qs and Python GTK 4 bindings.
[private]
gnome-minimize-target-verify:
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-minimize-target.py" ./scripts/run-gnome-shell.sh

# Headless: prove the GJS user-scripting layer — drop a script, edit it, reload
# via org.gnoblin.*, confirm the new code ran.
[private]
gnome-scripting-verify:
    ./tests/test-scripting.sh

# Headless: prove the `notifications` toggle — gnoblin releases org.freedesktop.
# Notifications when disabled (so an external daemon can own it) and reclaims it.
[private]
gnome-notifications-verify:
    ./tests/test-notifications-toggle.sh

# Headless: compile black-box clients from the owned protocol XML and exercise
# registry binding plus clean disconnect against the running compositor.
[private]
gnome-protocol-boundaries-verify:
    ./tests/test-protocol-boundaries.sh

# Headless: prove Lua protocol gating — disabling wlr-layer-shell in the config
# stops zwlr_layer_shell_v1 being advertised.
[private]
gnome-protocol-gating-verify:
    ./tests/test-protocol-gating.sh

# Produce a patched release tarball in ~/rpmbuild/SOURCES.
[private]
tarball PROJ:
    ./scripts/make-tarball.sh {{PROJ}}

# Build a binary RPM (Fedora). Patches are pre-applied in the tarball.
[private]
rpm PROJ:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="packaging/rpm/{{PROJ}}.spec"
    # Check gnoblin-* build deps BEFORE the tarball step, which resets the
    # submodule and re-applies the whole patch series. rpmbuild only checks
    # BuildRequires after all that work is already done, so a missing
    # gnoblin-mutter-devel otherwise costs a full patch+archive cycle first.
    missing=()
    while read -r dep; do
        rpm -q "$dep" >/dev/null 2>&1 || missing+=("$dep")
    done < <(sed -n 's/^BuildRequires:[[:space:]]*\(gnoblin-[A-Za-z0-9._+-]*\).*/\1/p' "$spec")
    if [ "${#missing[@]}" -gt 0 ]; then
        echo "gnoblin-{{PROJ}} needs these installed to build against:" >&2
        printf '     %s\n' "${missing[@]}" >&2
        echo >&2
        echo "They are gnoblin's own packages, so build and install them first:" >&2
        echo "     just rpm mutter" >&2
        version="$(./scripts/gnome-versions.py get mutter version)"; \
        echo "     sudo dnf install ~/rpmbuild/RPMS/*/gnoblin-mutter-$version-*.rpm ~/rpmbuild/RPMS/*/gnoblin-mutter-devel-$version-*.rpm" >&2
        exit 1
    fi
    just tarball {{PROJ}}
    rpmbuild -bb "$spec"

# Build both packages. Shell requires the private gnoblin-mutter-devel package
# to be installed first; see docs/installation.md for the initial build.
[private]
rpm-all:
    for p in {{rpm_projects}}; do just rpm "$p" || exit; done
    rpmbuild -bb packaging/rpm/gnoblin.spec

# Package a private runtime in a prepared Debian/Ubuntu build container.
[private]
deb:
    ./scripts/build-deb.sh
[private]
arch PROJ:
    @echo "Arch packaging is planned — see packaging/arch/README.md"

# --- tests (see scripts/ and the mutter in-tree suite) -----------------------

# Tier 1: logic test for the shared C config parser (src/config), no display.
[private]
test-config:
    ./tests/test-config.sh

test-touchpad-gestures:
    npm run test:touchpad-gestures

# Best of 3, fails past BOOT_BUDGET_MS (default 1350). This catches startup
# regressions in the patched shell; the script documents the headless test
# boundary and the real-hardware follow-up.
# Boot-time budget: headless boot, compositor start -> "GNOME Shell started".
[private]
test-boot-time:
    ./tests/test-boot-time.sh

# Builds a minimal shm layer-shell client and times process start -> first
# frame inside a headless session. gnoblin's chrome is all layer-shell, so this
# is the latency the desktop's responsiveness actually rides on. Reports only
# by default; set LAYER_BUDGET_MS to make it a gate.
# Layer-shell chrome latency: how fast a bar gets its first pixel up.
[private]
test-layer-latency:
    ./tests/test-layer-latency.sh

# Tier 2: mutter in-tree headless functional tests. Run serially: these tests
# each boot a headless compositor with virtual input/monitors, and parallel Meson
# scheduling can starve a test long enough to trip its 60s timeout.
#
# NEEDS A REAL ENVIRONMENT: the native/Wayland backend tests boot a compositor that
# monitors an ICC profile dir; in sandboxes lacking a local file monitor (inotify)
# they all bail with "Unable to find default local file monitor type" (exit 251) —
# environmental, NOT a gnoblin regression (the unit tests, which need no backend,
# pass; the ref-image tests even log "Image matched" before bailing). Run on real HW.
[private]
test-mutter: (patch "mutter")
    meson setup --reconfigure build/mutter-tests subprojects/mutter {{mutter_test_opts}} || meson setup build/mutter-tests subprojects/mutter {{mutter_test_opts}}
    meson compile -C build/mutter-tests
    count="$(meson test -C build/mutter-tests {{mutter_test_suites}} --list | sed '/^$/d' | wc -l)"; if [ "$count" -le 0 ]; then echo "FAIL: no Mutter tests selected"; exit 1; fi; echo ">> running $count Mutter unit/Wayland/native tests"
    meson test -C build/mutter-tests {{mutter_test_run_opts}} {{mutter_test_suites}}
    focus_count="$(meson test -C build/mutter-tests {{mutter_focus_tests}} --list | sed '/^$/d' | wc -l)"; if [ "$focus_count" -le 0 ]; then echo "FAIL: no Mutter focus tests selected"; exit 1; fi; echo ">> running $focus_count Mutter focus/stacking tests"
    meson test -C build/mutter-tests {{mutter_test_run_opts}} {{mutter_focus_tests}}

# Fast deterministic checks: syntax, parser behaviour, secure state publication,
# and RPM sidecar-source completeness. Does not boot a compositor.
[private]
verify-fast:
    ./scripts/gnome-versions.py check
    for file in scripts/*.sh tests/*.sh src/tools/*.sh src/tools/gnoblin-session src/tools/gnoblin-shell-service; do bash -n "$file" || exit; done
    tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT; PYTHONPYCACHEPREFIX="$tmp" python3 -m py_compile scripts/*.py tests/*.py tests/e2e/*.py src/tools/gnoblinctl
    ./tests/test-log-diagnostics.sh
    ./tests/test-secure-state.sh
    ./tests/test-rpm-sources.sh
    python3 tests/frame-renderer-policy.test.py
    python3 tests/session-environment.test.py
    python3 tests/package-isolation.test.py
    python3 tests/build-deps.test.py
    python3 tests/private-deps.test.py
    python3 tests/window-lifecycle-fuzz.test.py
    python3 tests/e2e/app-catalog.test.py
    just test-config

# Every isolated headless integration check against an existing ./install.
# This intentionally reuses installed binaries; use `verify-headless` to build.
# Run serially because each test starts GNOME Shell and shared host services.
[private]
verify-installed-headless:
    just gnome-verify
    just gnome-stock-protocol-isolation-verify
    just gnome-protocol-boundaries-verify
    just gnome-dbus-verify
    just gnome-native-chrome-verify
    just gnome-developer-console-verify
    just gnome-config-verify
    just gnome-scripting-verify
    just gnome-notifications-verify
    just gnome-protocol-gating-verify
    just gnome-devkit-verify

# Build the current source and patch set before running headless integration.
[private]
verify-headless: build-local
    just verify-installed-headless

# Default local gate: deterministic checks plus the complete headless suite.
[private]
verify:
    just verify-fast
    just verify-headless

# Release gate: local verification, Mutter's real-host suite, then both RPMs.
[private]
verify-release:
    just verify
    just test-mutter
    just rpm-all

# Compatibility alias. `test` is deliberately fast; use `verify` for headless
# integration and `verify-release` for the real-host and packaging gates.
[private]
test: verify-fast
    @echo ">> fast checks passed; compositor integration was not run."
    @echo ">> run 'just verify' or 'just verify-release' for broader gates."

[private]
clean:
    rm -rf build

# Isolated layer-shell animation and alpha-masked blur regression tests.
[private]
gnome-layer-animation-verify:
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-layer-resize.py" ./scripts/run-gnome-shell.sh
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-layer-animation.py" ./scripts/run-gnome-shell.sh
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-layer-lifecycle.py" ./scripts/run-gnome-shell.sh

[private]
gnome-window-effects-verify:
    GNOBLIN_CONFIG='' GNOBLIN_PREFIX="{{prefix}}" GNOBLIN_TEST_DBUS_CLIENT="{{justfile_directory()}}/tests/test-window-effects.py" ./scripts/run-gnome-shell.sh

# Build a source RPM from an already prepared release source directory.
[private]
srpm PROJECT SOURCES OUTPUT:
    ./scripts/build-srpm.sh "{{PROJECT}}" "{{SOURCES}}" "{{OUTPUT}}"

# Build the complete GitHub release asset set from a clean checkout.
[private]
release-assets OUTPUT="dist/release" TAG="":
    ./scripts/build-release-assets.sh "{{OUTPUT}}" "{{TAG}}"

# Publish prepared source RPMs to an existing COPR project in dependency order.
[private]
copr PROJECT SCHEMAS_SRPM MUTTER_SRPM SHELL_SRPM META_SRPM:
    ./scripts/publish-copr.sh "{{PROJECT}}" "{{SCHEMAS_SRPM}}" "{{MUTTER_SRPM}}" "{{SHELL_SRPM}}" "{{META_SRPM}}"

# Read-only checks across repository-owned source files.
[private]
check-gnome-version:
    ./scripts/gnome-versions.py check --upstream

# Refresh/check the native-package adapter input exported by the Nix flake.
[private]
package-manifest COMMAND="check":
    ./scripts/sync-package-manifest.py {{COMMAND}}

lint *args:
    ./scripts/quality.sh lint {{args}}

# Apply formatting. Pass --files followed by paths to limit the change.
format *args:
    ./scripts/quality.sh format {{args}}
