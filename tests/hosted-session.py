#!/usr/bin/env python3
"""Run `gnoblin --hosted` as the current user and drive it the way a session manager does.

usage: scripts/run-staged.sh PREFIX STAGE python3 tests/hosted-session.py [--evidence DIR]

The test starts a private session bus, a private PipeWire daemon and `gnoblin --hosted` below a
temporary XDG_RUNTIME_DIR. It never touches the desktop it runs in. The system bus address points at
a missing socket, so nothing inside can reach logind.

Checks:
  * the command line rejects invalid hosted invocations;
  * org.gnoblin.Hosted1 reports Ready only after the runtime handshake, and no viewer is started;
  * two virtual monitors are created through ScreenCast RecordVirtual and their frames reach a
    consumer, with layer-shell and toplevel clients visible in the pixels;
  * DisplayConfig positions the monitors and sets a logical scale;
  * RemoteDesktop is reachable and hands out an EIS socket;
  * layers.list and the layer lifecycle events report namespace, output and mapped state;
  * lock, logout and power requests are reported to the session manager instead of acting.

Needs: dbus-daemon, pipewire and its pw-dump and pw-link tools, python-gobject, Pillow,
wayland-scanner, a C compiler with libpipewire and wayland-client headers, foot.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time

from gi.repository import Gio, GLib  # noqa: E402
from PIL import Image  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
DISPLAY = "gnoblin-hosted-test"
API = {"major": 1, "minor": 81}
failures: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> bool:
    print(("PASS" if ok else "FAIL") + f": {name}" + (f" ({detail})" if detail and not ok else ""), flush=True)
    if not ok:
        failures.append(name)
    return ok


def note(text: str) -> None:
    print(f"INFO: {text}", flush=True)


def pump(seconds: float = 0.0, until=None, timeout: float = 20.0):
    """Run the GLib main context until `until()` is truthy or `seconds` have passed."""
    context = GLib.MainContext.default()
    deadline = time.monotonic() + (timeout if until else seconds)
    while time.monotonic() < deadline:
        while context.iteration(False):
            pass
        if until:
            value = until()
            if value:
                return value
        time.sleep(0.02)
    return until() if until else None


class Control:
    """One connection to the compositor control socket (docs/bridge/connection.md)."""

    def __init__(self, path: str):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(10)
        self.sock.connect(path)
        self.buffer = b""
        self.events: list[dict] = []
        self.greeting = self.read()
        self.counter = 0

    def read(self) -> dict:
        while b"\n" not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("control socket closed")
            self.buffer += chunk
        line, _, self.buffer = self.buffer.partition(b"\n")
        return json.loads(line)

    def send(self, message: dict) -> None:
        self.sock.sendall(json.dumps(message).encode() + b"\n")

    def request(self, method: str, arguments: dict | None = None) -> dict:
        self.counter += 1
        ident = f"r{self.counter}"
        self.send({"op": "api", "id": ident, "api_version": API, "method": method, "arguments": arguments or {}})
        while True:
            message = self.read()
            if message.get("id") == ident:
                return message
            self.events.append(message)

    def subscribe(self, names: list[str]) -> dict:
        self.send({"op": "events", "api_version": API, "events": names})
        return self.read()

    def drain(self, timeout: float = 0.2) -> None:
        self.sock.settimeout(timeout)
        try:
            while True:
                self.events.append(self.read())
        except (socket.timeout, TimeoutError):
            pass
        finally:
            self.sock.settimeout(10)


class Consumer:
    """A ScreenCast consumer: tests/hosted-capture.c reading one PipeWire video node.

    A desktop session manager (WirePlumber) normally links a screen-cast stream to its consumer.
    The private PipeWire instance of this test has none, so the consumer links the ports itself."""

    def __init__(self, tool: Path, node: int, width: int, height: int, env: dict, directory: Path):
        self.producer = node
        self.size = (width, height)
        self.directory = directory
        self.lock = threading.Lock()
        self.frames = 0
        self.node: int | None = None
        self.streaming = False
        self.linked = False
        self.dumped = threading.Event()
        self.serial = 0
        self.errors: list[str] = []
        self.process = subprocess.Popen(
            [str(tool), str(width), str(height)],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        threading.Thread(target=self.read_stdout, daemon=True).start()
        threading.Thread(target=lambda: self.errors.extend(self.process.stderr), daemon=True).start()

    def read_stdout(self) -> None:
        for line in self.process.stdout:
            fields = line.split()
            with self.lock:
                if fields[0] == "NODE":
                    self.node = int(fields[1])
                elif fields[0] == "STREAMING":
                    self.streaming = True
                elif fields[0] == "FRAME":
                    self.frames = int(fields[1])
                elif fields[0] == "DUMPED":
                    self.frames = max(self.frames, int(fields[2]))
                    self.dumped.set()

    def try_link(self) -> bool:
        with self.lock:
            node = self.node
        if self.linked or node is None:
            return self.linked
        dump = json.loads(subprocess.run(["pw-dump"], capture_output=True, text=True, check=True).stdout)
        ports = [(obj["id"], obj["info"]["props"]) for obj in dump if obj["type"] == "PipeWire:Interface:Port"]
        out = [i for i, p in ports if p.get("node.id") == self.producer and p.get("port.direction") == "out"]
        inn = [i for i, p in ports if p.get("node.id") == node and p.get("port.direction") == "in"]
        if out and inn:
            self.linked = (
                subprocess.run(["pw-link", str(out[0]), str(inn[0])], capture_output=True, text=True).returncode == 0
            )
        return self.linked

    def count(self) -> int:
        with self.lock:
            return self.frames

    def image(self) -> Image.Image | None:
        if not self.count():
            return None
        self.serial += 1
        path = self.directory / f"frame-{self.producer}-{self.serial}.raw"
        self.dumped.clear()
        self.process.stdin.write(f"dump {path}\n")
        self.process.stdin.flush()
        if not self.dumped.wait(5):
            return None
        data = path.read_bytes()
        path.unlink()
        return Image.frombuffer("RGB", self.size, data, "raw", "BGRX", 0, 1)

    def stop(self) -> None:
        if self.process.poll() is None:
            try:
                self.process.stdin.write("quit\n")
                self.process.stdin.flush()
            except (BrokenPipeError, ValueError):
                pass
            try:
                self.process.wait(5)
            except subprocess.TimeoutExpired:
                self.process.kill()


def pixel_near(image: Image.Image, position: tuple[int, int], rgb: tuple[int, int, int]) -> bool:
    got = image.getpixel(position)[:3]
    return all(abs(a - b) <= 2 for a, b in zip(got, rgb))


def rgb(colour: int) -> tuple[int, int, int]:
    return ((colour >> 16) & 255, (colour >> 8) & 255, colour & 255)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, help="directory that receives logs, JSON and PNG frames")
    parser.add_argument("--gnoblin", default=os.environ.get("GNOBLIN_BIN", "/usr/local/lib/gnoblin/bin/gnoblin"))
    parser.add_argument("--gnoblinctl", default=None)
    args = parser.parse_args()
    gnoblin = args.gnoblin
    gnoblinctl = args.gnoblinctl or str(Path(gnoblin).with_name("gnoblinctl"))
    tmp = Path(tempfile.mkdtemp(prefix="gnoblin-hosted-"))
    evidence = args.evidence or tmp / "evidence"
    evidence.mkdir(parents=True, exist_ok=True)
    run = tmp / "run"
    run.mkdir(mode=0o700)
    (tmp / "xdg-config-dirs").mkdir()
    env = {
        "PATH": os.environ["PATH"],
        "HOME": str(tmp / "home"),
        "XDG_RUNTIME_DIR": str(run),
        "XDG_CONFIG_HOME": str(tmp / "home/.config"),
        "XDG_CONFIG_DIRS": str(tmp / "xdg-config-dirs"),
        "XDG_DATA_HOME": str(tmp / "home/.local/share"),
        "XDG_CACHE_HOME": str(tmp / "home/.cache"),
        "XDG_STATE_HOME": str(tmp / "home/.local/state"),
        "XDG_SESSION_TYPE": "wayland",
        "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
        "LANG": "C.UTF-8",
    }
    Path(env["HOME"]).mkdir()
    processes: list[subprocess.Popen] = []
    consumers: list[Consumer] = []
    logs = {}

    def spawn(name: str, command: list[str], extra: dict | None = None) -> subprocess.Popen:
        log = open(evidence / f"{name}.log", "w")
        logs[name] = log
        process = subprocess.Popen(command, env={**env, **(extra or {})}, stdout=log, stderr=subprocess.STDOUT)
        processes.append(process)
        return process

    # --- command line -------------------------------------------------------------------------
    def cli(*argv: str) -> subprocess.CompletedProcess:
        return subprocess.run([gnoblin, *argv], env=env, capture_output=True, text=True, timeout=30)

    result = cli("--hosted")
    check(
        "--hosted without a display name is refused",
        result.returncode != 0 and "--hosted needs a valid Wayland display name" in result.stderr,
        result.stderr,
    )
    result = cli("--wayland-display", "x")
    check(
        "a display name without a nested mode is refused",
        result.returncode != 0 and "needs a valid Wayland display name" in result.stderr,
        result.stderr,
    )
    result = cli("--hosted", "--devkit", "--wayland-display", "x")
    check(
        "--hosted and --devkit cannot be combined",
        result.returncode != 0 and "cannot be combined" in result.stderr,
        result.stderr,
    )
    result = cli("--hosted", "--greeter", "--wayland-display", "x")
    check(
        "--hosted and --greeter cannot be combined",
        result.returncode != 0 and "--greeter cannot be combined" in result.stderr,
        result.stderr,
    )
    result = cli("--hosted", "--wayland-display", "../escape")
    check(
        "a display name that is a path is refused",
        result.returncode != 0 and "valid Wayland display name" in result.stderr,
        result.stderr,
    )
    check("the usage text lists --hosted", "--hosted" in cli("--bogus").stderr)

    exit_code = 1
    try:
        # --- private services ---------------------------------------------------------------
        bus_path = run / "bus"
        spawn("dbus-daemon", ["dbus-daemon", "--session", "--nofork", f"--address=unix:path={bus_path}"])
        deadline = time.monotonic() + 10
        while not bus_path.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={bus_path}"
        os.environ.update(env)  # this process is a bus client and a PipeWire consumer too
        spawn("pipewire", ["pipewire"])
        deadline = time.monotonic() + 10
        while not (run / "pipewire-0").exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)

        def call(dest, path, interface, method, parameters=None, reply=None, timeout=30000):
            return connection.call_sync(
                dest,
                path,
                interface,
                method,
                parameters,
                GLib.VariantType(reply) if reply else None,
                Gio.DBusCallFlags.NONE,
                timeout,
                None,
            )

        def name_owned(name: str) -> bool:
            return call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                GLib.Variant("(s)", (name,)),
                "(b)",
            ).unpack()[0]

        ready_changes: list[bool] = []

        def properties_changed(_c, _s, _p, _i, _sig, params, _d):
            interface, changed, _invalid = params.unpack()
            if interface == "org.gnoblin.Hosted1" and "Ready" in changed:
                ready_changes.append(changed["Ready"])

        connection.signal_subscribe(
            "org.gnoblin.Hosted1",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            "/org/gnoblin/Hosted1",
            None,
            Gio.DBusSignalFlags.NONE,
            properties_changed,
            None,
        )
        actions: list[str] = []
        connection.signal_subscribe(
            None,
            "org.gnoblin.Hosted1",
            "ActionRequested",
            "/org/gnoblin/Hosted1",
            None,
            Gio.DBusSignalFlags.NONE,
            lambda _c, _s, _p, _i, _sig, params, _d: actions.append(params.unpack()[0]),
            None,
        )

        # --- start the hosted session -------------------------------------------------------
        started = time.monotonic()
        guardian = spawn("gnoblin", [gnoblin, "--hosted", "--wayland-display", DISPLAY])
        owned = pump(until=lambda: name_owned("org.gnoblin.Hosted1"), timeout=60)
        check("org.gnoblin.Hosted1 is owned on the session bus", bool(owned))

        def ready() -> bool:
            return call(
                "org.gnoblin.Hosted1",
                "/org/gnoblin/Hosted1",
                "org.freedesktop.DBus.Properties",
                "Get",
                GLib.Variant("(ss)", ("org.gnoblin.Hosted1", "Ready")),
                "(v)",
            ).unpack()[0]

        first_ready = ready()
        became_ready = first_ready or pump(until=ready, timeout=60)
        check("Ready becomes true after the runtime handshake", bool(became_ready))
        note(
            f"Hosted1 name owned and Ready true {time.monotonic() - started:.1f}s after start; Ready was {first_ready} at first read; PropertiesChanged(Ready) seen: {ready_changes}"
        )
        if not first_ready:
            check(
                "a false to true Ready change was announced with PropertiesChanged",
                True in ready_changes,
                str(ready_changes),
            )
        display = call(
            "org.gnoblin.Hosted1",
            "/org/gnoblin/Hosted1",
            "org.freedesktop.DBus.Properties",
            "Get",
            GLib.Variant("(ss)", ("org.gnoblin.Hosted1", "WaylandDisplay")),
            "(v)",
        ).unpack()[0]
        check(
            "WaylandDisplay names the session-local socket", display == DISPLAY and (run / DISPLAY).is_socket(), display
        )
        check(
            "the control socket exists in the session runtime directory",
            (run / "gnoblin/compositor-v1.sock").is_socket(),
        )
        check("no viewer owns org.gnome.Mutter.Devkit", not name_owned("org.gnome.Mutter.Devkit"))
        for name in ("org.gnome.Mutter.ScreenCast", "org.gnome.Mutter.RemoteDesktop", "org.gnome.Mutter.DisplayConfig"):
            check(f"{name} is available", pump(until=lambda n=name: name_owned(n), timeout=20))

        def descendants(pid: int) -> list[tuple[int, str]]:
            found, parents = [], {}
            for entry in Path("/proc").iterdir():
                if entry.name.isdigit():
                    try:
                        stat_fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
                        parents[int(entry.name)] = int(stat_fields[1])
                    except (OSError, IndexError):
                        continue
            todo = [pid]
            while todo:
                current = todo.pop()
                for child, parent in parents.items():
                    if parent == current:
                        try:
                            command = (
                                (Path("/proc") / str(child) / "cmdline").read_bytes().replace(b"\0", b" ").decode()
                            )
                        except OSError:
                            command = ""
                        found.append((child, command))
                        todo.append(child)
            return found

        tree = descendants(guardian.pid)
        (evidence / "process-tree.txt").write_text("\n".join(f"{pid} {cmd}" for pid, cmd in tree) + "\n")
        check(
            "the compositor runs headless with no devkit viewer",
            any("--headless" in cmd for _p, cmd in tree) and not any("devkit" in cmd for _p, cmd in tree),
            str(tree),
        )

        control = Control(str(run / "gnoblin/compositor-v1.sock"))
        (evidence / "control-greeting.json").write_text(json.dumps(control.greeting, indent=2))
        layers = control.request("layers.list")
        check(
            "layers.list answers with no surfaces on a session without monitors",
            layers.get("result") == [],
            json.dumps(layers),
        )

        # --- lock requests: nobody listens yet ----------------------------------------------
        lock = subprocess.run([gnoblinctl, "session", "lock"], env=env, capture_output=True, text=True, timeout=15)
        check(
            "a lock request fails while no manager subscribes to lock-requested",
            lock.returncode != 0,
            lock.stdout + lock.stderr,
        )
        note(f"lock without subscriber: rc={lock.returncode} {lock.stderr.strip() or lock.stdout.strip()}")

        # --- virtual monitors through ScreenCast --------------------------------------------
        capture_tool = tmp / "hosted-capture"
        subprocess.run(
            [
                "cc",
                "-Wall",
                "-Wextra",
                "-Werror",
                *shlex.split(subprocess.check_output(["pkg-config", "--cflags", "libpipewire-0.3"], text=True)),
                str(ROOT / "tests/hosted-capture.c"),
                *shlex.split(subprocess.check_output(["pkg-config", "--libs", "libpipewire-0.3"], text=True)),
                "-o",
                str(capture_tool),
            ],
            check=True,
        )
        sc = "org.gnome.Mutter.ScreenCast"
        node_ids: dict[str, int] = {}

        def node_added(_c, _s, path, _i, _sig, params, _d):
            node_ids[path] = params.unpack()[0]

        def add_monitor(width: int, height: int):
            session = call(
                sc, "/org/gnome/Mutter/ScreenCast", sc, "CreateSession", GLib.Variant("(a{sv})", ({},)), "(o)"
            ).unpack()[0]
            stream = call(
                sc,
                session,
                sc + ".Session",
                "RecordVirtual",
                GLib.Variant("(a{sv})", ({"cursor-mode": GLib.Variant("u", 1)},)),
                "(o)",
            ).unpack()[0]
            connection.signal_subscribe(
                sc, sc + ".Stream", "PipeWireStreamAdded", stream, None, Gio.DBusSignalFlags.NONE, node_added, None
            )
            call(sc, session, sc + ".Session", "Start")
            node = pump(until=lambda: node_ids.get(stream), timeout=20)
            consumer = Consumer(capture_tool, node, width, height, {**env}, tmp) if node else None
            if consumer:
                consumers.append(consumer)
                pump(until=consumer.try_link, timeout=20)
            return session, stream, node, consumer

        session_a, stream_a, node_a, consumer_a = add_monitor(1280, 720)
        check("RecordVirtual yields a PipeWire node for monitor A", bool(node_a))
        session_b, stream_b, node_b, consumer_b = add_monitor(800, 600)
        check("RecordVirtual yields a PipeWire node for monitor B", bool(node_b))
        check(
            "frames reach the consumer of monitor A",
            bool(consumer_a and pump(until=lambda: consumer_a.count() > 0, timeout=30)),
            str(consumer_a.errors if consumer_a else None),
        )
        check(
            "frames reach the consumer of monitor B",
            bool(consumer_b and pump(until=lambda: consumer_b.count() > 0, timeout=30)),
        )
        if consumer_a is None or consumer_b is None:
            return 1

        dc = "org.gnome.Mutter.DisplayConfig"
        DC_STATE = "(ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv})"

        def display_state():
            serial, monitors, logical, _props = call(
                dc, "/org/gnome/Mutter/DisplayConfig", dc, "GetCurrentState", None, DC_STATE
            ).unpack()
            return serial, monitors, logical

        def two_monitors():
            state = display_state()
            return state if len(state[1]) == 2 else None

        state = pump(until=two_monitors, timeout=20)
        check("DisplayConfig lists both virtual monitors", bool(state))
        serial, monitors, logical = state
        (evidence / "display-state-initial.json").write_text(
            json.dumps({"monitors": monitors, "logical": logical}, indent=1, default=str)
        )
        connectors = sorted(m[0][0] for m in monitors)
        note(f"virtual monitor connectors: {connectors}")
        current = {}
        for monitor in monitors:
            for mode in monitor[1]:
                if mode[6].get("is-current"):
                    current[monitor[0][0]] = mode
        sizes = {c: (m[1], m[2]) for c, m in current.items()}
        check(
            "the monitors take the size the consumers negotiated",
            sorted(sizes.values()) == [(800, 600), (1280, 720)],
            str(sizes),
        )
        conn_a = next(c for c, s in sizes.items() if s == (1280, 720))
        conn_b = next(c for c, s in sizes.items() if s == (800, 600))

        def apply_layout(scale_a: float, x_b: int):
            serial, _m, _l = display_state()
            logical_width_a = int(1280 / scale_a)
            layout = [
                (0, 0, scale_a, 0, True, [(conn_a, current[conn_a][0], {})]),
                (logical_width_a if x_b is None else x_b, 0, 1.0, 0, False, [(conn_b, current[conn_b][0], {})]),
            ]
            call(
                dc,
                "/org/gnome/Mutter/DisplayConfig",
                dc,
                "ApplyMonitorsConfig",
                GLib.Variant("(uua(iiduba(ssa{sv}))a{sv})", (serial, 1, layout, {})),
            )

        # Monitor A gets the smallest logical scale above 1 that DisplayConfig allows, so the checks
        # below also prove that layer-shell sizes and the streamed pixels follow the scale.
        scale = min((x for x in current[conn_a][5] if x > 1.0), default=1.0)
        note(f"supported scales for monitor A: {current[conn_a][5]}; using {scale}")
        logical_width, logical_height = round(1280 / scale), round(720 / scale)
        bar_px, dock_px = round(32 * scale), round(48 * scale)
        try:
            apply_layout(scale, None)
            applied = True
        except GLib.Error as error:
            applied = False
            note(f"ApplyMonitorsConfig failed: {error.message}")
        check("ApplyMonitorsConfig sets monitor A's scale and positions monitor B", applied)
        _s, _m, logical = display_state()
        positions = {lm[5][0][0]: (lm[0], lm[1], lm[2]) for lm in logical}
        check(
            "monitor B sits at the scaled logical edge of monitor A",
            positions.get(conn_a) == (0, 0, scale) and positions.get(conn_b) == (logical_width, 0, 1.0),
            str(positions),
        )
        (evidence / "display-state-applied.json").write_text(json.dumps(display_state()[1:], indent=1, default=str))

        # --- layer-shell and toplevel clients -----------------------------------------------
        subscriber = Control(str(run / "gnoblin/compositor-v1.sock"))
        subscriber.subscribe(
            [
                "gnoblin.layer.created",
                "gnoblin.layer.changed",
                "gnoblin.layer.removed",
                "gnoblin.session.lock-requested",
            ]
        )
        build = tmp / "client-build"
        build.mkdir()
        layer_xml = ROOT / "src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
        xdg_xml = (
            Path(
                subprocess.check_output(["pkg-config", "--variable=pkgdatadir", "wayland-protocols"], text=True).strip()
            )
            / "stable/xdg-shell/xdg-shell.xml"
        )
        for mode, xml, name in (
            ("client-header", layer_xml, "wlr-layer-shell-unstable-v1-client-protocol.h"),
            ("private-code", layer_xml, "layer.c"),
            ("private-code", xdg_xml, "xdg.c"),
        ):
            subprocess.run(["wayland-scanner", mode, str(xml), str(build / name)], check=True)
        subprocess.run(
            [
                "cc",
                "-Wall",
                "-Wextra",
                "-Werror",
                f"-I{build}",
                str(ROOT / "tests/hosted-layer-client.c"),
                str(build / "layer.c"),
                str(build / "xdg.c"),
                *shlex.split(
                    subprocess.check_output(["pkg-config", "--cflags", "--libs", "wayland-client"], text=True)
                ),
                "-o",
                str(build / "client"),
            ],
            check=True,
        )
        client_env = {"WAYLAND_DISPLAY": DISPLAY}
        palette = {
            "wall_a": 0xFF204060,
            "bar_a": 0xFFD04020,
            "dock_a": 0xFF20C040,
            "wall_b": 0xFF602060,
            "bar_b": 0xFFE0E020,
        }
        specs = {
            "wall_a": ("bingux-wallpaper", "background", "all", 0, conn_a),
            "bar_a": ("bingux-top-bar", "top", "top", 32, conn_a),
            "dock_a": ("bingux-dock", "top", "bottom", 48, conn_a),
            "wall_b": ("bingux-wallpaper", "background", "all", 0, conn_b),
            "bar_b": ("bingux-top-bar", "top", "top", 32, conn_b),
        }
        clients = {}
        for key, (namespace, layer, anchor, size, output) in specs.items():
            clients[key] = spawn(
                f"layer-{key}",
                [str(build / "client"), namespace, layer, anchor, str(size), hex(palette[key]), output],
                client_env,
            )
        for key, process in clients.items():
            line = pump(
                until=lambda p=process, k=key: (
                    (evidence / f"layer-{k}.log").read_text().strip() or (p.poll() is not None and "EXITED")
                ),
                timeout=20,
            )
            check(f"layer client {key} maps", bool(line) and str(line).startswith("MAPPED"), str(line))

        def layer_records():
            return control.request("layers.list").get("result") or []

        def all_mapped():
            current = layer_records()
            return current if len([x for x in current if x.get("mapped")]) == 5 else None

        records = pump(until=all_mapped, timeout=20) or layer_records()
        (evidence / "layer-list.json").write_text(json.dumps(records, indent=2))
        by_ns_output = {(r.get("namespace"), r.get("monitor_id")): r for r in records}
        note("layers.list monitor_id values: " + ", ".join(sorted({str(r.get("monitor_id")) for r in records})))
        for key, (namespace, _l, _a, _s, output) in specs.items():
            record = by_ns_output.get((namespace, output))
            check(
                f"layers.list reports {namespace} on {output} as mapped",
                bool(record and record.get("mapped")),
                json.dumps(record),
            )
        subscriber.drain(0.5)
        created = [e for e in subscriber.events if e.get("event") == "gnoblin.layer.created"]
        check("layer.created events arrived for every surface", len(created) >= 5, f"{len(created)} events")
        (evidence / "layer-events.json").write_text(json.dumps(subscriber.events, indent=2))

        # --- the pixels ---------------------------------------------------------------------
        def frame_matches(consumer: Consumer, probes: list[tuple[tuple[int, int], int]]):
            def test():
                image = consumer.image()
                return image if image and all(pixel_near(image, pos, rgb(colour)) for pos, colour in probes) else None

            return test

        image_a = pump(
            until=frame_matches(
                consumer_a,
                [
                    ((640, 360), palette["wall_a"]),
                    ((640, bar_px - 4), palette["bar_a"]),
                    ((640, bar_px + 4), palette["wall_a"]),
                    ((640, 720 - dock_px + 4), palette["dock_a"]),
                    ((640, 720 - dock_px - 4), palette["wall_a"]),
                ],
            ),
            timeout=30,
        )
        check(
            f"monitor A frames show wallpaper, a {bar_px}px top bar and a {dock_px}px dock at scale {scale}",
            bool(image_a),
        )
        image_b = pump(
            until=frame_matches(consumer_b, [((400, 300), palette["wall_b"]), ((400, 10), palette["bar_b"])]),
            timeout=30,
        )
        check("monitor B frames show its own wallpaper and bar", bool(image_b))
        for name, consumer in (("a", consumer_a), ("b", consumer_b)):
            image = consumer.image()
            if image:
                image.convert("RGB").save(evidence / f"monitor-{name}-layers.png")

        # A toplevel from foot.
        foot = spawn(
            "foot",
            ["foot", "--title=hosted-test", "-e", "sh", "-c", "echo hosted session; sleep 600"],
            {"WAYLAND_DISPLAY": DISPLAY, "XDG_CACHE_HOME": env["XDG_CACHE_HOME"]},
        )
        windows = pump(
            until=lambda: [w for w in [control.request("window.list").get("result")] if "hosted-test" in json.dumps(w)],
            timeout=30,
        )
        check("a foot toplevel is listed by window.list", bool(windows))

        def centre_changed():
            for consumer, base in ((consumer_a, palette["wall_a"]), (consumer_b, palette["wall_b"])):
                image = consumer.image()
                if image and not all(
                    pixel_near(image, (x, y), rgb(base)) for x in range(300, 500, 40) for y in range(250, 450, 40)
                ):
                    return consumer, image
            return None

        changed = pump(until=centre_changed, timeout=30)
        check("the foot window shows up in the streamed frames", bool(changed))
        if changed:
            changed[1].convert("RGB").save(evidence / "monitor-with-foot.png")

        foot.send_signal(signal.SIGTERM)

        wall_a = next(
            (r for r in records if r.get("namespace") == "bingux-wallpaper" and r.get("monitor_id") == conn_a), {}
        )
        check(
            f"the wallpaper on monitor A is laid out at {logical_width}x{logical_height} logical pixels",
            (wall_a.get("geometry") or {}).get("width") == logical_width
            and (wall_a.get("geometry") or {}).get("height") == logical_height,
            json.dumps(wall_a.get("geometry")),
        )

        # --- RemoteDesktop and EIS ----------------------------------------------------------
        rd = "org.gnome.Mutter.RemoteDesktop"
        rd_session = call(rd, "/org/gnome/Mutter/RemoteDesktop", rd, "CreateSession", None, "(o)").unpack()[0]
        call(rd, rd_session, rd + ".Session", "Start")
        reply, fds = connection.call_with_unix_fd_list_sync(
            rd,
            rd_session,
            rd + ".Session",
            "ConnectToEIS",
            GLib.Variant("(a{sv})", ({},)),
            GLib.VariantType("(h)"),
            Gio.DBusCallFlags.NONE,
            10000,
            None,
            None,
        )
        eis_fd = fds.get(reply.unpack()[0])
        check("RemoteDesktop hands out an EIS socket to a same-user client", stat.S_ISSOCK(os.fstat(eis_fd).st_mode))
        os.close(eis_fd)
        call(rd, rd_session, rd + ".Session", "Stop")

        # --- lock, logout and power requests ------------------------------------------------
        lock = subprocess.run([gnoblinctl, "session", "lock"], env=env, capture_output=True, text=True, timeout=15)
        check(
            "a lock request succeeds once the manager subscribes to lock-requested",
            lock.returncode == 0,
            lock.stdout + lock.stderr,
        )
        subscriber.drain(0.5)
        check(
            "the subscriber receives gnoblin.session.lock-requested",
            any(e.get("event") == "gnoblin.session.lock-requested" for e in subscriber.events),
        )
        _s, _m, _l = display_state()
        for action in ("reboot", "poweroff", "suspend", "hibernate", "logout"):
            result = subprocess.run(
                [gnoblinctl, "session", action], env=env, capture_output=True, text=True, timeout=15
            )
            note(f"gnoblinctl session {action}: rc={result.returncode} {result.stdout.strip()} {result.stderr.strip()}")
            check(
                f"gnoblinctl session {action} is reported as ActionRequested({action})",
                bool(pump(until=lambda a=action: a in actions, timeout=10)),
                str(actions),
            )
        check("the hosted session is still running after those requests", guardian.poll() is None and bool(ready()))

        # --- a surface goes away ------------------------------------------------------------
        clients["dock_a"].terminate()

        def dock_removed():
            subscriber.drain(0.2)
            return any(e.get("event") == "gnoblin.layer.removed" for e in subscriber.events)

        check(
            "gnoblin.layer.removed arrives when a shell surface goes away", bool(pump(until=dock_removed, timeout=15))
        )
        check(
            "layers.list no longer lists the removed dock",
            not any(r.get("namespace") == "bingux-dock" for r in layer_records()),
        )

        # --- an output goes away ------------------------------------------------------------
        call(sc, session_b, sc + ".Session", "Stop")
        consumers[1].stop()
        pump(2.0)
        after = layer_records()
        (evidence / "layer-list-after-output-removal.json").write_text(json.dumps(after, indent=2))
        note(
            "after removing monitor B: "
            + json.dumps([(r.get("namespace"), r.get("monitor_id"), r.get("mapped")) for r in after])
        )
        subscriber.drain(0.5)
        (evidence / "layer-events-final.json").write_text(json.dumps(subscriber.events, indent=2))

        # --- stop ---------------------------------------------------------------------------
        guardian.send_signal(signal.SIGTERM)
        try:
            code = guardian.wait(timeout=30)
        except subprocess.TimeoutExpired:
            code = None
        check("SIGTERM stops the hosted session", code is not None, str(code))
        note(f"gnoblin exit status {code}")
        check("the Hosted1 name disappears with the session", not name_owned("org.gnoblin.Hosted1"))
        exit_code = 1 if failures else 0
    finally:
        for consumer in consumers:
            consumer.stop()
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
        for process in processes:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
        for log in logs.values():
            log.close()
        if args.evidence is None:
            shutil.rmtree(tmp, ignore_errors=True)
    print(f"\n{len(failures)} failed" if failures else "\nall checks passed")
    for failure in failures:
        print(f"  FAILED: {failure}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
