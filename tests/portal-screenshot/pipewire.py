#!/usr/bin/env python3
"""Real one-frame PipeWire delivery with a private daemon and a fake Mutter service.

Requires GStreamer videotestsrc/pipewiresink and PipeWire command line tools.
No desktop, session manager, device monitor, or host bus is used.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
from gi.repository import Gio, GLib
from transport import NAME, SESSION, STREAM, XML


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("client", type=Path)
    parser.add_argument("--mode", choices=("success", "cancel", "closed"), default="success")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="gnoblin-pw-") as tmp:
        env = dict(
            os.environ,
            XDG_RUNTIME_DIR=tmp,
            PIPEWIRE_RUNTIME_DIR=tmp,
            TMPDIR=tmp,
            GIO_USE_VFS="local",
            GNOBLIN_TEST_RED="1",
        )
        env.pop("PIPEWIRE_REMOTE", None)
        processes = []
        try:
            with open(Path(tmp) / "pipewire.log", "w+") as log:
                daemon = subprocess.Popen(["pipewire"], env=env, stdout=log, stderr=log)
                processes.append(daemon)
                for _ in range(100):
                    if (Path(tmp) / "pipewire-0").exists():
                        break
                    assert daemon.poll() is None
                    time.sleep(0.02)
                producer = subprocess.Popen(
                    [
                        "gst-launch-1.0",
                        "-q",
                        "videotestsrc",
                        "is-live=true",
                        "pattern=red",
                        "!",
                        "video/x-raw,format=BGRx,width=64,height=32,framerate=30/1",
                        "!",
                        "pipewiresink",
                        "mode=provide",
                        "stream-properties=props,node.name=gnoblin-test-source",
                    ],
                    env=env,
                    stdout=log,
                    stderr=log,
                )
                processes.append(producer)

                def dump():
                    return json.loads(subprocess.check_output(["pw-dump"], env=env, text=True, timeout=3))

                node = None
                for _ in range(100):
                    objects = dump()
                    for obj in objects:
                        if (
                            obj["type"] == "PipeWire:Interface:Node"
                            and obj.get("info", {}).get("props", {}).get("node.name") == "gnoblin-test-source"
                        ):
                            node = obj["id"]
                    if node:
                        break
                    assert producer.poll() is None
                    time.sleep(0.02)
                assert node is not None, "no producer"
                bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
                bus.call_sync(
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "RequestName",
                    GLib.Variant("(su)", (NAME, 4)),
                    None,
                    Gio.DBusCallFlags.NONE,
                    2000,
                    None,
                )
                events = []

                def method(conn, sender, path, iface, name, params, invocation):
                    events.append(name)
                    if name == "CreateSession":
                        invocation.return_value(GLib.Variant("(o)", (SESSION,)))
                    elif name == "RecordArea":
                        invocation.return_value(GLib.Variant("(o)", (STREAM,)))
                    else:
                        invocation.return_value(GLib.Variant("()", ()))
                    if name == "Start":
                        conn.emit_signal(
                            sender, STREAM, NAME + ".Stream", "PipeWireStreamAdded", GLib.Variant("(u)", (node,))
                        )

                xml = Gio.DBusNodeInfo.new_for_xml(XML)
                bus.register_object("/org/gnome/Mutter/ScreenCast", xml.interfaces[0], method, None, None)
                bus.register_object(SESSION, xml.interfaces[1], method, None, None)
                loop = GLib.MainLoop()
                thread = threading.Thread(target=loop.run, daemon=True)
                thread.start()
                client = subprocess.Popen(
                    [str(args.client)] + (["1000"] if args.mode == "cancel" else []),
                    env=env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                processes.append(client)
                try:
                    # A full desktop uses a session manager. Here link only our
                    # two nodes explicitly; no ALSA/BlueZ/host devices are probed.
                    ports = {}
                    for _ in range(100):
                        objects = dump()
                        for obj in objects:
                            if obj["type"] == "PipeWire:Interface:Port":
                                props = obj.get("info", {}).get("props", {})
                                ports[props.get("port.direction")] = obj["id"]
                        if "in" in ports and "out" in ports:
                            break
                        assert client.poll() is None, client.communicate()
                        time.sleep(0.03)
                    assert "in" in ports and "out" in ports, objects
                    if args.mode == "success":
                        subprocess.run(["pw-link", str(ports["out"]), str(ports["in"])], env=env, check=True, timeout=3)
                    elif args.mode == "closed":
                        bus.emit_signal(None, SESSION, NAME + ".Session", "Closed", None)
                    stdout, stderr = client.communicate(timeout=12)
                    expected = 0 if args.mode == "success" else 1
                    if client.returncode != expected:
                        log.seek(0)
                        raise AssertionError((stdout, stderr, log.read()))
                    assert stdout.startswith("PNG 64 32 " if args.mode == "success" else "ERROR "), (stdout, stderr)
                    assert not list(Path(tmp).glob("gnoblin-screenshot-*")), "temporary PNG leaked"
                    assert events == ["CreateSession", "RecordArea", "Start", "Stop"], events
                    print("PASS real PipeWire", args.mode, stdout.strip(), events)
                finally:
                    loop.quit()
                    thread.join()
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=3)


if __name__ == "__main__":
    main()
