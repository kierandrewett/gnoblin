# gnoblin build orchestration — the gnoblin-shell compositor (src/compositor/) +
# Rust layer-shell clients (src/clients/), on a mutter patched only for gnoblin's
# Wayland protocol overlays. gnome-shell has been RETIRED (the compositor replaced
# it); its submodule + patches/gnome-shell/ remain only as legacy reference and
# are no longer built — delete them once you're sure you don't want them.
#
# mutter is a pinned submodule kept pristine; gnoblin's changes are patches/mutter/
# + the overlays under src/. `just dev` builds the whole stack into ./install.

set shell := ["bash", "-uc"]

# gnome-shell intentionally dropped (retired); gnome-control-center kept for the
# legacy settings panel.
patch_projects := "mutter gnome-control-center"
rpm_projects := "mutter"

# Local dev prefix: the whole gnoblin stack is built+installed here for the devkit.
prefix := justfile_directory() / "install"

_default:
    @just --list

# Initialise / update the pinned submodules.
init:
    git submodule update --init --recursive
    @echo "mutter               -> $(git -C subprojects/mutter               describe --tags)"
    @echo "gnome-shell          -> $(git -C subprojects/gnome-shell          describe --tags)"
    @echo "gnome-control-center -> $(git -C subprojects/gnome-control-center describe --tags)"

# Apply the patch series to a subproject (resets it to the pinned tag first).
patch PROJ:
    ./scripts/apply-patches.sh {{PROJ}}

# Apply patches to every subproject.
patch-all:
    for p in {{patch_projects}}; do ./scripts/apply-patches.sh "$p"; done

# Reset a subproject back to its pristine pinned tag.
reset PROJ:
    #!/usr/bin/env bash
    set -euo pipefail
    case {{PROJ}} in mutter) t=49.5;; gnome-shell|gnome-control-center) t=49.6;; *) echo "unknown {{PROJ}}"; exit 1;; esac
    git -C subprojects/{{PROJ}} am --abort 2>/dev/null || true
    git -C subprojects/{{PROJ}} checkout -qf "$t"
    git -C subprojects/{{PROJ}} reset -q --hard "$t"
    git -C subprojects/{{PROJ}} clean -qfdx
    echo "{{PROJ}} reset to $t"

reset-all:
    for p in {{patch_projects}}; do just reset "$p"; done

# Configure + compile a subproject with meson into build/<proj> (dev build).
build PROJ: (patch PROJ)
    meson setup --reconfigure build/{{PROJ}} subprojects/{{PROJ}} || meson setup build/{{PROJ}} subprojects/{{PROJ}}
    meson compile -C build/{{PROJ}}

# --- dev stack: build the whole gnoblin stack into ./install and run it ------
#
#   just dev            build+install mutter (incl. Devkit viewer) + gnoblin-shell
#                       + layer-shell clients + schema into ./install
#   just devkit         open the Mutter Devkit window running gnoblin-shell
#   just devkit foot    ...with `foot` running inside it
#   just devkit-verify  headless: boot the stack, list advertised protocols
#
mutter_dev_opts := "--prefix=" + prefix + " --libdir=lib64 -Ddevkit=enabled -Dtests=disabled -Ddocs=false -Dprofiler=false -Dudev_dir=" + prefix + "/lib/udev"
mutter_test_opts := "--prefix=" + prefix + " --libdir=lib64 -Ddevkit=enabled -Dtests=enabled -Dmutter_tests=true -Dclutter_tests=false -Dcogl_tests=false -Ddocs=false -Dprofiler=false -Dudev_dir=" + prefix + "/lib/udev"
mutter_test_suites := "--suite mutter:mutter/unit --suite mutter:mutter/wayland --suite mutter:mutter/backends/native"
mutter_focus_tests := "mutter:focus-default-window-globally-active-input mutter:click-to-focus-and-raise mutter:overview-focus mutter:sloppy-focus mutter:sloppy-focus-pointer-rest mutter:sloppy-focus-auto-raise mutter:popup-focus"
mutter_test_run_opts := "--no-rebuild --num-processes 1 --print-errorlogs"

# Build + install patched mutter (incl. the Mutter Devkit viewer) into ./install.
dev-mutter: (patch "mutter")
    meson setup --reconfigure build/mutter subprojects/mutter {{mutter_dev_opts}} || meson setup build/mutter subprojects/mutter {{mutter_dev_opts}}
    meson install -C build/mutter

# Build + install gnoblin-shell against the freshly built mutter in ./install.
dev-shell: dev-mutter
    PKG_CONFIG_PATH={{prefix}}/lib64/pkgconfig meson setup --reconfigure build/gnoblin-shell src/compositor --prefix={{prefix}} --libdir=lib64 || PKG_CONFIG_PATH={{prefix}}/lib64/pkgconfig meson setup build/gnoblin-shell src/compositor --prefix={{prefix}} --libdir=lib64
    PKG_CONFIG_PATH={{prefix}}/lib64/pkgconfig meson install -C build/gnoblin-shell

# Install gnoblin's layer-shell clients (topbar, dock) + settings schema.
dev-userspace:
    ./scripts/install-userspace.sh {{prefix}}

# Build the whole gnoblin stack (mutter + shell + userspace) into ./install.
dev: dev-shell dev-userspace
    @echo ">> gnoblin stack installed in {{prefix}} — run 'just devkit' or 'just devkit-verify'"

# Open gnoblin-shell in the Mutter Devkit window (own dbus/gsettings). Optional
# client runs inside it, e.g. `just devkit foot`. Build first with `just dev`.
devkit *CLIENT:
    ./scripts/run-devkit.sh visible {{CLIENT}}

# Headless: boot the gnoblin stack and list the protocols it advertises.
devkit-verify:
    ./scripts/run-devkit.sh verify

# Drive a headless gnoblin-shell from the LLM/CI side with no human in the loop:
# boot, screenshot (grim), add a late virtual monitor (ScreenCast), inject input,
# run arbitrary clients, detect compositor aborts. Owns its own dbus + sandbox.
#   just harness smoke | shot OUT.png | late OUT.png | storm | run CLIENT
#   just harness keys 'Super+Space:calc' OUT.png | wm 'spawn:foot,maximize' OUT.png | boot
harness *ARGS:
    python3 ./scripts/devkit-harness.py {{ARGS}}

# Review one or more screenshots with external visual/design feedback.
design-review *SCREENSHOTS:
    ./scripts/design-review.sh {{SCREENSHOTS}}

# Produce a patched release tarball in ~/rpmbuild/SOURCES.
tarball PROJ:
    ./scripts/make-tarball.sh {{PROJ}}

# Build a binary RPM (Fedora). Patches are pre-applied in the tarball.
rpm PROJ: (tarball PROJ)
    rpmbuild -bb packaging/rpm/{{PROJ}}.spec

rpm-all:
    for p in {{rpm_projects}}; do just rpm "$p"; done

# deb / arch packaging are scaffolded; see packaging/{deb,arch}/README.md.
deb PROJ:
    @echo "deb packaging not implemented yet — see packaging/deb/README.md"
arch PROJ:
    @echo "arch packaging not implemented yet — see packaging/arch/README.md"

# --- tests (see scripts/ and the mutter in-tree suite) -----------------------

# Tier 1: static / build checks (patches apply, protocol lints, schema default).
test-build:
    ./scripts/test-build.sh

# Rust client checks — formatting, clippy, and the parsers/logic behind the
# Slint shell clients (config, client-arg protocol, app matching) + the remaining
# gtk4 daemons. No display. The Slint clients are a workspace at src/clients
# (shared target).
test-clients:
    cargo fmt --manifest-path src/clients/Cargo.toml --all -- --check
    cargo clippy --manifest-path src/clients/Cargo.toml --all-targets -- -D warnings
    cargo test --manifest-path src/clients/Cargo.toml

# Security/geometry logic the visual + lock features rely on: PAM rejects wrong
# passwords; the shadow/rounded-corner SDF is geometrically correct. No display.
test-logic:
    ./scripts/test-logic.sh

# Tier 2: mutter in-tree headless functional tests. Run serially: these tests
# each boot a headless compositor with virtual input/monitors, and parallel Meson
# scheduling can starve a test long enough to trip its 60s timeout.
test-mutter: (patch "mutter")
    meson setup --reconfigure build/mutter-tests subprojects/mutter {{mutter_test_opts}} || meson setup build/mutter-tests subprojects/mutter {{mutter_test_opts}}
    meson compile -C build/mutter-tests
    count="$(meson test -C build/mutter-tests {{mutter_test_suites}} --list | sed '/^$/d' | wc -l)"; if [ "$count" -le 0 ]; then echo "FAIL: no Mutter tests selected"; exit 1; fi; echo ">> running $count Mutter unit/Wayland/native tests"
    meson test -C build/mutter-tests {{mutter_test_run_opts}} {{mutter_test_suites}}
    focus_count="$(meson test -C build/mutter-tests {{mutter_focus_tests}} --list | sed '/^$/d' | wc -l)"; if [ "$focus_count" -le 0 ]; then echo "FAIL: no Mutter focus tests selected"; exit 1; fi; echo ">> running $focus_count Mutter focus/stacking tests"
    meson test -C build/mutter-tests {{mutter_test_run_opts}} {{mutter_focus_tests}}

# LEGACY tier 3: nested *gnome-shell* smoke (gnome-shell has been retired; this
# remains only as reference). The real integration tests are `test-devkit` below.
test-nested:
    ./scripts/test-nested.sh

# Tier 3 (integration): headless behavioural regressions driving a real
# gnoblin-shell via the devkit harness — frame-callback crash guard, keybind ->
# launcher, maximize/strut, fullscreen cover, lock, snap regions, notifications,
# multi-monitor, wallpaper. Needs a dev build in ./install.
# See tests/layer-shell/README.md and scripts/devkit-harness.py.
test-devkit:
    ./tests/layer-shell/run-devkit-dbus.sh
    ./tests/layer-shell/run-frame-callback-crash.sh
    ./tests/layer-shell/run-layer-shell-protocol.sh
    ./tests/layer-shell/run-output-destroyed-closes.sh
    ./tests/layer-shell/run-configure-storm.sh
    ./tests/layer-shell/run-autostart-retry.sh
    ./tests/layer-shell/run-autostart-output-removal.sh
    ./tests/layer-shell/run-keybind-launcher.sh
    ./tests/layer-shell/run-launcher-activates-app.sh
    ./tests/layer-shell/run-role-spawn-reap.sh
    ./tests/layer-shell/run-explicit-command-reap.sh
    ./tests/layer-shell/run-pointer-input.sh
    ./tests/layer-shell/run-dock-launch.sh
    ./tests/layer-shell/run-firefox-launch.sh
    ./tests/layer-shell/run-dock-live-favorites.sh
    ./tests/layer-shell/run-osd-passthrough.sh
    ./tests/layer-shell/run-dock-menu-input-region.sh
    ./tests/layer-shell/run-window-menu-input.sh
    ./tests/layer-shell/run-layer-move-focus.sh
    ./tests/layer-shell/run-layer-keyboard-focus.sh
    ./tests/layer-shell/run-topbar-live-commands.sh
    ./tests/layer-shell/run-topbar-layout-live.sh
    ./tests/layer-shell/run-topbar-focused-app-menu.sh
    ./tests/layer-shell/run-notification-center-input.sh
    ./tests/layer-shell/run-maximize-strut.sh
    ./tests/layer-shell/run-fullscreen-cover.sh
    ./tests/layer-shell/run-lock-engage.sh
    ./tests/layer-shell/run-snap-regions.sh
    ./tests/layer-shell/run-notifications.sh
    ./tests/layer-shell/run-region-lifetime.sh
    ./tests/layer-shell/run-slint-animation-frames.sh
    ./tests/layer-shell/run-topbar-live-motion.sh
    ./tests/layer-shell/run-topbar-live-backdrop.sh
    ./tests/layer-shell/run-night-light-hotplug.sh
    ./tests/layer-shell/run-multimonitor.sh
    ./tests/layer-shell/run-power-menu-output-size.sh
    ./tests/layer-shell/run-power-menu-resize.sh
    ./tests/layer-shell/run-wallpaper.sh
    ./tests/layer-shell/run-wallpaper-output.sh
    ./tests/layer-shell/run-background-layer-input.sh
    ./tests/layer-shell/run-protocols.sh
    ./tests/layer-shell/run-kde-appmenu-backend.sh
    ./tests/layer-shell/run-topbar-dbusmenu.sh
    ./tests/layer-shell/run-effects-shadow.sh
    ./tests/layer-shell/run-maximize-animation.sh
    ./tests/layer-shell/run-foreign-toplevel.sh
    ./tests/layer-shell/run-window-rules.sh
    ./tests/layer-shell/run-tiling.sh
    ./tests/layer-shell/run-overview.sh
    ./tests/layer-shell/run-switcher.sh
    ./tests/layer-shell/run-grid.sh
    ./tests/layer-shell/run-input-config.sh
    ./tests/layer-shell/run-output-config.sh
    ./tests/layer-shell/run-workspaces.sh
    ./tests/layer-shell/run-blur.sh
    ./tests/layer-shell/run-gestures.sh
    ./tests/layer-shell/run-maximize-effects.sh

# Optional exact `just devkit` smoke. This opens the Mutter Devkit viewer on the
# host session and interrupts it with timeout, so keep it separate from the
# headless `test-devkit` suite.
test-devkit-visible:
    ./tests/layer-shell/run-visible-devkit.sh

# Run every tier the environment allows.
test: test-build test-clients test-logic
    @echo ">> tier 2 (mutter): 'just test-mutter'  |  tier 3 (integration): 'just test-devkit' — needs ./install from 'just dev'."

# Install the launcher shim + Super+Space keybinding for the current user.
setup-launcher:
    ./dist/install-launcher.sh

clean:
    rm -rf build
