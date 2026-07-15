# gnoblin build orchestration — patched GNOME Shell on patched Mutter. gnoblin is
# "just GNOME + Mutter": Mutter carries the wlr-layer-shell + protocol overlays
# (patches/mutter/ + src/protocols/); GNOME Shell carries a thin patch set (relaxed
# extension loading, unsafe-mode, portal auto-grant, hidden native top bar) and the
# `gnoblin` session mode that strips its stock UI. Chrome is bring-your-own: any
# layer-shell client (Quickshell, waybar, a custom one, or none) draws the UI.
#
# The from-scratch C++ compositor + Rust/Slint clients were RETIRED; recover them
# from the `archive/cpp-compositor` tag. Submodules are pinned + pristine; gnoblin's
# changes are patches/ + src/ overlays. `just dev` builds the whole stack into ./install.

set shell := ["bash", "-uc"]

# Patched subprojects built by `just dev`. (slint + the C++ compositor are retired.)
patch_projects := "mutter gnome-shell"
rpm_projects := "mutter gnome-shell"

# Local dev prefix: the whole gnoblin stack is built+installed here (no system install).
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
    case {{PROJ}} in mutter) t=49.5;; gnome-shell) t=49.6;; gnome-control-center) t=49.6;; xdg-desktop-portal-gnome) t=49.0;; *) echo "unknown {{PROJ}}"; exit 1;; esac
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
#   just dev            build+install patched mutter + patched gnome-shell + session
#                       data into ./install
#   just gnome-verify   headless: boot gnome-shell in gnoblin mode, check layer-shell
#   just gnome-dbus-verify  headless: org.gnoblin.* control protocol round-trip
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
# subsystems are toggled live over org.gnoblin.* — bring-your-own chrome draws the UI.
dev-gnome-shell: dev-mutter (patch "gnome-shell")
    # ALWAYS build clean: `patch gnome-shell` resets the submodule (git clean/checkout)
    # and re-copies the overlay every run, which resets source mtimes underneath the
    # build dir. Reusing it yields a half-stale libshell/libst (observed: duplicate
    # g_boxed_type registration → GJS boxed-prototype crash at boot). A fresh build dir
    # is the only reliably-correct option here.
    rm -rf build/gnome-shell
    PKG_CONFIG_PATH={{prefix}}/lib64/pkgconfig meson setup build/gnome-shell subprojects/gnome-shell {{gnome_shell_dev_opts}}
    PKG_CONFIG_PATH={{prefix}}/lib64/pkgconfig meson install -C build/gnome-shell

# --- optional: unattended screen-share portal backend -----------------------
#
# xdg-desktop-portal-gnome is the org.freedesktop.impl.portal.desktop.gnome
# backend that shows the ScreenCast source-picker + RemoteDesktop consent
# dialogs (the ones rustdesk trips on Wayland). Our patch adds macOS-style
# PER-APP persistent grants: tick "Always allow this app" once and that app
# never re-prompts (grant stored in ~/.config/gnoblin/portal-grants/, keyed on
# app-id or the app's executable); other apps still prompt each time. List/revoke
# with `gnoblinctl screen-grants` / `gnoblinctl revoke-grant <id>`. It is NOT part
# of `just dev` — build it explicitly:
#
#   just dev-portal
#
# then (re)start the backend so it owns the impl portal, e.g.:
#
#   ./install/libexec/xdg-desktop-portal-gnome -r
#
portal_dev_opts := "--prefix=" + prefix + " --libdir=lib64"

# Build + install the patched xdg-desktop-portal-gnome backend into ./install.
dev-portal: (patch "xdg-desktop-portal-gnome")
    meson setup --reconfigure build/xdg-desktop-portal-gnome subprojects/xdg-desktop-portal-gnome {{portal_dev_opts}} || meson setup build/xdg-desktop-portal-gnome subprojects/xdg-desktop-portal-gnome {{portal_dev_opts}}
    meson install -C build/xdg-desktop-portal-gnome

# --- optional: gnoblin settings (forked gnome-control-center) ---------------
#
# A light fork of gnome-control-center: it keeps the `gnome-control-center`
# binary name (so "open Settings" / Exec=gnome-control-center gets this build
# when ./install is ahead on PATH), adds a `gnoblin` panel that drives the
# org.gnoblin.Shell control protocol (feature toggles + screencast grants +
# soft reload), and hides panels that make no sense under gnoblin.
#
# Two gnoblin changes on top of the pinned 49.6 tag:
#   - overlay:  src/control-center/panels/gnoblin/*  (the whole panel)
#   - patch:    patches/gnome-control-center/10-gnoblin-panel  (2 one-line regs)
# Hiding is done purely at install time below (delete the panel .desktop files),
# so it is trivially reversible and needs no patch.
#
# It is NOT part of `just dev` — build it explicitly:
#
#   just dev-settings
#
settings_dev_opts := "--prefix=" + prefix + " --libdir=lib64"

# Panels that make no sense under gnoblin (no GNOME top bar / overview / dash /
# workspaces gestures). Hidden by removing their installed panel .desktop, which
# cc_panel_loader_fill_model() treats as "panel absent" (skips it from the list).
settings_hidden_panels := "multitasking"

# Build + install the gnoblin-forked gnome-control-center into ./install, then
# hide the panels that don't apply under gnoblin.
dev-settings: (patch "gnome-control-center")
    meson setup --reconfigure build/gnome-control-center subprojects/gnome-control-center {{settings_dev_opts}} || meson setup build/gnome-control-center subprojects/gnome-control-center {{settings_dev_opts}}
    # blueprint-compiler: g-c-c compiles .blp UI files. If the system package is
    # present, meson uses it. Otherwise it falls back to the meson wrap, whose
    # build-side launcher can't import its own package (sys.path[0] is the build dir,
    # not the source) — so link the source package next to the launcher. No-op when
    # the system blueprint-compiler is used (no wrap dir).
    bpsrc="{{justfile_directory()}}/subprojects/gnome-control-center/subprojects/blueprint-compiler/blueprintcompiler"; \
    bpdir="build/gnome-control-center/subprojects/blueprint-compiler"; \
    if [ -d "$bpdir" ] && [ -d "$bpsrc" ]; then ln -sfn "$bpsrc" "$bpdir/blueprintcompiler"; fi
    meson install -C build/gnome-control-center
    # Hide non-applicable panels (reversible: just re-run dev-settings to restore).
    for p in {{settings_hidden_panels}}; do \
      rm -f "{{prefix}}/share/applications/gnome-$p-panel.desktop"; \
      echo ">> hid $p panel"; \
    done
    @echo ">> gnoblin settings installed in {{prefix}} — run: {{prefix}}/bin/gnome-control-center gnoblin"

# Build the whole gnoblin stack (patched mutter + patched gnome-shell) into ./install.
dev: dev-gnome-shell dev-session
    @echo ">> gnoblin stack (mutter + gnome-shell) installed in {{prefix}} — run 'just gnome-verify'"

# Install the gnoblin session data (session mode, gnome-session, .desktop) into ./install.
dev-session:
    ./scripts/install-session.sh {{prefix}}

# Register the gnoblin session with your live systemd --user instance (links
# org.gnoblin.Shell.target/@wayland.service -- gnoblin-specific unit names,
# does NOT touch org.gnome.Shell*) and print the (root) command to make
# "Gnoblin" appear at your login manager's session picker. NOT run by
# `just dev`/`dev-session` -- it's the one step that touches state outside
# ./install. See docs/installation.md.
dev-session-register:
    ./scripts/register-session.sh {{prefix}}

# Devkit: open a VISIBLE nested gnoblin session (a window in your current Wayland
# session) + a terminal already wired to it — so you can launch your own chrome
# against gnoblin without vendoring anything here. In the terminal, run e.g.
#   qs -p ~/dev/kobel-shell
# and your bar appears inside the nested gnoblin. Optional arg picks the terminal.
#   just gnome-devkit          # foot/kitty/alacritty auto-detected
#   just gnome-devkit kitty
gnome-devkit *TERMINAL:
    ./scripts/run-gnome-devkit.sh {{TERMINAL}}

# Headless regression test for the devkit: boot a nested gnoblin and confirm the
# spawned-terminal env (isolated bus + gnoblinctl) can drive org.gnoblin.Shell.
gnome-devkit-verify:
    ./scripts/test-gnome-devkit.sh

# Headless: boot patched gnome-shell in the `gnoblin` session mode and verify it
# starts + advertises wlr-layer-shell (so any layer-shell client can draw chrome).
# This is the stack's headless smoke test.
gnome-verify:
    ./scripts/run-gnome-shell.sh

# Headless: boot stock GNOME mode from the patched packages and prove Gnoblin's
# custom Wayland globals and org.gnoblin.Shell component remain unavailable.
gnome-stock-protocol-isolation-verify:
    ./scripts/test-stock-protocol-isolation.sh

# Headless: exercise the org.gnoblin.* control protocol end-to-end over D-Bus
# (Ping / GetVersion / Reload → soft in-process reload).
gnome-dbus-verify:
    ./scripts/test-gnome-dbus.sh

# Headless: prove live extension hot-reload — install a probe extension, edit its
# code, ReloadExtension over org.gnoblin.*, confirm the new code ran (no relogin).
gnome-hot-reload-verify:
    ./scripts/test-hot-reload.sh

# Headless: prove the GJS user-scripting layer — drop a script, edit it, reload
# via org.gnoblin.*, confirm the new code ran.
gnome-scripting-verify:
    ./scripts/test-scripting.sh

# Headless: prove the `notifications` toggle — gnoblin releases org.freedesktop.
# Notifications when disabled (so an external daemon can own it) and reclaims it.
gnome-notifications-verify:
    ./scripts/test-notifications-toggle.sh

# Headless: compile black-box clients from the owned protocol XML and exercise
# registry binding plus clean disconnect against the running compositor.
gnome-protocol-boundaries-verify:
    ./scripts/test-protocol-boundaries.sh

# Headless: prove gnoblin.conf [protocols] gating — disabling wlr-layer-shell in
# the config stops zwlr_layer_shell_v1 being advertised.
gnome-protocol-gating-verify:
    ./scripts/test-protocol-gating.sh

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

# Tier 1: logic test for the shared C config parser (src/config), no display.
test-config:
    ./scripts/test-config.sh

# Tier 2: mutter in-tree headless functional tests. Run serially: these tests
# each boot a headless compositor with virtual input/monitors, and parallel Meson
# scheduling can starve a test long enough to trip its 60s timeout.
#
# NEEDS A REAL ENVIRONMENT: the native/Wayland backend tests boot a compositor that
# monitors an ICC profile dir; in sandboxes lacking a local file monitor (inotify)
# they all bail with "Unable to find default local file monitor type" (exit 251) —
# environmental, NOT a gnoblin regression (the unit tests, which need no backend,
# pass; the ref-image tests even log "Image matched" before bailing). Run on real HW.
test-mutter: (patch "mutter")
    meson setup --reconfigure build/mutter-tests subprojects/mutter {{mutter_test_opts}} || meson setup build/mutter-tests subprojects/mutter {{mutter_test_opts}}
    meson compile -C build/mutter-tests
    count="$(meson test -C build/mutter-tests {{mutter_test_suites}} --list | sed '/^$/d' | wc -l)"; if [ "$count" -le 0 ]; then echo "FAIL: no Mutter tests selected"; exit 1; fi; echo ">> running $count Mutter unit/Wayland/native tests"
    meson test -C build/mutter-tests {{mutter_test_run_opts}} {{mutter_test_suites}}
    focus_count="$(meson test -C build/mutter-tests {{mutter_focus_tests}} --list | sed '/^$/d' | wc -l)"; if [ "$focus_count" -le 0 ]; then echo "FAIL: no Mutter focus tests selected"; exit 1; fi; echo ">> running $focus_count Mutter focus/stacking tests"
    meson test -C build/mutter-tests {{mutter_test_run_opts}} {{mutter_focus_tests}}

# Fast deterministic checks: syntax, parser behaviour, secure state publication,
# and RPM sidecar-source completeness. Does not boot a compositor.
verify-fast:
    bash -n scripts/*.sh src/tools/*
    tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT; PYTHONPYCACHEPREFIX="$tmp" python3 -m py_compile scripts/*.py
    ./scripts/test-log-diagnostics.sh
    ./scripts/test-secure-state.sh
    ./scripts/test-rpm-sources.sh
    just test-config

# Every isolated headless integration check against an existing ./install.
# This intentionally reuses installed binaries; use `verify-headless` to build.
# Run serially because each test starts GNOME Shell and shared host services.
verify-installed-headless:
    just gnome-verify
    just gnome-stock-protocol-isolation-verify
    just gnome-protocol-boundaries-verify
    just gnome-dbus-verify
    just gnome-hot-reload-verify
    just gnome-scripting-verify
    just gnome-notifications-verify
    just gnome-protocol-gating-verify
    just gnome-devkit-verify

# Build the current source and patch set before running headless integration.
verify-headless: dev
    just verify-installed-headless

# Default local gate: deterministic checks plus the complete headless suite.
verify:
    just verify-fast
    just verify-headless

# Release gate: local verification, Mutter's real-host suite, then both RPMs.
verify-release:
    just verify
    just test-mutter
    just rpm-all

# Compatibility alias. `test` is deliberately fast; use `verify` for headless
# integration and `verify-release` for the real-host and packaging gates.
test: verify-fast
    @echo ">> fast checks passed; compositor integration was not run."
    @echo ">> run 'just verify' or 'just verify-release' for broader gates."

clean:
    rm -rf build
