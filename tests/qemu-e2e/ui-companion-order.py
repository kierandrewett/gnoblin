#!/usr/bin/env python3
"""VM pixels/input regression: RUNNER RUN --quickshell PATH --report NEW_DIRECTORY.

Requires Pillow, a running disposable VM and guest qs/Python/gnoblinctl. Does
not change installed QML/config. Run on the old build first: companions-above
must fail if native ordering is absent or pixels disagree with pointer picking.
"""

import argparse
import json
from pathlib import Path
import select
import socket
import subprocess
import sys
import time

from PIL import Image, ImageStat

# One SSH process keeps ownership sockets alive; a separate watcher verifies
# published/cleared state rather than treating an accepted reply as delivery.
HELPER = r"""
import json, os, socket, sys
clients, names = {}, {}
def receive(connection):
    return json.loads(connection.readline())
def until(connection, match):
    while True:
        event = receive(connection)
        if match(event): return event
def connect(key):
    sock = socket.socket(socket.AF_UNIX); sock.settimeout(8)
    sock.connect(os.environ['XDG_RUNTIME_DIR'] + '/gnoblin/compositor-v1.sock')
    stream = sock.makefile('rwb', buffering=0)
    hello = receive(stream)
    assert 'ui-sessions' in hello['capabilities'], hello
    clients[key] = (sock, stream); names[key] = []
    stream.write(b'{"op":"ui-session","action":"watch","id":"watch"}\n')
    assert until(stream, lambda e: e.get('id') == 'watch')['result']['accepted']
connect('watcher'); print('{"ready":true}', flush=True)
for line in sys.stdin:
    request = json.loads(line); key = request['owner']
    if request.get('disconnect'):
        stream = clients[key][1]; stream.close(); clients.pop(key)[0].close()
        for name in names.pop(key):
            until(clients['watcher'][1], lambda e: e.get('event') == 'ui-state' and e.get('name') == name and e.get('state', 1) is None)
    else:
        if key not in clients: connect(key)
        name, state = request['name'], request['state']
        payload = dict(op='ui-session', action='state', id='state', name=name, state=state)
        clients[key][1].write(json.dumps(payload).encode() + b'\n')
        assert until(clients[key][1], lambda e: e.get('id') == 'state')['result']['accepted']
        until(clients['watcher'][1], lambda e: e.get('event') == 'ui-state' and e.get('name') == name and e.get('state') == state)
        if name not in names[key]: names[key].append(name)
    print('{"accepted":true}', flush=True)
"""

QML = """pragma ComponentBehavior: Bound
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
Scope {
    id: root
    property bool sharedNamespace: false
    property var windows: [a, b, companionLoader.item]
    component Tile: PanelWindow {
        id: tile
        required property int index
        property int clicks: 0
        screen: Quickshell.screens[0]
        visible: true
        anchors { left: true; top: true }
        margins { left: 120 + index * 100; top: 140 + index * 60 }
        implicitWidth: 420; implicitHeight: 300
        color: ["#ef2525", "#25ef25", "#2525ef"][index]
        exclusionMode: ExclusionMode.Ignore
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.namespace: "PREFIX-" + ["A", "B", root.sharedNamespace ? "B" : "C"][index]
        WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
        MouseArea { id: pointer; anchors.fill: parent; hoverEnabled: true; onClicked: tile.clicks++ }
        property bool hovered: pointer.containsMouse
    }
    Tile { id: a; index: 0 }
    Tile { id: b; index: 1 }
    Loader { id: companionLoader; sourceComponent: Tile { index: 2 } }
    IpcHandler {
        target: "probe"
        function status(): string { return JSON.stringify(root.windows.map(w => w.clicks)); }
        function hover(): string { return JSON.stringify(root.windows.map(w => w.hovered)); }
        function focus(index: int, exclusive: bool): void {
            root.windows[index].WlrLayershell.keyboardFocus = exclusive ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None;
        }
        function layer(top: bool): void { b.WlrLayershell.layer = top ? WlrLayer.Top : WlrLayer.Overlay; }
        function monitor(index: int): void { b.screen = Quickshell.screens[index]; }
        function mapped(visible: bool): void { b.visible = visible; }
        function namespace(shared: bool): void {
            // Namespaces are fixed after windowConnected; recreate only C.
            companionLoader.active = false;
            root.sharedNamespace = shared;
            companionLoader.active = true;
        }
    }
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runner", type=Path)
    parser.add_argument("run", type=Path)
    parser.add_argument("--quickshell", required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--lock-client", help="Guest path to session-lock-smoke; run optional lock-unlock regression")
    args = parser.parse_args()
    args.report.mkdir(parents=True, exist_ok=False)
    prefix = "ui-companion-" + str(time.time_ns())
    env = [
        "env",
        "XDG_RUNTIME_DIR=/run/user/1000",
        "WAYLAND_DISPLAY=wayland-0",
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
    ]
    command = [str(args.runner.resolve()), "guest", str(args.run.resolve()), "--", *env]
    ctl = "/usr/local/lib/gnoblin/bin/gnoblinctl"
    helper = sock = stream = directory = None
    results, cleanup_errors = [], []
    button_held = False
    shared_namespace = False

    def guest(*argv):
        return subprocess.check_output([*command, *argv], timeout=20)

    def query(*argv):
        return json.loads(guest(ctl, *argv))

    def ipc(method, *argv):
        result = guest(args.quickshell, "ipc", "-p", directory, "call", "probe", method, *argv)
        return json.loads(result) if result.strip() else None

    def helper_reply():
        assert select.select([helper.stdout], [], [], 12)[0], "Session helper timed out"
        return json.loads(helper.stdout.readline())

    def publish(owner="one", surface="A", companions=("B",), **fields):
        state = dict(
            visible=True,
            surface=prefix + "-" + surface,
            companions=[prefix + "-" + c for c in companions],
            companionsAbove=True,
        )
        state.update(fields)
        helper.stdin.write(json.dumps(dict(owner=owner, name=prefix + "." + owner, state=state)) + "\n")
        helper.stdin.flush()
        assert helper_reply()["accepted"]

    def qmp(name, **arguments):
        stream.write(json.dumps(dict(execute=name, arguments=arguments)) + "\n")
        stream.flush()
        while True:
            reply = json.loads(stream.readline())
            assert "error" not in reply, reply
            if "return" in reply:
                return reply["return"]

    def layer(tile):
        namespace = "B" if shared_namespace and tile == "C" else tile
        return next(
            (
                w
                for w in query("layer", "list")["layers"]
                if w.get("namespace") == prefix + "-" + namespace
                and w["mapped"]
                and (not shared_namespace or w["geometry"]["x"] == 120 + "ABC".index(tile) * 100)
            ),
            None,
        )

    def wait_for(predicate):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            result = predicate()
            if result:
                return result
            time.sleep(0.1)
        raise AssertionError("Fixture layer transition timed out")

    def move(point):
        x, y = point
        for offset in (1, 0):
            qmp(
                "input-send-event",
                events=[
                    dict(type="abs", data=dict(axis="x", value=int((x + offset) / width * 32767))),
                    dict(type="abs", data=dict(axis="y", value=int(y / height * 32767))),
                ],
            )
        time.sleep(0.1)

    def click(point, target, stationary=False):
        nonlocal button_held
        before = ipc("status")
        if not stationary:
            move(point)
        pressed_at = time.monotonic()
        for down in (True, False):
            button_held = down
            qmp("input-send-event", events=[dict(type="btn", data=dict(down=down, button="left"))])
            time.sleep(0.1)
        deadline = time.monotonic() + 2
        while True:
            after = ipc("status")
            observed_at = time.monotonic()
            if after != before or observed_at >= deadline:
                break
            time.sleep(min(0.025, deadline - observed_at))
        delta = [a - b for a, b in zip(after, before)]
        latency_ms = round((observed_at - pressed_at) * 1000, 3) if after != before else None
        # Host-observed latency includes QMP/SSH round trips; never resend input.
        results.append(
            dict(
                event="click",
                point=point,
                target=target,
                before=before,
                after=after,
                delta=delta,
                press_to_observed_delivery_ms=latency_ms,
                observation_elapsed_ms=round((observed_at - pressed_at) * 1000, 3),
            )
        )
        (args.report / "results.json").write_text(json.dumps(results, indent=2))
        print(f"CLICK {target} at {point}: delta={delta}, press-to-observed-delivery={latency_ms} ms", flush=True)
        assert delta == [int(i == "ABC".index(target)) for i in range(3)], (
            point,
            target,
            before,
            after,
        )

    def check(label, target, stationary=True):
        if not stationary:
            move((400, 320))
        time.sleep(0.35)
        raw = args.report.resolve() / (label + ".ppm")
        qmp("screendump", filename=str(raw))
        with Image.open(raw) as image:
            rgb = image.convert("RGB")
            rgb.save(raw.with_suffix(".png"))
            sample = ImageStat.Stat(rgb.crop((446, 346, 454, 354))).mean
        raw.unlink()
        results.append(dict(case=label, target=target, pixels=sample, layers=query("layer", "list")))
        (args.report / "results.json").write_text(json.dumps(results, indent=2))
        expected = [(239, 37, 37), (37, 239, 37), (37, 37, 239)]["ABC".index(target)]
        assert max(abs(a - b) for a, b in zip(sample, expected)) < 12, (label, target, sample)
        hovered = ipc("hover")
        assert hovered == [i == "ABC".index(target) for i in range(3)], (label, target, hovered)
        click((400, 320), target, stationary=True)
        print("PASS", label, flush=True)

    def raise_tile(tile):
        # LayerSurface records are read-only; typed window commands exclude them.
        # NONE -> EXCLUSIVE is the real protocol path that calls meta_window_raise.
        move((400, 320))  # Do not send motion after the native restack.
        ipc("focus", str("ABC".index(tile)), "true")
        wait_for(lambda: layer(tile) and layer(tile)["keyboard_interactive"] == "exclusive")
        ipc("focus", str("ABC".index(tile)), "false")
        wait_for(lambda: layer(tile) and layer(tile)["keyboard_interactive"] == "none")

    def ordinary(label):
        raise_tile("C")
        check(label + "-raise", "C")
        for tile, point in [("A", (150, 160)), ("B", (250, 470)), ("C", (700, 530))]:
            click(point, tile)
            check(label + "-click-" + tile, tile, stationary=False)

    try:
        monitors = query("monitor", "list")["monitors"]
        assert all(m.get("scale", 1) == 1 for m in monitors), "Use VM monitor scale 1 for physical pixels"
        assert all(m["x"] >= 0 and m["y"] >= 0 for m in monitors), "Use nonnegative VM output positions"
        assert (
            monitors[0]["x"] == monitors[0]["y"] == 0 and monitors[0]["width"] >= 800 and monitors[0]["height"] >= 600
        )
        width = max(m["x"] + m["width"] for m in monitors)
        height = max(m["y"] + m["height"] for m in monitors)
        directory = guest("mktemp", "-d", "/tmp/" + prefix + "-XXXXXX").decode().strip()
        guest(
            "python3",
            "-c",
            "from pathlib import Path; import sys; Path(sys.argv[1]).write_text(sys.argv[2])",
            directory + "/shell.qml",
            QML.replace("PREFIX", prefix),
        )
        guest("systemd-run", "--user", "--collect", "--unit=" + prefix, *env, args.quickshell, "-p", directory)
        deadline = time.monotonic() + 10
        while True:
            try:
                ipc("status")
                break
            except subprocess.CalledProcessError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.2)
        sock = socket.socket(socket.AF_UNIX)
        sock.settimeout(10)
        sock.connect(str(args.run / "qmp.sock"))
        stream = sock.makefile("rw")
        stream.readline()
        qmp("qmp_capabilities")
        with (args.report / "helper.log").open("w") as log:
            helper = subprocess.Popen(
                [*command, "python3", "-u", "-c", HELPER],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=log,
                text=True,
            )
        assert helper_reply()["ready"]
        for index, tile in enumerate("ABC"):
            record = wait_for(lambda: layer(tile))
            assert record["layer"] == "overlay" and record["keyboard_interactive"] == "none", record
            assert record["geometry"] == dict(x=120 + index * 100, y=140 + index * 60, width=420, height=300), record
        ordinary("baseline")
        for shared_namespace in (False, True):
            if shared_namespace:
                ipc("namespace", "true")
                wait_for(lambda: layer("C"))
            label = "shared-namespace-star" if shared_namespace else "distinct-namespace-star"
            publish(companions=("B",) if shared_namespace else ("B", "C"))
            raise_tile("B")
            raise_tile("C")
            check(label + "-latest-companion", "C")
            for attempt in range(3):
                raise_tile("A")
                check(label + f"-base-raise-{attempt + 1}", "C")
        publish(visible=False)
        ipc("namespace", "false")
        shared_namespace = False
        wait_for(lambda: layer("C"))
        publish()
        raise_tile("A")
        check("companions-above", "B")
        click((150, 160), "A")
        check("constrained-editor-click", "B", stationary=False)
        raise_tile("C")
        check("unrelated-control", "C")
        publish(companionsAbove=False)
        raise_tile("B")
        check("companions-below", "A")
        publish(visible=False)
        ordinary("hidden")
        publish(companions=("B", "B"))
        raise_tile("A")
        check("deduplicated", "B")
        publish(companions=("A", "B"))
        ordinary("self-invalid")
        publish(companionsAbove="invalid")
        ordinary("invalid-fields")
        publish(companions=("B", "C"))
        publish(owner="two", surface="C", companions=("B",))
        ordinary("namespace-conflict")
        publish(owner="two", surface="B", companions=("A",))
        ordinary("conflict-and-cycle")  # C must be released too: entire declarations disabled.
        publish(owner="two", visible=False)
        publish()
        raise_tile("A")
        check("conflict-removed", "B")
        ipc("mapped", "false")
        wait_for(lambda: layer("B") is None)
        raise_tile("A")
        check("companion-unmapped", "A")
        ipc("mapped", "true")
        wait_for(lambda: layer("B"))
        check("companion-remapped-stationary", "B")
        ipc("layer", "true")
        wait_for(lambda: layer("B") and layer("B")["layer"] == "top")
        raise_tile("A")
        check("different-layer", "A")
        ipc("layer", "false")
        wait_for(lambda: layer("B") and layer("B")["layer"] == "overlay")
        raise_tile("A")
        check("layer-restored", "B")
        if len(monitors) > 1:
            ipc("monitor", "1")
            wait_for(lambda: layer("B") and layer("B")["monitor_id"] != layer("A")["monitor_id"])
            check("different-monitor-stationary", "A")
            ipc("monitor", "0")
            wait_for(lambda: layer("B") and layer("B")["monitor_id"] == layer("A")["monitor_id"])
            check("monitor-restored-stationary", "B")
        else:
            print("SKIP different-monitor: VM has one monitor", flush=True)
        if args.lock_client:
            publish()
            output = guest(args.lock_client, "lock-unlock").decode()
            (args.report / "lock-client.log").write_text(output)
            print(output, end="" if output.endswith("\n") else "\n", flush=True)
            lines = output.splitlines()
            locked = next((i for i, line in enumerate(lines) if line.startswith("LOCKED:")), None)
            unlocked = next((i for i, line in enumerate(lines) if line.startswith("UNLOCKED:")), None)
            assert locked is not None and unlocked is not None and locked < unlocked, output
            raise_tile("A")
            check("lock-unlock-companion-restored", "B")
        else:
            print("SKIP lock-unlock: --lock-client not supplied", flush=True)
        helper.stdin.write(json.dumps(dict(owner="one", disconnect=True)) + "\n")
        helper.stdin.flush()
        assert helper_reply()["accepted"]
        ordinary("owner-disconnected")
    finally:
        failed = sys.exc_info()[0] is not None

        def cleanup(label, action):
            try:
                action()
            except Exception as error:
                cleanup_errors.append(f"{label}: {error}")

        if helper:
            cleanup("owner EOF", helper.stdin.close)
            cleanup("owner exit", lambda: helper.wait(timeout=5))
            if helper.poll() is None:
                cleanup("owner terminate", helper.terminate)
                cleanup("owner reap", lambda: helper.wait(timeout=5))
            if helper.poll() is None:
                cleanup("owner kill", helper.kill)
                cleanup("owner final reap", lambda: helper.wait(timeout=5))
            cleanup("helper output", helper.stdout.close)
        if button_held and stream:
            cleanup(
                "pointer release",
                lambda: qmp("input-send-event", events=[dict(type="btn", data=dict(down=False, button="left"))]),
            )

        def stop_fixture():
            try:
                guest("systemctl", "--user", "stop", prefix + ".service")
            except Exception:
                guest("systemctl", "--user", "kill", "--signal=KILL", prefix + ".service")
                raise

        if directory:
            cleanup("fixture stop", stop_fixture)
        if directory:
            cleanup(
                "fixture log",
                lambda: (args.report / "fixture.log").write_bytes(
                    guest("journalctl", "--user", "-u", prefix, "--no-pager")
                ),
            )
            cleanup(
                "fixture directory",
                lambda: guest("python3", "-c", "import shutil,sys; shutil.rmtree(sys.argv[1])", directory),
            )
        if stream:
            cleanup("QMP stream", stream.close)
        if sock:
            cleanup("QMP socket", sock.close)
        if cleanup_errors:
            print("\n".join(cleanup_errors), file=sys.stderr)
            if not failed:
                raise RuntimeError("Fixture cleanup failed")


if __name__ == "__main__":
    main()
