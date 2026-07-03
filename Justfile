# gnoblin build orchestration — patched GNOME Shell on patched Mutter. gnoblin is
# "just GNOME + Mutter": Mutter carries the wlr-layer-shell + protocol overlays
# (patches/mutter/ + src/protocols/); GNOME Shell carries a thin patch set (relaxed
# extension loading, unsafe-mode, portal auto-grant, hidden native top bar) and the
# `gnoblin` session mode that strips its stock UI. Quickshell (repo: kobel) draws
# the chrome as layer-shell clients.
#
# The from-scratch C++ compositor + Rust/Slint clients were RETIRED; recover them
# from the `archive/cpp-compositor` tag. Submodules are pinned + pristine; gnoblin's
# changes are patches/ + src/ overlays. `just dev` builds the whole stack into ./install.

set shell := ["bash", "-uc"]

# Patched subprojects built by `just dev`. (slint + the C++ compositor are retired.)
patch_projects := "mutter gnome-shell"
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
    case {{PROJ}} in mutter) t=49.5;; gnome-shell) t=49.6;; *) echo "unknown {{PROJ}}"; exit 1;; esac
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
gnome_shell_dev_opts := "--prefix=" + prefix + " --libdir=lib64 -Dtests=false -Dman=false -Dgtk_doc=false"

# Build + install patched mutter (incl. the Mutter Devkit viewer) into ./install.
dev-mutter: (patch "mutter")
    meson setup --reconfigure build/mutter subprojects/mutter {{mutter_dev_opts}} || meson setup build/mutter subprojects/mutter {{mutter_dev_opts}}
    meson install -C build/mutter

# Build + install patched gnome-shell against the freshly built mutter in ./install.
# gnome-shell is the compositor+shell again; its stock UI (panel/overview/dash) is
# stripped via the `gnoblin` session mode + a minimal native-topbar patch, and its
# subsystems are toggled live over org.gnoblin.* — Quickshell (kobel) draws the chrome.
dev-gnome-shell: dev-mutter (patch "gnome-shell")
    # ALWAYS build clean: `patch gnome-shell` resets the submodule (git clean/checkout)
    # and re-copies the overlay every run, which resets source mtimes underneath the
    # build dir. Reusing it yields a half-stale libshell/libst (observed: duplicate
    # g_boxed_type registration → GJS boxed-prototype crash at boot). A fresh build dir
    # is the only reliably-correct option here.
    rm -rf build/gnome-shell
    PKG_CONFIG_PATH={{prefix}}/lib64/pkgconfig meson setup build/gnome-shell subprojects/gnome-shell {{gnome_shell_dev_opts}}
    PKG_CONFIG_PATH={{prefix}}/lib64/pkgconfig meson install -C build/gnome-shell

# Build the whole gnoblin stack (patched mutter + patched gnome-shell) into ./install.
dev: dev-gnome-shell dev-session
    @echo ">> gnoblin stack (mutter + gnome-shell) installed in {{prefix}} — run 'just gnome-verify'"

# Install the gnoblin session data (session mode, gnome-session, .desktop) into ./install.
dev-session:
    ./scripts/install-session.sh {{prefix}}

# Headless: boot patched gnome-shell in the `gnoblin` session mode and verify it
# starts + advertises wlr-layer-shell (so any layer-shell client can draw chrome).
# This is the current stack's smoke test (the C++ devkit paths below are legacy).
gnome-verify:
    ./scripts/run-gnome-shell.sh

# Headless: exercise the org.gnoblin.* control protocol end-to-end over D-Bus
# (Ping / GetVersion / Reload → soft in-process reload).
gnome-dbus-verify:
    ./scripts/test-gnome-dbus.sh

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

# Run every tier the environment allows. The C++/Slint integration tiers
# (test-clients / test-logic / test-devkit) were retired with that stack.
test:
    @echo ">> smoke: 'just gnome-verify' + 'just gnome-dbus-verify' (need ./install from 'just dev')."
    @echo ">> tier 2 (mutter): 'just test-mutter'."

# Install the launcher shim + Super+Space keybinding for the current user.
setup-launcher:
    ./dist/install-launcher.sh

clean:
    rm -rf build
