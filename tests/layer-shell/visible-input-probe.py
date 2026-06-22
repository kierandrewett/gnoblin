#!/usr/bin/env python3
# Runs inside `scripts/run-devkit.sh visible ...`, after the virtual monitor
# exists. It uses the visible Devkit session's private Mutter RemoteDesktop API
# to prove keyboard input reaches gnoblin-shell keybindings and Slint clients.
import os
import pathlib
import sys
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib


KEYS = {
    "super": 125,
    "space": 57,
    "return": 28,
    "escape": 1,
    **{c: kc for c, kc in zip("qwertyuiop", [16, 17, 18, 19, 20, 21, 22, 23, 24, 25])},
    **{c: kc for c, kc in zip("asdfghjkl", [30, 31, 32, 33, 34, 35, 36, 37, 38])},
    **{c: kc for c, kc in zip("zxcvbnm", [44, 45, 46, 47, 48, 49, 50])},
}


def proxy(name, path, iface):
    return Gio.DBusProxy.new_for_bus_sync(
        Gio.BusType.SESSION, Gio.DBusProxyFlags.NONE, None, name, path, iface, None
    )


def install_foot_desktop():
    appdir = pathlib.Path(os.environ["XDG_DATA_HOME"]) / "applications"
    appdir.mkdir(parents=True, exist_ok=True)
    (appdir / "foot.desktop").write_text(
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=Foot\n"
        "Exec=foot\n"
        "Icon=foot\n"
        "Terminal=false\n"
    )


def start_remote_desktop():
    rd = proxy(
        "org.gnome.Mutter.RemoteDesktop",
        "/org/gnome/Mutter/RemoteDesktop",
        "org.gnome.Mutter.RemoteDesktop",
    )
    session_path = rd.call_sync("CreateSession", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[
        0
    ]
    session = proxy(
        "org.gnome.Mutter.RemoteDesktop",
        session_path,
        "org.gnome.Mutter.RemoteDesktop.Session",
    )
    session.call_sync("Start", None, Gio.DBusCallFlags.NONE, -1, None)
    return session


def key(session, keycode, pressed):
    session.call_sync(
        "NotifyKeyboardKeycode",
        GLib.Variant("(ub)", (keycode, pressed)),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    )


def send_combo(session, combo):
    codes = [KEYS[part.strip().lower()] for part in combo.split("+")]
    for code in codes:
        key(session, code, True)
        time.sleep(0.01)
    for code in reversed(codes):
        key(session, code, False)
        time.sleep(0.01)


def type_text(session, text):
    for ch in text.lower():
        code = KEYS[ch]
        key(session, code, True)
        time.sleep(0.01)
        key(session, code, False)
        time.sleep(0.02)


def display_processes(needle):
    display_marker = f"WAYLAND_DISPLAY={os.environ['WAYLAND_DISPLAY']}".encode()
    found = []
    for proc in pathlib.Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            env = (proc / "environ").read_bytes()
            if display_marker not in env:
                continue
            cmd = (proc / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if needle in cmd:
            found.append(f"{proc.name} {cmd.strip()}")
    return found


def shell_windows():
    shell = proxy("dev.gnoblin.Shell", "/dev/gnoblin/Shell", "dev.gnoblin.Shell")
    return shell.call_sync("ListWindows", None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]


def wait_until(label, predicate, timeout):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.25)
    raise RuntimeError(f"timed out waiting for {label}; last={last!r}")


def open_launcher(session):
    launcher = None
    for _ in range(3):
        send_combo(session, "Super+Space")
        try:
            launcher = wait_until(
                "gnoblin-launcher after Super+Space",
                lambda: display_processes("gnoblin-launcher"),
                3,
            )
            break
        except RuntimeError:
            continue
    if not launcher:
        raise RuntimeError("Super+Space did not spawn gnoblin-launcher")

    # In the visible Mutter Devkit, process creation can precede the launcher's
    # layer-surface configure, wl_keyboard binding, and keyboard enter by a few
    # hundred milliseconds. Typing as soon as /proc shows the process races that
    # startup path and drops the synthetic keys before the client can receive
    # them.
    time.sleep(0.75)
    return launcher


def close_launcher(session):
    send_combo(session, "Escape")
    wait_until("launcher to close after Escape", lambda: not display_processes("gnoblin-launcher"), 5)


def main():
    marker = os.environ.get("GNOBLIN_VISIBLE_INPUT_MARKER")
    if not marker:
        raise RuntimeError("GNOBLIN_VISIBLE_INPUT_MARKER is required")

    install_foot_desktop()
    session = start_remote_desktop()
    time.sleep(0.5)

    cycles = int(os.environ.get("GNOBLIN_VISIBLE_INPUT_CYCLES", "0"))
    idle_seconds = float(os.environ.get("GNOBLIN_VISIBLE_INPUT_IDLE_SECONDS", "0"))
    for i in range(cycles):
        open_launcher(session)
        type_text(session, "foot")
        close_launcher(session)
        if idle_seconds > 0:
            time.sleep(idle_seconds)

    launcher = open_launcher(session)
    type_text(session, "foot")
    send_combo(session, "Return")

    try:
        foot = wait_until(
            "foot toplevel after launcher Return",
            lambda: [w for w in shell_windows() if w[2] == "foot" and not w[4]],
            10,
        )
    except RuntimeError as exc:
        raise RuntimeError(
            f"{exc}; windows={shell_windows()!r}; "
            f"launchers={display_processes('gnoblin-launcher')!r}"
        )
    wait_until("launcher to close after activation", lambda: not display_processes("gnoblin-launcher"), 5)

    pathlib.Path(marker).write_text("ok")
    print(f"visible input probe: launcher={launcher[0]} foot={foot[0]}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"visible input probe failed: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
