#!/usr/bin/env bash
# Tier 1 — static / build checks that need no display and no full compile:
#   * every patch applies cleanly to the pinned tag (zero rejects)
#   * the wlr-layer-shell protocol XML is well-formed and scannable
#   * the mutter overlay-key schema default is empty
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fail=0
ok()   { echo "  ok    $*"; }
bad()  { echo "  FAIL  $*"; fail=1; }

reset_to_tag() {
  local sm="$1" tag="$2"
  git -C "$sm" am --abort >/dev/null 2>&1 || true
  git -C "$sm" checkout -qf "$tag"
  git -C "$sm" reset -q --hard "$tag"
  git -C "$sm" clean -qfdx
}

echo "== repo script syntax checks =="
if python3 - "$ROOT" <<'PY'
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
files = sorted(
    path
    for name in ("scripts", "tests/layer-shell")
    for path in root.joinpath(name).glob("**/*.py")
    if path.is_file()
)
errors = []
for path in files:
    try:
        source = path.read_text(encoding="utf-8")
        compile(source, str(path), "exec")
    except SyntaxError as exc:
        errors.append(f"{path.relative_to(root)}:{exc.lineno}:{exc.offset}: {exc.msg}")
    except UnicodeDecodeError as exc:
        errors.append(f"{path.relative_to(root)}: cannot decode as UTF-8: {exc}")

if errors:
    for error in errors:
        print(error)
    sys.exit(1)

print(f"checked {len(files)} Python files")
PY
then
  ok "Python harness/test files parse"
else
  bad "Python harness/test files contain syntax errors"
fi

shell_count=0
shell_bad=0
while IFS= read -r script; do
  shell_count=$((shell_count + 1))
  if ! bash -n "$script"; then
    shell_bad=1
  fi
done < <(
  {
    find "$ROOT/scripts" "$ROOT/tests/layer-shell" -type f -name '*.sh'
    find "$ROOT/dist" -maxdepth 1 -type f -perm -111
  } | sort -u
)
if [ "$shell_bad" -eq 0 ]; then
  ok "shell scripts parse with bash -n ($shell_count files)"
else
  bad "one or more shell scripts fail bash -n"
fi

if grep -q 'export PATH="\$PREFIX/bin:\$PATH"' "$ROOT/scripts/run-devkit.sh" &&
   grep -Fq 'env["PATH"] = f"{PREFIX}/bin:"' "$ROOT/scripts/devkit-harness.py" &&
   ! grep -q "GNOBLIN_LAYER_SHELL_COMPONENT_DIR" "$ROOT/scripts/run-devkit.sh" &&
   ! grep -q "GNOBLIN_LAYER_SHELL_COMPONENT_DIR" "$ROOT/scripts/devkit-harness.py" &&
   grep -q "rm -rf \"\$PREFIX/libexec/gnoblin\"" "$ROOT/scripts/install-userspace.sh" &&
   grep -q "warn_if_stale_artifacts" "$ROOT/scripts/run-devkit.sh" &&
   grep -Fq 'crate="${bin#gnoblin-}"' "$ROOT/scripts/run-devkit.sh" &&
   grep -Fq '"$ROOT/src/clients/$crate"' "$ROOT/scripts/run-devkit.sh" &&
   ! grep -Fq 'find "$ROOT/src/clients" \' "$ROOT/scripts/run-devkit.sh" &&
   grep -q "installed Slint/userspace clients may be stale" "$ROOT/scripts/run-devkit.sh"; then
  ok "devkit resolves installed clients from prefix bin and warns on per-client stale artifacts"
else
  bad "devkit client resolution or stale-artifact warning regressed"
fi

if python3 - "$ROOT" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
targets = [
    root / "scripts" / "run-devkit.sh",
    root / "scripts" / "devkit-harness.py",
]
required = [
    "default=gtk",
    "org.freedesktop.impl.portal.ScreenCast=gnome",
    "org.freedesktop.impl.portal.RemoteDesktop=gnome",
    "org.freedesktop.impl.portal.Screenshot=gnome",
    "org.freedesktop.impl.portal.GlobalShortcuts=gnome",
    "org.freedesktop.impl.portal.Background=none",
    "org.freedesktop.impl.portal.Clipboard=none",
    "org.freedesktop.impl.portal.InputCapture=none",
    "org.freedesktop.impl.portal.Lockdown=none",
    "org.freedesktop.impl.portal.Secret=none",
    "org.freedesktop.impl.portal.Usb=none",
    "org.freedesktop.impl.portal.Wallpaper=none",
]
errors = []
for path in targets:
    text = path.read_text(encoding="utf-8")
    rel = path.relative_to(root)
    if "default=gnome;gtk" in text:
        errors.append(f"{rel}: devkit portal config re-enables broad GNOME default")
    for needle in required:
        if needle not in text:
            errors.append(f"{rel}: devkit portal config missing {needle}")

if errors:
    print("\n".join(errors))
    sys.exit(1)
PY
then
  ok "devkit portal selection keeps GTK as default and GNOME only for screencast paths"
else
  bad "devkit portal selection regressed"
fi

if python3 - "$ROOT" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
text = (root / "scripts" / "run-devkit.sh").read_text(encoding="utf-8")
home_capture = text.find("HOST_HOME=")
home_isolation = text.find('export HOME="$DK/home"')
wallpaper_default = text.find('$HOST_HOME/Documents/wallpaper_light.jpg')
if min(home_capture, home_isolation, wallpaper_default) < 0:
    print("run-devkit.sh does not preserve host home for the default wallpaper")
    sys.exit(1)
if not (home_capture < home_isolation and home_capture < wallpaper_default):
    print("run-devkit.sh captures host home too late for default wallpaper")
    sys.exit(1)
PY
then
  ok "visible devkit default wallpaper resolves from the host home before sandboxing"
else
  bad "visible devkit default wallpaper can be lost when HOME is sandboxed"
fi

if python3 - "$ROOT" <<'PY'
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
text = (root / "scripts" / "run-devkit.sh").read_text(encoding="utf-8")
try:
    seed = text.split("# Seed a gnoblin.conf in the sandbox", 1)[1]
    seed = seed.split('} > "$DK/config/gnoblin/gnoblin.conf"', 1)[0]
except IndexError:
    print("could not locate visible devkit config generator")
    sys.exit(1)

required = {
    'echo "[topbar]"',
    'echo "left = workspaces, focused-app, appmenu, spring"',
    'echo "center = clock"',
    'echo "right = launcher, tray, status"',
    'echo "appmenu-backend = auto"',
}
missing = sorted(line for line in required if line not in seed)
if missing:
    print("visible devkit config omits the desktop topbar defaults:")
    print("\n".join(missing))
    sys.exit(1)
PY
then
  ok "visible devkit preserves the default topbar workspaces/focused-app/appmenu layout"
else
  bad "visible devkit can hide the default workspaces/focused-app/appmenu widgets"
fi

if grep -q 'target-dir = "build/cargo-target"' "$ROOT/.cargo/config.toml" &&
   grep -q "cargo metadata" "$ROOT/scripts/install-userspace.sh" &&
   grep -q "target_directory" "$ROOT/scripts/install-userspace.sh" &&
   ! grep -q 'src/clients/target/release/\$bin' "$ROOT/scripts/install-userspace.sh" &&
   grep -q "build/cargo-target/release/gnoblin-launcher" "$ROOT/dist/gnoblin-launcher" &&
   grep -q "build/cargo-target/release/gnoblin-launcher" "$ROOT/dist/install-launcher.sh"; then
  ok "Cargo client output stays out of src and installers resolve the configured target dir"
else
  bad "Cargo client output or installer lookup still assumes src/clients/target"
fi

if grep -q "prefix_from_pid" "$ROOT/scripts/devkit-compare-topbar.sh" &&
   grep -q "gnoblin-shell" "$ROOT/scripts/devkit-compare-topbar.sh" &&
   grep -q "bin/gnoblin-topbar" "$ROOT/scripts/devkit-compare-topbar.sh" &&
   grep -q "bin/gnoblin-dock" "$ROOT/scripts/devkit-compare-topbar.sh" &&
   ! grep -Eq "build/gnome-shell|org\\.gnome\\.Shell|src/clients/(topbar|dock)/gnoblin-|gnoblin-(topbar|dock)\\.js" \
     "$ROOT/scripts/devkit-compare-topbar.sh"; then
  ok "devkit compare helper targets installed gnoblin clients"
else
  bad "devkit compare helper still references retired gnome-shell/source clients"
fi

if grep -q "gnoblin-shell" "$ROOT/scripts/capture-devkit.sh" &&
   grep -q -- "--wayland-display" "$ROOT/scripts/capture-devkit.sh" &&
   ! grep -Eq "gnome-shell --wayland|nested gnome-shell|gnome-shell --devkit" \
     "$ROOT/scripts/capture-devkit.sh"; then
  ok "devkit capture helper targets live gnoblin-shell devkit sessions"
else
  bad "devkit capture helper still targets retired gnome-shell devkit sessions"
fi

if grep -q 'TOPBAR_BIN="\$GNOBLIN_PREFIX/bin/gnoblin-topbar"' "$ROOT/scripts/test-nested.sh" &&
   grep -q 'DOCK_BIN="\$GNOBLIN_PREFIX/bin/gnoblin-dock"' "$ROOT/scripts/test-nested.sh" &&
   grep -q 'RUN_GNOBLIN_CLIENTS="${RUN_GNOBLIN_CLIENTS:-0}"' "$ROOT/scripts/test-nested.sh" &&
   grep -q "legacy GNOME Shell gnoblin autostart is retired" "$ROOT/scripts/test-nested.sh" &&
   ! grep -Eq "GNOBLIN_LAYER_SHELL_COMPONENT_DIR|src/clients/(topbar|dock)/gnoblin-" \
     "$ROOT/scripts/test-nested.sh"; then
  ok "legacy nested test keeps gnoblin clients opt-in and avoids retired source paths"
else
  bad "legacy nested test still references retired source-client paths"
fi

if ! grep -Eq "wofi|fuzzel|--show drun|Install 'wofi'" \
     "$ROOT/dist/gnoblin-launcher" "$ROOT/dist/install-launcher.sh" &&
   grep -q "GNOBLIN_LAUNCHER" "$ROOT/dist/gnoblin-launcher" &&
   grep -q "GNOBLIN_SOURCE_ROOT" "$ROOT/dist/gnoblin-launcher" &&
   grep -q "find_launcher" "$ROOT/dist/install-launcher.sh" &&
   grep -q "write_launcher_shim" "$ROOT/dist/install-launcher.sh" &&
   grep -q "GNOBLIN_INSTALL_LAUNCHER_NO_GSETTINGS" "$ROOT/dist/install-launcher.sh"; then
  ok "launcher setup delegates to the Slint launcher instead of installing a stale implementation"
else
  bad "launcher setup can still shadow the Slint launcher with a stale helper"
fi

launcher_tmp="$(mktemp -d)"
launcher_ok=0
mkdir -p "$launcher_tmp/fake/dist" "$launcher_tmp/home"
cp "$ROOT/dist/gnoblin-launcher" "$ROOT/dist/install-launcher.sh" "$launcher_tmp/fake/dist/"
if HOME="$launcher_tmp/home" \
   PATH="/usr/bin:/bin" \
   GNOBLIN_INSTALL_LAUNCHER_NO_GSETTINGS=1 \
   "$launcher_tmp/fake/dist/install-launcher.sh" >/dev/null 2>&1; then
  mkdir -p "$launcher_tmp/fake/install/bin"
  {
    echo '#!/usr/bin/env bash'
    echo 'echo later-build "$@"'
  } > "$launcher_tmp/fake/install/bin/gnoblin-launcher"
  chmod +x "$launcher_tmp/fake/install/bin/gnoblin-launcher"
  if bash -n "$launcher_tmp/home/.local/bin/gnoblin-launcher" &&
     [ "$(HOME="$launcher_tmp/home" PATH="/usr/bin:/bin" \
          "$launcher_tmp/home/.local/bin/gnoblin-launcher" --probe)" = "later-build --probe" ]; then
    launcher_ok=1
  fi
fi
rm -rf "$launcher_tmp"
if [ "$launcher_ok" -eq 1 ]; then
  ok "launcher shim installed before a build finds the later repo-local Slint launcher"
else
  bad "launcher shim installed before a build cannot find the later repo-local Slint launcher"
fi

launcher_tmp="$(mktemp -d)"
launcher_ok=0
mkdir -p "$launcher_tmp/fake/dist" "$launcher_tmp/fake/build/cargo-target/debug" "$launcher_tmp/home"
cp "$ROOT/dist/gnoblin-launcher" "$ROOT/dist/install-launcher.sh" "$launcher_tmp/fake/dist/"
{
  echo '#!/usr/bin/env bash'
  echo 'echo cargo-target-build "$@"'
} > "$launcher_tmp/fake/build/cargo-target/debug/gnoblin-launcher"
chmod +x "$launcher_tmp/fake/build/cargo-target/debug/gnoblin-launcher"
if HOME="$launcher_tmp/home" \
   PATH="/usr/bin:/bin" \
   GNOBLIN_INSTALL_LAUNCHER_NO_GSETTINGS=1 \
   "$launcher_tmp/fake/dist/install-launcher.sh" >/dev/null 2>&1 &&
   [ "$(HOME="$launcher_tmp/home" PATH="/usr/bin:/bin" \
        "$launcher_tmp/home/.local/bin/gnoblin-launcher" --probe)" = "cargo-target-build --probe" ]; then
  launcher_ok=1
fi
rm -rf "$launcher_tmp"
if [ "$launcher_ok" -eq 1 ]; then
  ok "launcher setup finds repo-local Cargo target binaries outside src"
else
  bad "launcher setup cannot find repo-local Cargo target binaries outside src"
fi

if ! grep -q "just tarball gnome-shell" "$ROOT/packaging/deb/README.md" &&
   ! grep -q "gtk4-rs + gtk4-layer-shell" "$ROOT/README.md" &&
   grep -q "Rust + Slint layer-shell clients" "$ROOT/README.md"; then
  ok "developer packaging/docs describe the current gnoblin-shell + Slint client stack"
else
  bad "developer packaging/docs still describe retired gnome-shell or GTK client paths"
fi

if python3 - "$ROOT" <<'PY'
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
targets = [
    root / "src/clients/crates/gnoblin-runtime/vendor/slint/Dock.slint",
    root / "src/clients/crates/gnoblin-runtime/vendor/slint/Popouts.slint",
    root / "src/clients/crates/gnoblin-runtime/vendor/slint/widgets/ContextMenu.slint",
]
standalone_targets = [
    root / "src/clients/launcher/ui/launcher.slint",
    root / "src/clients/notifyd/ui/notifications.slint",
    root / "src/clients/osd/ui/osd.slint",
    root / "src/clients/crates/gnoblin-runtime/vendor/slint/Compositor.slint",
    root / "src/clients/crates/gnoblin-runtime/vendor/slint/WindowChrome.slint",
]
errors = []
for path in targets:
    lines = path.read_text(encoding="utf-8").splitlines()
    for idx, line in enumerate(lines):
        if "background: Theme.chrome-shadow-source" not in line:
            continue
        window = lines[max(0, idx - 8) : min(len(lines), idx + 9)]
        has_token_blur = False
        has_token_offset = False
        for prop_line in window:
            stripped = prop_line.strip()
            if stripped.startswith(("width:", "height:")) and (
                "parent.width +" in stripped or "parent.height +" in stripped
            ):
                errors.append(
                    f"{path.relative_to(root)}:{idx + 1}: shadow source is larger than its surface"
                )
            if stripped.startswith("border-radius:") and " + " in stripped:
                errors.append(
                    f"{path.relative_to(root)}:{idx + 1}: shadow source radius is expanded past surface radius"
                )
            if stripped.startswith("drop-shadow-blur:"):
                has_token_blur = "Tokens." in stripped
            if stripped.startswith("drop-shadow-offset-y:"):
                has_token_offset = "Tokens." in stripped
        if not has_token_blur:
            errors.append(
                f"{path.relative_to(root)}:{idx + 1}: shadow source blur is not tokenized"
            )
        if not has_token_offset:
            errors.append(
                f"{path.relative_to(root)}:{idx + 1}: shadow source offset is not tokenized"
            )

for path in standalone_targets:
    lines = path.read_text(encoding="utf-8").splitlines()
    for idx, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith(("drop-shadow-blur:", "drop-shadow-offset-y:")) and "Tokens." not in stripped:
            errors.append(
                f"{path.relative_to(root)}:{idx + 1}: standalone Slint shadow is not tokenized"
            )

floating_standalone_targets = [
    root / "src/clients/launcher/ui/launcher.slint",
    root / "src/clients/notifyd/ui/notifications.slint",
    root / "src/clients/osd/ui/osd.slint",
]
for path in floating_standalone_targets:
    text = path.read_text(encoding="utf-8")
    for needle in (
        "background: Theme.chrome-shadow-source",
        "drop-shadow-color: Theme.chrome-shadow",
        "border-radius: Tokens.popout-corner-radius - Tokens.chrome-hairline-width",
        "height: Tokens.chrome-highlight-height",
    ):
        if needle not in text:
            errors.append(
                f"{path.relative_to(root)} missing layered floating chrome token {needle}"
            )
    if "drop-shadow-color: Theme.shadow-color" in text:
        errors.append(
            f"{path.relative_to(root)} casts floating surface shadow from the legacy shadow alias"
        )

compositor = root / "src/clients/crates/gnoblin-runtime/vendor/slint/Compositor.slint"
compositor_text = compositor.read_text(encoding="utf-8")
for label, start, end, needles in (
    (
        "launcher",
        "// ── Layer 68: App launcher",
        "// ── Layer 70: Debug overlay",
        (
            "background: Theme.chrome-shadow-source",
            "background: Theme.menu-bg",
            "drop-shadow-blur: Tokens.menu-shadow-blur",
            "drop-shadow-color: Theme.chrome-shadow",
            "border-radius: Tokens.menu-corner-radius - Tokens.chrome-hairline-width",
            "border-color: Theme.menu-border",
            "height: Tokens.chrome-highlight-height",
        ),
    ),
    (
        "debug overlay",
        "// ── Layer 70: Debug overlay",
        "// ── Layer 75: Help overlay",
        (
            "background: Theme.chrome-shadow-source",
            "background: Theme.surface-bg",
            "drop-shadow-blur: Tokens.popout-shadow-blur",
            "drop-shadow-color: Theme.chrome-shadow",
            "border-radius: Tokens.popout-corner-radius - Tokens.chrome-hairline-width",
            "border-color: Theme.surface-border",
            "height: Tokens.chrome-highlight-height",
        ),
    ),
    (
        "help overlay",
        "// ── Layer 75: Help overlay",
        "// ── Layer 99: Custom cursor overlay",
        (
            "background: Theme.chrome-shadow-source",
            "background: Theme.surface-bg",
            "drop-shadow-blur: Tokens.popout-shadow-blur",
            "drop-shadow-color: Theme.chrome-shadow",
            "border-radius: Tokens.popout-corner-radius - Tokens.chrome-hairline-width",
            "border-color: Theme.surface-border",
            "height: Tokens.chrome-highlight-height",
        ),
    ),
):
    try:
        section = compositor_text.split(start, 1)[1].split(end, 1)[0]
    except IndexError:
        errors.append(f"{compositor.relative_to(root)} missing {label} chrome section marker")
        continue
    for needle in needles:
        if needle not in section:
            errors.append(
                f"{compositor.relative_to(root)} {label} chrome missing token {needle}"
            )

for path in sorted((root / "src/clients").glob("**/*.slint")):
    if "/target/" in path.as_posix() or path.name == "Tokens.slint":
        continue
    text = path.read_text(encoding="utf-8")
    if "Theme.shadow-color" in text:
        errors.append(
            f"{path.relative_to(root)} casts shadow from legacy Theme.shadow-color alias"
        )
    for idx, line in enumerate(text.splitlines(), start=1):
        if re.search(r"\b(?:background|border-color):\s*#[0-9A-Fa-f]{3,8}\b", line):
            errors.append(
                f"{path.relative_to(root)}:{idx}: raw Slint chrome colour should be a Theme/Tokens value"
            )

dock_text = (root / "src/clients/crates/gnoblin-runtime/vendor/slint/Dock.slint").read_text(encoding="utf-8")
try:
    tooltip = dock_text.split("tip-body := Rectangle", 1)[1].split("// Active indicator", 1)[0]
except IndexError:
    errors.append("Dock.slint missing dock tooltip chrome section")
else:
    if "Speech-bubble" in tooltip or "viewbox-width:  parent.width" in tooltip:
        errors.append("Dock.slint tooltip regressed to a single custom Path silhouette")
    for needle in (
        "property <length> arrow-h",
        "fill: parent.bubble-bg",
        "background: Theme.chrome-shadow-source",
        "drop-shadow-blur: Tokens.tooltip-shadow-blur",
        "drop-shadow-offset-y: Tokens.tooltip-shadow-offset-y",
        "height: Tokens.chrome-highlight-height",
        "border-radius: parent.radius - Tokens.chrome-hairline-width",
    ):
        if needle not in tooltip:
            errors.append(f"Dock.slint tooltip missing layered chrome token {needle}")

icon_button_text = (
    root / "src/clients/crates/gnoblin-runtime/vendor/slint/widgets/IconButton.slint"
).read_text(encoding="utf-8")
if "Press-scale" in icon_button_text or "touch.pressed ? 0." in icon_button_text:
    errors.append("IconButton.slint reintroduced mouse-down press scaling")
if "property <float> hover-scale: ta.pressed" in dock_text:
    errors.append("Dock.slint reintroduced mouse-down press scaling")

popouts = (root / "src/clients/crates/gnoblin-runtime/vendor/slint/Popouts.slint").read_text(encoding="utf-8")
panel = (root / "src/clients/crates/gnoblin-runtime/vendor/slint/Panel.slint").read_text(encoding="utf-8")
tokens_text = (root / "src/clients/crates/gnoblin-runtime/vendor/slint/Tokens.slint").read_text(encoding="utf-8")
zoo = root / "src/clients/crates/gnoblin-runtime/vendor/slint/ComponentZoo.slint"
for needle in (
    "export component ShellRoundButton",
    "export component ShellSliderRow",
    "export component ShellTile",
    "export component ShellNotificationCard",
    "export component ShellNotificationStack",
):
    if needle not in popouts:
        errors.append(f"Popouts.slint missing shared shell primitive {needle}")
if "export global ShellPalette" not in tokens_text or "CcPalette" in tokens_text or "CcPalette" in popouts:
    errors.append("Shell primitive palette regressed to control-centre-private CcPalette naming")
if not zoo.exists() or "LayerShellComponentZoo" not in zoo.read_text(encoding="utf-8"):
    errors.append("Layer-shell component zoo is missing")
old_icon_refs = (
    "../assets/icons/account.svg",
    "../assets/icons/network.svg",
    "../assets/icons/wifi.svg",
    "../assets/icons/wifi-high.svg",
    "../assets/icons/wifi-none.svg",
    "../assets/icons/battery.svg",
    "../assets/icons/audio.svg",
    "../assets/icons/bluetooth.svg",
    "../assets/icons/bell.svg",
    "../assets/icons/search.svg",
    "../assets/icons/power.svg",
)
for ref in old_icon_refs:
    if ref in popouts or ref in panel:
        errors.append(f"quick settings/topbar still reference legacy icon {ref}")
for ref in (
    "../assets/icons/symbolic/ethernet.svg",
    "../assets/icons/symbolic/wifi.svg",
    "../assets/icons/symbolic/wifi-off.svg",
    "../assets/icons/symbolic/volume.svg",
    "../assets/icons/symbolic/bluetooth.svg",
    "../assets/icons/symbolic/bell-off.svg",
    "../assets/icons/symbolic/apps.svg",
):
    if ref not in popouts + panel:
        errors.append(f"quick settings/topbar missing symbolic icon {ref}")

tokens = (root / "src/clients/crates/gnoblin-runtime/vendor/slint/Tokens.slint").read_text(encoding="utf-8")
for needle in (
    "dock-corner-radius",
    "menu-corner-radius",
    "popout-corner-radius",
    "tooltip-corner-radius",
    "control-corner-radius",
    "dock-shadow-blur",
    "menu-shadow-blur",
    "popout-shadow-blur",
    "tooltip-shadow-blur",
    "control-shadow-blur",
    "window-shadow-blur",
):
    if needle not in tokens:
        errors.append(f"Tokens.slint missing configurable Slint chrome token {needle}")

for needle in (
    "motion-fast-ms",
    "motion-medium-ms",
    "motion-overlay-open-ms",
    "motion-overlay-close-ms",
    "motion-fast-style",
    "motion-medium-style",
    "motion-overlay-style",
    "motion-overlay-slide-value",
    "motion-overlay-scale-from-value",
    "motion-overlay-open-duration",
    "motion-overlay-close-duration",
    "fast-curve",
    "medium-curve",
    "overlay-curve",
):
    if needle not in tokens:
        errors.append(f"Tokens.slint missing configurable Slint motion token {needle}")

for path in sorted((root / "src/clients").glob("**/*.slint")):
    if "/target/" in path.as_posix() or path.name == "Tokens.slint":
        continue
    lines = path.read_text(encoding="utf-8").splitlines()
    for idx, line in enumerate(lines, start=1):
        if re.search(r"duration:\s*[0-9]+ms", line):
            errors.append(
                f"{path.relative_to(root)}:{idx}: Slint animation duration bypasses motion tokens"
            )
        for band in ("fast", "medium", "overlay"):
            if f"duration: Motion.{band}" not in line:
                continue
            window = "\n".join(lines[idx - 1 : min(len(lines), idx + 4)])
            if f"Motion.{band}-curve" not in window:
                errors.append(
                    f"{path.relative_to(root)}:{idx}: Motion.{band} duration does not use Motion.{band}-curve"
                )

for path in sorted((root / "src/clients").glob("*/src/main.rs")):
    if path.match("*/gnoblin-runtime/*"):
        continue
    text = path.read_text(encoding="utf-8")
    if "set_motion_scale(" in text and "apply_shell_motion_to_theme!" not in text:
        errors.append(
            f"{path.relative_to(root)}: client sets only motion_scale instead of full Slint motion"
        )

if errors:
    print("\n".join(errors))
    sys.exit(1)
PY
then
  ok "Slint chrome shadows and animation tokens stay centralized"
else
  bad "Slint chrome shadows or animation tokens regressed"
fi

echo "== patches apply cleanly to the pinned tags =="
declare -A TAGS=( [mutter]=49.5 [gnome-shell]=49.6 [gnome-control-center]=49.6 )
for proj in mutter gnome-shell gnome-control-center; do
  sm="$ROOT/subprojects/$proj"
  [ -d "$sm/.git" ] || { bad "$proj submodule missing (run 'just init')"; continue; }
  reset_to_tag "$sm" "${TAGS[$proj]}"
  while IFS= read -r p; do
    if git -C "$sm" apply --check "$p" 2>/dev/null; then ok "${p#$ROOT/}"; else bad "${p#$ROOT/}"; fi
  done < <(find "$ROOT/patches/$proj" -name '*.patch' | sort)
done

echo "== applying mutter overlay + patches for content checks =="
sm="$ROOT/subprojects/mutter"
reset_to_tag "$sm" 49.5
"$ROOT/scripts/copy-overlay.sh" mutter "$sm" >/dev/null
while IFS= read -r p; do git -C "$sm" apply "$p" 2>/dev/null; done \
  < <(find "$ROOT/patches/mutter" -name '*.patch' | sort)

echo "== overlay-key schema default is empty =="
schema="$sm/data/org.gnome.mutter.gschema.xml.in"
if grep -Pzo "(?s)name=\"overlay-key\".*?<default>''</default>" "$schema" >/dev/null 2>&1; then
  ok "overlay-key default = ''"
else
  bad "overlay-key default is not ''"
fi

pointer="$sm/src/wayland/meta-wayland-pointer.c"
if [ -f "$pointer" ] && python3 - "$pointer" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
try:
    set_focus = text.split("meta_wayland_pointer_set_focus (MetaWaylandPointer *pointer,", 1)[1]
    set_focus = set_focus.split("void\nmeta_wayland_pointer_focus_surface", 1)[0]
    update_cursor = text.split("meta_wayland_pointer_update_cursor_surface (MetaWaylandPointer *pointer)", 1)[1]
    update_cursor = update_cursor.split("static void\nensure_update_cursor_surface", 1)[0]
except IndexError:
    sys.exit(1)

focus_needles = (
    "struct wl_client *old_client = NULL;",
    "struct wl_client *new_client = NULL;",
    "client_changed = old_client != new_client;",
    "client_changed && pointer->button_count == 0",
    "pointer->cursor_surface = NULL;",
    "pointer->cursor_shape = META_CURSOR_INVALID;",
    "g_clear_object (&pointer->shape_sprite);",
)
update_needles = (
    "!pointer->current && pointer->button_count == 0",
    "surface = NULL;",
)
if any(needle not in set_focus for needle in focus_needles):
    sys.exit(1)
if any(needle not in update_cursor for needle in update_needles):
    sys.exit(1)
PY
then
  ok "Wayland pointer clears stale client cursors on idle no-surface and client-focus changes"
else
  bad "Wayland pointer cursor reset for idle no-surface/client-focus changes is missing"
fi

echo "== wlr-layer-shell protocol XML lints + scans =="
xml="$sm/src/wayland/protocol/wlr-layer-shell-unstable-v1.xml"
if [ -f "$xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$xml" 2>/dev/null && ok "XML well-formed" || bad "XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$xml" "$tmp" 2>/dev/null && \
       grep -q "zwlr_layer_shell_v1_interface" "$tmp"; then
      ok "wayland-scanner generates zwlr_layer_shell_v1 server header"
    else
      bad "wayland-scanner failed on layer-shell XML"
    fi
    rm -f "$tmp"
  fi
else
  echo "  skip  layer-shell XML not present yet"
fi
if [ -f "$sm/src/wayland/meta-wayland-layer-shell.c" ] &&
   grep -q "meta_window_recalc_features" "$sm/src/wayland/meta-wayland-layer-shell.c" &&
   grep -q "meta_window_on_all_workspaces_changed" "$sm/src/wayland/meta-wayland-layer-shell.c" &&
   grep -q "meta_window_update_visibility" "$sm/src/wayland/meta-wayland-layer-shell.c" &&
   grep -q "META_WAYLAND_LAYER_SHELL_KEYBOARD_FOCUSABLE_KEY" "$sm/src/wayland/meta-wayland-layer-shell.c" &&
   grep -q "window->input = TRUE" "$sm/src/wayland/meta-wayland-layer-shell.c" &&
   grep -q "CLOSED_LAYER_WINDOW_DESTROY_DELAY_MS" "$sm/src/wayland/meta-wayland-layer-shell.c" &&
   grep -q "g_timeout_add_full" "$sm/src/wayland/meta-wayland-layer-shell.c" &&
   grep -q "META_WAYLAND_LAYER_SHELL_KEYBOARD_FOCUSABLE_KEY" "$sm/src/wayland/meta-window-wayland.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-layer-shell.c"; then
  ok "layer-shell surfaces use sticky semantics, visibility updates, pointer input, delayed closed cleanup, and settings-gated protocol startup"
else
  bad "layer-shell surfaces do not refresh sticky semantics, visibility, pointer input, delayed closed cleanup, or protocol settings correctly"
fi
screencopy_xml="$sm/src/wayland/protocol/wlr-screencopy-unstable-v1.xml"
if [ -f "$screencopy_xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$screencopy_xml" 2>/dev/null && ok "screencopy XML well-formed" || bad "screencopy XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$screencopy_xml" "$tmp" 2>/dev/null && \
       grep -q "zwlr_screencopy_manager_v1_interface" "$tmp"; then
      ok "wayland-scanner generates zwlr_screencopy server header"
    else
      bad "wayland-scanner failed on screencopy XML"
    fi
    rm -f "$tmp"
  fi
else
  bad "screencopy XML not present"
fi
if [ -f "$sm/src/wayland/meta-wayland-screencopy.c" ] &&
   grep -q "meta_wayland_init_screencopy" "$sm/src/wayland/meta-wayland-screencopy.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-screencopy.c" &&
   grep -q "meta_wayland_init_screencopy" "$sm/src/wayland/meta-wayland-surface.c" &&
   grep -q "wlr-screencopy-unstable-v1" "$sm/src/meson.build"; then
  ok "mutter exports settings-gated wlr-screencopy for nested compositor capture"
else
  bad "mutter wlr-screencopy overlay, settings gate, or init wiring is missing"
fi
idle_notify_xml="$sm/src/wayland/protocol/ext-idle-notify-v1.xml"
if [ -f "$idle_notify_xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$idle_notify_xml" 2>/dev/null && ok "idle-notify XML well-formed" || bad "idle-notify XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$idle_notify_xml" "$tmp" 2>/dev/null && \
       grep -q "ext_idle_notifier_v1_interface" "$tmp"; then
      ok "wayland-scanner generates ext_idle_notifier_v1 server header"
    else
      bad "wayland-scanner failed on idle-notify XML"
    fi
    rm -f "$tmp"
  fi
else
  bad "idle-notify XML not present"
fi
if [ -f "$sm/src/wayland/meta-wayland-idle-notify.c" ] &&
   grep -q "meta_wayland_init_idle_notify" "$sm/src/wayland/meta-wayland-idle-notify.c" &&
   grep -q "meta_idle_monitor_add_idle_watch_full" "$sm/src/wayland/meta-wayland-idle-notify.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-idle-notify.c" &&
   grep -q "meta_wayland_init_idle_notify" "$sm/src/wayland/meta-gnoblin-protocols.c" &&
   grep -q "ext-idle-notify-v1" "$sm/src/meson.build"; then
  ok "mutter exports settings-gated ext-idle-notify backed by MetaIdleMonitor"
else
  bad "mutter ext-idle-notify overlay, settings gate, or init wiring is missing"
fi
ftl_xml="$sm/src/wayland/protocol/ext-foreign-toplevel-list-v1.xml"
if [ -f "$ftl_xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$ftl_xml" 2>/dev/null && ok "foreign-toplevel-list XML well-formed" || bad "foreign-toplevel-list XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$ftl_xml" "$tmp" 2>/dev/null && \
       grep -q "ext_foreign_toplevel_list_v1_interface" "$tmp"; then
      ok "wayland-scanner generates ext_foreign_toplevel_list_v1 server header"
    else
      bad "wayland-scanner failed on foreign-toplevel-list XML"
    fi
    rm -f "$tmp"
  fi
else
  bad "foreign-toplevel-list XML not present"
fi
if [ -f "$sm/src/wayland/meta-wayland-foreign-toplevel-list.c" ] &&
   grep -q "meta_wayland_init_foreign_toplevel_list" "$sm/src/wayland/meta-wayland-foreign-toplevel-list.c" &&
   grep -q "meta_display_list_all_windows" "$sm/src/wayland/meta-wayland-foreign-toplevel-list.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-foreign-toplevel-list.c" &&
   grep -q "meta_wayland_init_foreign_toplevel_list" "$sm/src/wayland/meta-gnoblin-protocols.c" &&
   grep -q "ext-foreign-toplevel-list-v1" "$sm/src/meson.build"; then
  ok "mutter exports settings-gated ext-foreign-toplevel-list from the window list"
else
  bad "mutter ext-foreign-toplevel-list overlay, settings gate, or init wiring is missing"
fi
gamma_xml="$sm/src/wayland/protocol/wlr-gamma-control-unstable-v1.xml"
if [ -f "$gamma_xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$gamma_xml" 2>/dev/null && ok "gamma-control XML well-formed" || bad "gamma-control XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$gamma_xml" "$tmp" 2>/dev/null && \
       grep -q "zwlr_gamma_control_manager_v1_interface" "$tmp"; then
      ok "wayland-scanner generates zwlr_gamma_control_manager_v1 server header"
    else
      bad "wayland-scanner failed on gamma-control XML"
    fi
    rm -f "$tmp"
  fi
else
  bad "gamma-control XML not present"
fi
if [ -f "$sm/src/wayland/meta-wayland-gamma-control.c" ] &&
   grep -q "meta_wayland_init_gamma_control" "$sm/src/wayland/meta-wayland-gamma-control.c" &&
   grep -q "meta_crtc_set_gamma_lut" "$sm/src/wayland/meta-wayland-gamma-control.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-gamma-control.c" &&
   grep -q "meta_wayland_init_gamma_control" "$sm/src/wayland/meta-gnoblin-protocols.c" &&
   grep -q "wlr-gamma-control-unstable-v1" "$sm/src/meson.build"; then
  ok "mutter exports settings-gated wlr-gamma-control backed by per-CRTC gamma LUTs"
else
  bad "mutter wlr-gamma-control overlay, settings gate, or init wiring is missing"
fi
ftm_xml="$sm/src/wayland/protocol/wlr-foreign-toplevel-management-unstable-v1.xml"
if [ -f "$ftm_xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$ftm_xml" 2>/dev/null && ok "foreign-toplevel-management XML well-formed" || bad "foreign-toplevel-management XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$ftm_xml" "$tmp" 2>/dev/null && \
       grep -q "zwlr_foreign_toplevel_manager_v1_interface" "$tmp"; then
      ok "wayland-scanner generates zwlr_foreign_toplevel_manager_v1 server header"
    else
      bad "wayland-scanner failed on foreign-toplevel-management XML"
    fi
    rm -f "$tmp"
  fi
else
  bad "foreign-toplevel-management XML not present"
fi
if [ -f "$sm/src/wayland/meta-wayland-foreign-toplevel-management.c" ] &&
   grep -q "meta_wayland_init_foreign_toplevel_management" "$sm/src/wayland/meta-wayland-foreign-toplevel-management.c" &&
   grep -q "meta_window_activate" "$sm/src/wayland/meta-wayland-foreign-toplevel-management.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-foreign-toplevel-management.c" &&
   grep -q "meta_wayland_init_foreign_toplevel_management" "$sm/src/wayland/meta-gnoblin-protocols.c" &&
   grep -q "wlr-foreign-toplevel-management-unstable-v1" "$sm/src/meson.build"; then
  ok "mutter exports settings-gated wlr-foreign-toplevel-management with window control"
else
  bad "mutter wlr-foreign-toplevel-management overlay, settings gate, or init wiring is missing"
fi
opm_xml="$sm/src/wayland/protocol/wlr-output-power-management-unstable-v1.xml"
if [ -f "$opm_xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$opm_xml" 2>/dev/null && ok "output-power-management XML well-formed" || bad "output-power-management XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$opm_xml" "$tmp" 2>/dev/null && \
       grep -q "zwlr_output_power_manager_v1_interface" "$tmp"; then
      ok "wayland-scanner generates zwlr_output_power_manager_v1 server header"
    else
      bad "wayland-scanner failed on output-power-management XML"
    fi
    rm -f "$tmp"
  fi
else
  bad "output-power-management XML not present"
fi
if [ -f "$sm/src/wayland/meta-wayland-output-power-management.c" ] &&
   grep -q "meta_wayland_init_output_power_management" "$sm/src/wayland/meta-wayland-output-power-management.c" &&
   grep -q "set_power_save_mode" "$sm/src/wayland/meta-wayland-output-power-management.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-output-power-management.c" &&
   grep -q "meta_wayland_init_output_power_management" "$sm/src/wayland/meta-gnoblin-protocols.c" &&
   grep -q "wlr-output-power-management-unstable-v1" "$sm/src/meson.build"; then
  ok "mutter exports settings-gated wlr-output-power-management (global DPMS)"
else
  bad "mutter wlr-output-power-management overlay, settings gate, or init wiring is missing"
fi
dctrl_xml="$sm/src/wayland/protocol/ext-data-control-v1.xml"
if [ -f "$dctrl_xml" ]; then
  if command -v xmllint >/dev/null 2>&1; then
    xmllint --noout "$dctrl_xml" 2>/dev/null && ok "data-control XML well-formed" || bad "data-control XML malformed"
  fi
  if command -v wayland-scanner >/dev/null 2>&1; then
    tmp="$(mktemp --suffix=.h)"
    if wayland-scanner server-header "$dctrl_xml" "$tmp" 2>/dev/null && \
       grep -q "ext_data_control_manager_v1_interface" "$tmp"; then
      ok "wayland-scanner generates ext_data_control_manager_v1 server header"
    else
      bad "wayland-scanner failed on data-control XML"
    fi
    rm -f "$tmp"
  fi
else
  bad "data-control XML not present"
fi
if [ -f "$sm/src/wayland/meta-wayland-data-control.c" ] &&
   grep -q "meta_wayland_init_data_control" "$sm/src/wayland/meta-wayland-data-control.c" &&
   grep -q "meta_selection_set_owner" "$sm/src/wayland/meta-wayland-data-control.c" &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/meta-wayland-data-control.c" &&
   grep -q "meta_wayland_init_data_control" "$sm/src/wayland/meta-gnoblin-protocols.c" &&
   grep -q "ext-data-control-v1" "$sm/src/meson.build"; then
  ok "mutter exports settings-gated ext-data-control backed by MetaSelection"
else
  bad "mutter ext-data-control overlay, settings gate, or init wiring is missing"
fi
if [ -f "$sm/src/wayland/meta-gnoblin-protocols.c" ] &&
   grep -q "meta_gnoblin_init_protocols" "$sm/src/wayland/meta-gnoblin-protocols.c" &&
   grep -q "meta_gnoblin_init_protocols" "$sm/src/wayland/meta-wayland-surface.c"; then
  ok "gnoblin protocols are registered through a single aggregated shell-init hook"
else
  bad "gnoblin protocol aggregator overlay or shell-init wiring is missing"
fi
if [ -f "$sm/src/wayland/gnoblin-config.c" ] &&
   grep -q "gnoblin_config_get_bool" "$sm/src/wayland/gnoblin-config.c" &&
   grep -q "gnoblin_config_get_keys" "$sm/src/wayland/gnoblin-config.c" &&
   grep -q "gnoblin-config.c" "$sm/src/meson.build" &&
   ! grep -rq "gnoblin_settings_boolean" "$sm/src/wayland/meta-wayland-data-control.c"; then
  ok "gnoblin protocols are gated by the gnoblin config file, not GSettings"
else
  bad "gnoblin config-file overlay is missing or protocols still use GSettings"
fi
if grep -q "uninstalled_variables" "$sm/src/meson.build" &&
   grep -Fq "'typelibdir=\${prefix}/src'" "$sm/src/meson.build" &&
   grep -q "'have_fonts=@0@'.format(have_fonts)" "$sm/src/meson.build"; then
  ok "mutter uninstalled pkg-config metadata supports gnome-shell rebuilds"
else
  bad "mutter uninstalled pkg-config metadata is missing shell-required variables"
fi

reset_to_tag "$sm" 49.5

echo "== applying gnome-control-center patches for content checks =="
sm="$ROOT/subprojects/gnome-control-center"
if [ -d "$sm/.git" ]; then
  reset_to_tag "$sm" 49.6
  "$ROOT/scripts/copy-overlay.sh" gnome-control-center "$sm" >/dev/null
  while IFS= read -r p; do git -C "$sm" apply "$p" 2>/dev/null; done \
    < <(find "$ROOT/patches/gnome-control-center" -name '*.patch' | sort)

  gnoblin_panel="$sm/panels/gnoblin/cc-gnoblin-panel.c"
  gnoblin_panel_ui="$sm/panels/gnoblin/cc-gnoblin-panel.blp"
  gnoblin_panel_meson="$sm/panels/gnoblin/meson.build"
  gnoblin_panel_desktop="$sm/panels/gnoblin/gnome-gnoblin-panel.desktop.in"
  gnoblin_panel_loader="$sm/shell/cc-panel-loader.c"
  gnoblin_panels_meson="$sm/panels/meson.build"
  if [ -f "$gnoblin_panel" ] &&
     [ -f "$gnoblin_panel_ui" ] &&
     [ -f "$gnoblin_panel_meson" ] &&
     grep -q "cc_gnoblin_panel_get_type" "$gnoblin_panel_loader" &&
     grep -q "PANEL_TYPE(\"gnoblin\"" "$gnoblin_panel_loader" &&
     grep -q "'gnoblin'" "$gnoblin_panels_meson" &&
     grep -q "panels_list += cappletname" "$gnoblin_panel_meson" &&
     grep -q "desktop = 'gnome-@0@-panel.desktop'.format(cappletname)" "$gnoblin_panel_meson" &&
     grep -q "cc-gnoblin-panel.blp" "$gnoblin_panel_meson" &&
     grep -q "org.gnome.Settings-gnoblin-symbolic" "$gnoblin_panel_desktop"; then
    ok "gnome-control-center exposes the Gnoblin Settings panel"
  else
    bad "gnome-control-center Gnoblin panel registration is incomplete"
  fi
  if grep -q "GNOBLIN_SCHEMA_ID \"org.gnoblin.shell\"" "$gnoblin_panel" &&
     grep -q "\"topbar-enabled\"" "$gnoblin_panel" &&
     grep -q "\"topbar-monitor-mode\"" "$gnoblin_panel" &&
     grep -q "\"topbar-monitors\"" "$gnoblin_panel" &&
     grep -q "\"dock-enabled\"" "$gnoblin_panel" &&
     grep -q "\"dock-monitor-mode\"" "$gnoblin_panel" &&
     grep -q "\"dock-monitors\"" "$gnoblin_panel" &&
     grep -q "\"wlr-layer-shell-enabled\"" "$gnoblin_panel" &&
     grep -q "\"wlr-screencopy-enabled\"" "$gnoblin_panel" &&
     grep -q "g_settings_schema_source_lookup" "$gnoblin_panel"; then
    ok "Gnoblin Settings panel edits the shared feature schema and guards missing schemas"
  else
    bad "Gnoblin Settings panel is not bound to the shared feature schema"
  fi
  if grep -q "Primary Display" "$gnoblin_panel_ui" &&
     grep -q "All Displays" "$gnoblin_panel_ui" &&
     grep -q "Selected Displays" "$gnoblin_panel_ui" &&
     grep -q "topbar_displays_group" "$gnoblin_panel_ui" &&
     grep -q "dock_displays_group" "$gnoblin_panel_ui" &&
     grep -q "gdk_display_get_monitors" "$gnoblin_panel" &&
     grep -q "monitor_id" "$gnoblin_panel" &&
     grep -q "monitor-%u-%d,%d-%dx%d" "$gnoblin_panel"; then
    ok "Gnoblin Settings panel supports primary/all/manual per-monitor topbar and dock policy"
  else
    bad "Gnoblin Settings panel is missing per-monitor topbar/dock controls"
  fi
  reset_to_tag "$sm" 49.6
else
  bad "gnome-control-center submodule missing (run 'just init')"
fi

echo "== repo-owned Rust/Slint shell clients are wired correctly =="
clients="$ROOT/src/clients"
cargo_toml="$clients/Cargo.toml"
runtime_loop="$clients/crates/gnoblin-runtime/src/layer_shell_runtime.rs"
core_lib="$clients/crates/gnoblin-core/src/lib.rs"
if [ -f "$cargo_toml" ] &&
   grep -q '"crates/gnoblin-core"' "$cargo_toml" &&
   grep -q '"crates/gnoblin-desktop"' "$cargo_toml" &&
   grep -q '"crates/gnoblin-runtime"' "$cargo_toml" &&
   grep -q '"topbar"' "$cargo_toml" &&
   grep -q '"dock"' "$cargo_toml"; then
  ok "client workspace declares split shared crates + topbar/dock members"
else
  bad "client workspace is missing split shared crates or topbar/dock members"
fi
if grep -q "gnoblin_runtime::.*BarApp" "$clients/topbar/src/main.rs" &&
   grep -q "gnoblin_runtime::.*BarConfig" "$clients/topbar/src/main.rs" &&
   grep -q "gnoblin_runtime::.*run" "$clients/topbar/src/main.rs" &&
   grep -q "gnoblin_runtime::.*BarApp" "$clients/dock/src/main.rs" &&
   grep -q "gnoblin_runtime::.*BarConfig" "$clients/dock/src/main.rs" &&
   grep -q "gnoblin_runtime::.*run" "$clients/dock/src/main.rs" &&
   grep -q "impl BarApp" "$clients/topbar/src/main.rs" &&
   grep -q "impl BarApp" "$clients/dock/src/main.rs" &&
   grep -q "smithay_client_toolkit::shell::wlr_layer" "$clients/topbar/src/main.rs" &&
   grep -q "smithay_client_toolkit::shell::wlr_layer" "$clients/dock/src/main.rs" &&
   [ -f "$clients/topbar/ui/topbar.slint" ] &&
   [ -f "$clients/dock/ui/dock.slint" ]; then
  ok "topbar and dock are Slint wlr-layer-shell clients on the shared runtime"
else
  bad "topbar/dock are not Slint wlr-layer-shell clients on the shared runtime"
fi

plugin="$ROOT/src/compositor/gnoblin-shell-plugin.cpp"
anim="$ROOT/src/compositor/gnoblin-anim.cpp"
example_conf="$ROOT/src/data/gnoblin.conf.example"
open_anim_body="$(awk '
  /static gboolean wants_window_open_animation/ { flag = 1 }
  /static gboolean wants_window_close_animation/ { exit }
  flag { print }
' "$plugin" 2>/dev/null)"
close_anim_body="$(awk '
  /static gboolean wants_window_close_animation/ { flag = 1 }
  /\/\* After any stacking change/ { exit }
  flag { print }
' "$plugin" 2>/dev/null)"
if [ -n "$open_anim_body" ] &&
   printf '%s\n' "$open_anim_body" | grep -q "META_WINDOW_MENU" &&
   printf '%s\n' "$open_anim_body" | grep -q "META_WINDOW_DROPDOWN_MENU" &&
   printf '%s\n' "$open_anim_body" | grep -q "META_WINDOW_POPUP_MENU" &&
   printf '%s\n' "$open_anim_body" | grep -q "META_WINDOW_COMBO" &&
   [ -n "$close_anim_body" ] &&
   ! printf '%s\n' "$close_anim_body" | grep -q "META_WINDOW_MENU" &&
   ! printf '%s\n' "$close_anim_body" | grep -q "META_WINDOW_DROPDOWN_MENU" &&
   ! printf '%s\n' "$close_anim_body" | grep -q "META_WINDOW_POPUP_MENU" &&
   ! printf '%s\n' "$close_anim_body" | grep -q "META_WINDOW_COMBO" &&
   grep -q "open.menu" "$anim" &&
   grep -q "open.normal" "$anim" &&
   grep -q "^open.menu = 80, ease-out-quad, 0.995" "$example_conf" &&
   grep -q "^open.popup-menu = 80, ease-out-quad, 0.995" "$example_conf" &&
   grep -q "close animations" "$example_conf"; then
  ok "window animations target by type: menus open minimally while destroy stays toplevel-safe"
else
  bad "window animation type targeting is missing menu open support or destroy safety"
fi
if python3 - "$plugin" "$anim" "$example_conf" <<'PY'
import pathlib
import sys

plugin = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
anim = pathlib.Path(sys.argv[2]).read_text(encoding="utf-8")
conf = pathlib.Path(sys.argv[3]).read_text(encoding="utf-8")
try:
    # Two-phase GNOME-style cross-fade: size_change (prepare) freezes the live
    # actor + captures a freeze-frame clone; size_changed (run) animates + thaws.
    prepare = plugin.split("static void gnoblin_shell_plugin_size_change", 1)[1]
    prepare = prepare.split("static void gnoblin_shell_plugin_size_changed", 1)[0]
    run = plugin.split("static void gnoblin_shell_plugin_size_changed", 1)[1]
    run = run.split("/* ---- cancellation ---- */", 1)[0]
except IndexError:
    sys.exit(1)

# Prepare freezes the client surface and snapshots the old contents into a clone.
for needle in (
    "meta_window_actor_freeze(window_actor);",
    "meta_window_actor_paint_to_content(",
    "state->resize_frozen = TRUE;",
):
    if needle not in prepare:
        sys.exit(1)
# Run drives the transform animation; completion (or the run) thaws the surface.
if "animate(actor," not in run:
    sys.exit(1)
if "meta_window_actor_thaw(window_actor);" not in plugin:
    sys.exit(1)
if "finish_size_change" not in plugin:
    sys.exit(1)
for needle in (
    "a->duration_ms = !g_strcmp0(effect, \"unmaximize\") ? 280 : 300;",
    "a->mode = CLUTTER_EASE_OUT_QUART;",
):
    if needle not in anim:
        sys.exit(1)
for needle in (
    "maximize = 300, ease-out-quart",
    "unmaximize = 280, ease-out-quart",
):
    if needle not in conf:
        sys.exit(1)
PY
then
  ok "maximize/unmaximize frame zoom freezes client surfaces and uses smooth defaults"
else
  bad "maximize/unmaximize frame zoom is missing surface freeze/thaw or smooth defaults"
fi
# The shared run loop must commit at most once per wl_surface frame callback.
# Committing twice aborts mutter on frame_callback_list, while failing to request
# a frame callback stalls Slint animations until unrelated input arrives.
if [ -f "$runtime_loop" ] &&
   grep -q "pub trait BarApp" "$runtime_loop" &&
   grep -q "frame_pending" "$runtime_loop" &&
   grep -q "ready_to_render" "$runtime_loop" &&
   grep -q "has_active_animations" "$runtime_loop" &&
   grep -q "update_timers_and_animations" "$runtime_loop" &&
   grep -q "next_dispatch_timeout" "$runtime_loop" &&
   grep -q "surface.frame(&self.qh" "$runtime_loop" &&
   ! grep -q "stale_ticks" "$runtime_loop"; then
  ok "runtime run loop gates rendering, pumps Slint animations, and requests frame callbacks"
else
  bad "runtime run loop is missing Slint animation/frame callback pacing"
fi

if python3 - "$runtime_loop" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
try:
    try_run = text.split("fn try_run(config: BarConfig", 1)[1]
    state_init = try_run.index("let mut state = State")
    precommit_roundtrip = try_run.index("initial input registry roundtrip")
    initial_commit = try_run.index("state.layer.commit();")
except (IndexError, ValueError):
    sys.exit(1)

if not (state_init < precommit_roundtrip < initial_commit):
    sys.exit(1)
if "layer.commit();\n\n    let mut state = State" in try_run:
    sys.exit(1)
PY
then
  ok "runtime binds input devices before the initial layer-surface commit"
else
  bad "runtime can still commit keyboard layer surfaces before wl_keyboard is bound"
fi

if [ -f "$runtime_loop" ] &&
   [ -f "$core_lib" ] &&
   grep -q "fn try_run(config: BarConfig" "$runtime_loop" &&
   grep -q "fn setup_egl(" "$runtime_loop" &&
   grep -q "Result<EglState, RuntimeError>" "$runtime_loop" &&
   grep -q "pub type RuntimeError" "$core_lib" &&
   grep -q "fn runtime_error" "$core_lib" &&
   grep -q "fn window(&self) -> Option<&slint::Window>" "$runtime_loop" &&
   ! grep -q 'expect("connect to wayland")' "$runtime_loop" &&
   ! grep -q 'expect("registry init")' "$runtime_loop" &&
   ! grep -q 'expect("wlr-layer-shell")' "$runtime_loop" &&
   ! grep -q 'expect("EGL:' "$runtime_loop" &&
   ! grep -q 'expect("FemtoVGRenderer::new")' "$runtime_loop"; then
  ok "runtime layer-shell startup reports errors instead of panicking"
else
  bad "runtime layer-shell startup still has panic-prone setup paths"
fi

slint_layer_clients=(
  "$clients/topbar/src/main.rs"
  "$clients/dock/src/main.rs"
  "$clients/launcher/src/main.rs"
  "$clients/notifyd/src/main.rs"
  "$clients/osd/src/main.rs"
  "$clients/window-menu/src/main.rs"
  "$clients/power-menu/src/main.rs"
)
slint_layer_ok=1
for client_src in "${slint_layer_clients[@]}"; do
  if [ ! -f "$client_src" ] ||
     ! grep -q "Result<(), RuntimeError>" "$client_src" ||
     ! grep -q "fn window(&self) -> Option<&slint::Window>" "$client_src" ||
     grep -q "expect(" "$client_src"; then
    slint_layer_ok=0
  fi
done
if [ "$slint_layer_ok" -eq 1 ]; then
  ok "Slint layer clients report component/show failures instead of panicking"
else
  bad "Slint layer clients still have panic-prone component/show paths"
fi

if python3 - "$clients" <<'PY'
import pathlib
import re
import sys

clients = pathlib.Path(sys.argv[1])
panic_call = re.compile(r"(?:\.(?:unwrap|expect)\s*\(|\b(?:panic|todo|unimplemented)!\s*\()")
errors = []


def code_without_line_comment(line: str) -> str:
    out = []
    quote = None
    escaped = False
    i = 0
    while i < len(line):
        ch = line[i]
        nxt = line[i + 1] if i + 1 < len(line) else ""
        if quote:
            out.append(" ")
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == quote:
                quote = None
            i += 1
            continue
        if ch == '"':
            quote = ch
            out.append(" ")
        elif ch == "/" and nxt == "/":
            break
        else:
            out.append(ch)
        i += 1
    return "".join(out)


def strip_strings_and_comments(line: str, block_comment: bool) -> tuple[str, bool]:
    out = []
    quote = None
    escaped = False
    i = 0
    while i < len(line):
        ch = line[i]
        nxt = line[i + 1] if i + 1 < len(line) else ""
        if block_comment:
            if ch == "*" and nxt == "/":
                block_comment = False
                i += 2
            else:
                i += 1
            out.append(" ")
            continue
        if quote:
            out.append(" ")
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == quote:
                quote = None
            i += 1
            continue
        if ch == "/" and nxt == "/":
            break
        if ch == "/" and nxt == "*":
            block_comment = True
            out.append(" ")
            i += 2
            continue
        if ch == '"':
            quote = ch
            out.append(" ")
        else:
            out.append(ch)
        i += 1
    return "".join(out), block_comment


def runtime_lines(lines: list[str]) -> list[tuple[int, str]]:
    runtime = []
    i = 0
    while i < len(lines):
        stripped = lines[i].strip()
        if stripped.startswith("#[cfg(") and "test" in stripped:
            i += 1
            while i < len(lines) and lines[i].strip().startswith("#["):
                i += 1
            depth = 0
            saw_brace = False
            block_comment = False
            while i < len(lines):
                code, block_comment = strip_strings_and_comments(lines[i], block_comment)
                if "{" in code:
                    saw_brace = True
                depth += code.count("{") - code.count("}")
                i += 1
                if saw_brace:
                    if depth <= 0 and not block_comment:
                        break
                else:
                    break
            continue
        runtime.append((i + 1, lines[i]))
        i += 1
    return runtime


for path in sorted(clients.rglob("*.rs")):
    if path.name == "build.rs" or "/target/" in path.as_posix():
        continue
    lines = path.read_text(encoding="utf-8").splitlines()
    for lineno, line in runtime_lines(lines):
        if panic_call.search(code_without_line_comment(line)):
            errors.append(f"{path.relative_to(clients)}:{lineno}: {line.strip()}")

if errors:
    print("\n".join(errors))
    sys.exit(1)
PY
then
  ok "Rust client runtime paths avoid bare panic/unwrap/expect"
else
  bad "Rust client runtime paths still contain panic-prone calls"
fi

wallpaper_client="$clients/wallpaper/src/main.rs"
night_light_client="$clients/night-light/src/main.rs"
if [ -f "$wallpaper_client" ] &&
   [ -f "$night_light_client" ] &&
   grep -q "fn try_main() -> Result<(), RuntimeError>" "$wallpaper_client" &&
   grep -q "state.draw()?" "$wallpaper_client" &&
   grep -q "fn try_main() -> Result<(), RuntimeError>" "$night_light_client" &&
   grep -q "queue.roundtrip(&mut state).is_err()" "$night_light_client" &&
   ! grep -q "expect(" "$wallpaper_client" &&
   ! grep -q "expect(" "$night_light_client"; then
  ok "non-Slint layer clients report startup/draw errors instead of panicking"
else
  bad "wallpaper or night-light still has panic-prone startup/draw paths"
fi

if python3 "$ROOT/scripts/devkit_crash_detector.py" >/dev/null; then
  ok "devkit harness crash detector catches critical/runtime failures without flagging benign teardown"
else
  bad "devkit harness crash detector signatures regressed"
fi

echo
[ "$fail" -eq 0 ] && { echo "TIER 1: PASS"; exit 0; } || { echo "TIER 1: FAIL"; exit 1; }
