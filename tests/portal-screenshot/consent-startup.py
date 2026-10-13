#!/usr/bin/python3
"""Check #143 with real portal processes on a private bus and Broadway display.

Requires a built full Gnoblin backend. The original frontend 1.20.4 is expected
to fail consent parity; the adjacent frontend patch is a separately built
candidate, not applied or installed by this test. Policy replies are controlled;
no compositor, image provider, host service or native seat is operated.
"""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
from gi.repository import Gio, GLib

HERE = Path(__file__).resolve().parent
BACKEND = None
FRONTEND = None
VERSION = 2
NAME = "org.freedesktop.impl.portal.desktop.gnoblin"
FRONT = "org.freedesktop.portal.Desktop"
PATH = "/org/freedesktop/portal/desktop"
STORE = "org.freedesktop.impl.portal.PermissionStore"
APP = "dev.gnoblin.ColdConsentProbe"
FLAGS = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION


def connect():
    return Gio.DBusConnection.new_for_address_sync(os.environ["DBUS_SESSION_BUS_ADDRESS"], FLAGS, None, None)


def call(bus, name, path, iface, method, params=None, timeout=3000):
    return bus.call_sync(name, path, iface, method, params, None, Gio.DBusCallFlags.NONE, timeout, None)


def wait(fn, timeout=5):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if fn():
            return
        time.sleep(0.02)
    raise AssertionError("wait timed out")


def client(work):
    assert Path("/.flatpak-info").exists()
    wait(lambda: (work / "ready").exists())
    bus = connect()
    loop = GLib.MainLoop()
    responses = []

    def response(*args):
        responses.append(args[5].unpack())
        (work / "response.json").write_text(json.dumps(responses))
        loop.quit()

    token = "probe"
    handle = PATH + "/request/" + bus.get_unique_name()[1:].replace(".", "_") + "/" + token
    bus.signal_subscribe(
        FRONT, "org.freedesktop.portal.Request", "Response", handle, None, Gio.DBusSignalFlags.NONE, response, None
    )
    result = call(
        bus,
        FRONT,
        PATH,
        "org.freedesktop.portal.Screenshot",
        "Screenshot",
        GLib.Variant(
            "(sa{sv})", ("", {"interactive": GLib.Variant("b", False), "handle_token": GLib.Variant("s", token)})
        ),
    )
    (work / "handle").write_text(result.unpack()[0])
    GLib.timeout_add_seconds(35, lambda: loop.quit() or False)
    loop.run()
    assert len(responses) == 1, responses
    assert responses[0][0] != 0 and not responses[0][1], responses
    # Preserve the client until the controller has inspected permission state.
    wait(lambda: (work / "done").exists())


def inside(work):
    bus = connect()
    loop = GLib.MainLoop()
    thread = threading.Thread(target=loop.run, daemon=True)
    thread.start()
    procs = []
    sandbox = None
    events = []
    records = []
    failures = []
    monitor = connect()
    trace = (work / "bus.jsonl").open("w", buffering=1)

    def filt(conn, msg, incoming, data):
        if incoming:
            body = msg.get_body()
            event = {
                "time": time.monotonic(),
                "type": int(msg.get_message_type()),
                "sender": msg.get_sender(),
                "destination": msg.get_destination(),
                "interface": msg.get_interface(),
                "member": msg.get_member(),
                "path": msg.get_path(),
                "serial": msg.get_serial(),
                "reply_serial": msg.get_reply_serial(),
                "body": body.unpack() if body else None,
            }
            events.append(event)
            trace.write(json.dumps(event) + "\n")
        return None if incoming and msg.get_message_type() == Gio.DBusMessageType.METHOD_CALL else msg

    monitor.add_filter(filt, None)
    call(
        monitor,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus.Monitoring",
        "BecomeMonitor",
        GLib.Variant("(asu)", ([], 0)),
    )
    level = {"value": "inherit"}
    xml = "<node><interface name='org.gnoblin.Compositor'><method name='CheckPermission'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='s' direction='out'/><arg type='s' direction='out'/><arg type='as' direction='out'/><arg type='u' direction='out'/><arg type='b' direction='out'/></method></interface></node>"
    info = Gio.DBusNodeInfo.new_for_xml(xml)

    def policy(conn, sender, path, iface, method, params, inv):
        if level["value"] == "unavailable":
            inv.return_dbus_error("org.freedesktop.DBus.Error.Failed", "Private policy service unavailable")
        else:
            inv.return_value(GLib.Variant("(ssasub)", (level["value"], "private-test-policy", [], 0, False)))

    bus.register_object("/org/gnoblin/Compositor", info.interfaces[0], policy, None, None)
    call(
        bus,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        GLib.Variant("(su)", ("org.gnoblin.Compositor", 4)),
    )

    def owner(name):
        return call(
            bus,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            GLib.Variant("(s)", (name,)),
        ).unpack()[0]

    def stats():
        if not owner(NAME):
            return (0, [])
        try:
            return call(bus, NAME, "/org/gnoblin/ConsentProbe", "org.gnoblin.ConsentProbe", "Stats").unpack()
        except GLib.Error:
            return (0, [])

    def permissions():
        try:
            return call(
                bus,
                STORE,
                "/org/freedesktop/impl/portal/PermissionStore",
                STORE,
                "Lookup",
                GLib.Variant("(ss)", ("screenshot", "screenshot")),
            ).unpack()[0]
        except GLib.Error as e:
            assert "NotFound" in str(e), e
            return {}

    def clear():
        try:
            call(
                bus,
                STORE,
                "/org/freedesktop/impl/portal/PermissionStore",
                STORE,
                "Delete",
                GLib.Variant("(ss)", ("screenshot", "screenshot")),
            )
        except GLib.Error as e:
            assert "NotFound" in str(e), e
        assert permissions() == {}

    def run_client(case):
        nonlocal sandbox
        case.mkdir()
        instance = case.name
        inst = work / "runtime/.flatpak" / instance
        inst.mkdir(parents=True)
        (case / "flatpak-info").write_text(
            "[Application]\nname="
            + APP
            + "\nruntime=org.gnoblin.Probe/x86_64/test\n[Instance]\ninstance-id="
            + instance
            + "\napp-path=/app\nruntime-path=/usr\n[Context]\nshared=ipc;\nsockets=session-bus;\n"
        )
        with (inst / "bwrapinfo.json").open("w") as out, (case / "client.log").open("w") as log:
            cmd = [
                "bwrap",
                "--die-with-parent",
                "--unshare-pid",
                "--unshare-net",
                "--new-session",
                "--ro-bind",
                "/usr",
                "/usr",
                "--symlink",
                "usr/bin",
                "/bin",
                "--symlink",
                "usr/lib64",
                "/lib64",
                "--symlink",
                "usr/lib",
                "/lib",
                "--ro-bind",
                "/etc",
                "/etc",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--tmpfs",
                "/tmp",
                "--ro-bind",
                str(work / "runtime"),
                str(work / "runtime"),
                "--ro-bind",
                str(case / "flatpak-info"),
                "/.flatpak-info",
                "--ro-bind",
                str(HERE),
                "/app",
                "--bind",
                str(case),
                "/output",
                "--chdir",
                "/app",
                "--info-fd",
                str(out.fileno()),
                "/usr/bin/python3",
                "/app/consent-startup.py",
                "/output",
                "--client",
            ]
            sandbox = subprocess.Popen(cmd, pass_fds=(out.fileno(),), stdout=log, stderr=subprocess.STDOUT)

        def ready():
            assert sandbox.poll() is None, (case / "client.log").read_text()
            try:
                return json.loads((inst / "bwrapinfo.json").read_text()).get("child-pid")
            except ValueError:
                return False

        wait(ready)
        (case / "ready").touch()

    with (work / "processes.log").open("w") as log:
        try:
            for command, name in [
                (["gtk4-broadwayd", "--unixsocket=" + str(work / "runtime/http.sock"), ":37"], None),
                (["/usr/libexec/xdg-permission-store"], STORE),
                ([str(FRONTEND), "--verbose"], FRONT),
            ]:
                procs.append(subprocess.Popen(command, stdout=log, stderr=log))
                wait(lambda: owner(name)) if name else wait(lambda: (work / "runtime/broadway38.socket").exists())
            wait(lambda: stats()[0] > 0)
            scenarios = [(cold, None, "inherit", "close") for cold in [False, True, False, True, False, True, False]]
            for stored, policy_level, action in [
                ("no", "inherit", None),
                ("yes", "inherit", None),
                ("yes", "ask", "close"),
                (None, "deny", None),
                (None, "unavailable", None),
                (None, "allow", None),
                (None, "inherit", "deny"),
                (None, "inherit", "grant"),
            ]:
                scenarios.extend([(False, stored, policy_level, action), (True, stored, policy_level, action)])
            for index, (cold, stored, policy_level, action) in enumerate(scenarios):
                case = work / (str(index) + ("-cold" if cold else "-warm"))
                clear()
                level["value"] = policy_level
                if stored:
                    call(
                        bus,
                        STORE,
                        "/org/freedesktop/impl/portal/PermissionStore",
                        STORE,
                        "SetPermission",
                        GLib.Variant("(sbssas)", ("screenshot", True, "screenshot", APP, [stored])),
                    )
                if cold:
                    # A saved denial can leave the backend stopped after the prior case.
                    if owner(NAME):
                        pid = stats()[0]
                        assert pid
                        os.kill(pid, signal.SIGTERM)
                        wait(lambda: not owner(NAME))
                    time.sleep(3)
                else:
                    call(
                        bus,
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "StartServiceByName",
                        GLib.Variant("(su)", (NAME, 0)),
                    )
                    wait(lambda: stats()[0] > 0)
                    # Let the real frontend observe GetAll after backend activation.
                    time.sleep(0.1)
                before = len(events)
                run_client(case)

                def got_dialog_or_result():
                    assert sandbox.poll() is None, (case / "client.log").read_text()
                    return any(w[2] for w in stats()[1]) or (case / "response.json").exists()

                wait(got_dialog_or_result, 8)
                windows = stats()[1]
                time.sleep(0.2)
                observed = events[before:]
                calls = [
                    e
                    for e in observed
                    if e["type"] == 1
                    and e["member"]
                    in ["Screenshot", "AccessDialog", "CheckPermission", "CreateSession", "GetAll", "Lookup"]
                ]
                access = [e for e in calls if e["member"] == "AccessDialog"]
                shots = [
                    e
                    for e in calls
                    if e["interface"] == "org.freedesktop.impl.portal.Screenshot" and e["member"] == "Screenshot"
                ]
                mapped = [w for w in windows if w[2]]
                record = {
                    "case": case.name,
                    "cold": cold,
                    "stored": stored,
                    "policy": policy_level,
                    "action": action,
                    "windows": windows,
                    "before_response": (case / "response.json").exists(),
                    "calls": calls,
                    "permissions_before_action": permissions(),
                    "failures": [],
                }

                def check(condition, message):
                    if not condition:
                        record["failures"].append(message)
                        failures.append(case.name + ": " + message)

                needs_dialog = policy_level in ("inherit", "ask") and (
                    VERSION < 2 or stored is None or (stored == "yes" and policy_level == "ask")
                )
                if needs_dialog:
                    check(bool(mapped), "required consent dialog was not shown")
                if VERSION >= 2:
                    if stored == "no":
                        check(
                            not access and not shots and not mapped and record["before_response"],
                            "stored denial was not enforced by frontend",
                        )
                    else:
                        check(
                            bool(access) == (stored != "yes"), "generic Access call differs from stored-permission path"
                        )
                        for shot in shots:
                            check(
                                shot["body"][3].get("permission_store_checked") is True,
                                "frontend omitted permission_store_checked",
                            )
                    if mapped and stored is None and policy_level == "inherit":
                        check(
                            mapped[0][0] == "AdwMessageDialog", "generic Access replaced by backend screenshot dialog"
                        )
                else:
                    check(not access, "legacy backend unexpectedly received generic Access")
                    for shot in shots:
                        check(
                            "permission_store_checked" not in shot["body"][3], "legacy backend received permission hint"
                        )
                if policy_level in ("deny", "unavailable"):
                    check(not mapped and record["before_response"], "policy failure did not reject promptly")
                    check(not any(e["member"] == "CreateSession" for e in observed), "policy failure started capture")
                if VERSION >= 2 and stored == "yes" and policy_level == "inherit":
                    check(not mapped and record["before_response"], "stored approval unexpectedly needs consent")
                if mapped:
                    check(not record["before_response"], "request completed before consent")
                    check(not any(e["member"] == "CreateSession" for e in observed), "capture started before consent")
                    response = action or "close"
                    # An old-frontend cold request shows the screenshot dialog instead.
                    # Do not grant that substitute when testing the stored denial case.
                    if stored == "no":
                        response = "close"
                    call(
                        bus,
                        NAME,
                        "/org/gnoblin/ConsentProbe",
                        "org.gnoblin.ConsentProbe",
                        "Respond",
                        GLib.Variant("(s)", (response,)),
                    )
                wait(lambda: (case / "response.json").exists())
                wait(lambda: not any(w[2] for w in stats()[1]))
                time.sleep(0.05)
                record["response"] = json.loads((case / "response.json").read_text())
                record["permissions_after_action"] = permissions()
                expected = {APP: [stored]} if stored else {}
                if VERSION >= 2 and stored is None:
                    if policy_level == "allow" or (policy_level == "inherit" and action == "grant"):
                        expected = {APP: ["yes"]}
                    elif policy_level in ("deny", "unavailable") or (policy_level == "inherit" and action == "deny"):
                        expected = {APP: ["no"]}
                check(permissions() == expected, "unexpected saved permission after decision")
                record["calls_after_action"] = [
                    e
                    for e in events[before:]
                    if e["type"] == 1
                    and e["member"]
                    in ["Screenshot", "AccessDialog", "CheckPermission", "CreateSession", "SetPermission"]
                ]
                final_shots = [
                    e
                    for e in record["calls_after_action"]
                    if e["interface"] == "org.freedesktop.impl.portal.Screenshot" and e["member"] == "Screenshot"
                ]
                expects_shot = (
                    VERSION < 2
                    or stored == "yes"
                    or policy_level == "allow"
                    or (stored is None and policy_level == "inherit" and action == "grant")
                )
                check(len(final_shots) == int(expects_shot), "unexpected backend Screenshot call count")
                for shot in final_shots:
                    hint = shot["body"][3]
                    check(
                        hint.get("permission_store_checked") is True
                        if VERSION >= 2
                        else "permission_store_checked" not in hint,
                        "incorrect permission hint on final Screenshot call",
                    )
                records.append(record)
                print(
                    case.name,
                    "stored",
                    stored,
                    "policy",
                    policy_level,
                    "windows",
                    [w[0] for w in mapped],
                    "failures",
                    record["failures"],
                    flush=True,
                )
                (case / "done").touch()
                sandbox.wait(timeout=3)
                assert sandbox.returncode == 0, (case / "client.log").read_text()
                sandbox = None
                (work / "results.json").write_text(json.dumps(records, indent=2) + "\n")
            (work / "verdict.json").write_text(
                json.dumps({"passed": not failures, "failures": failures}, indent=2) + "\n"
            )
        finally:
            if sandbox and sandbox.poll() is None:
                sandbox.terminate()
                sandbox.wait(timeout=3)
            if owner(NAME):
                pid = stats()[0]
                if pid:
                    os.kill(pid, signal.SIGTERM)
            for p in reversed(procs):
                if p.poll() is None:
                    p.terminate()
                    p.wait(timeout=3)
            loop.quit()
            thread.join()
            monitor.close_sync(None)
            trace.close()
    raise SystemExit(bool(failures))


def main():
    global BACKEND, FRONTEND, VERSION
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("output", type=Path)
    p.add_argument("--backend", type=Path)
    p.add_argument("--frontend", type=Path, default=Path("/usr/libexec/xdg-desktop-portal"))
    p.add_argument("--backend-version", type=int, choices=(1, 2), default=2)
    p.add_argument("--inside", action="store_true")
    p.add_argument("--client", action="store_true")
    a = p.parse_args()
    if not a.client and not a.backend:
        p.error("--backend is required")
    BACKEND = a.backend.resolve() if a.backend else None
    FRONTEND = a.frontend.resolve()
    VERSION = a.backend_version
    work = a.output.resolve()
    if a.client:
        return client(work)
    if a.inside:
        return inside(work)
    work.mkdir(exist_ok=False)
    import shlex

    flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "libadwaita-1"], text=True))
    subprocess.run(
        [
            "cc",
            "-shared",
            "-fPIC",
            "-g",
            "-Wall",
            "-Wextra",
            "-Wno-unused-parameter",
            "-Wno-deprecated-declarations",
            str(HERE / "consent-observer.c"),
            "-o",
            str(work / "observe.so"),
            *flags,
        ],
        check=True,
    )
    for part in ["runtime", "config", "data", "cache", "portals", "services", "tmp"]:
        (work / part).mkdir(mode=0o700)
    (work / "portals/gnoblin.portal").write_text(
        "[portal]\nDBusName="
        + NAME
        + "\nInterfaces=org.freedesktop.impl.portal.Screenshot;org.freedesktop.impl.portal.Access;\nUseIn=gnoblin\n"
    )
    (work / "portals/portals.conf").write_text(
        "[preferred]\ndefault=none\norg.freedesktop.impl.portal.Screenshot=gnoblin\norg.freedesktop.impl.portal.Access=gnoblin\n"
    )
    (work / "services/backend.service").write_text(
        "[D-BUS Service]\nName="
        + NAME
        + "\nExec=/usr/bin/env LD_PRELOAD="
        + str(work / "observe.so")
        + " "
        + str(BACKEND)
        + "\n"
    )
    (work / "bus.conf").write_text(
        "<busconfig><type>session</type><listen>unix:path="
        + str(work / "runtime/bus")
        + "</listen><auth>EXTERNAL</auth><servicedir>"
        + str(work / "services")
        + '</servicedir><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/><allow eavesdrop="true"/></policy></busconfig>'
    )
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(work / "runtime"),
        XDG_CONFIG_HOME=str(work / "config"),
        XDG_DATA_HOME=str(work / "data"),
        XDG_CACHE_HOME=str(work / "cache"),
        XDG_CONFIG_DIRS=str(work / "config"),
        XDG_DATA_DIRS="/usr/share",
        XDG_DESKTOP_PORTAL_DIR=str(work / "portals"),
        XDG_CURRENT_DESKTOP="gnoblin",
        XDG_SESSION_TYPE="x11",
        GDK_BACKEND="broadway",
        BROADWAY_DISPLAY=":37",
        GTK_A11Y="none",
        GIO_USE_VFS="local",
        GSETTINGS_BACKEND="memory",
        TMPDIR=str(work / "tmp"),
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=" + str(work / "runtime/bus"),
        G_MESSAGES_DEBUG="xdg-desktop-portal",
        PYTHONDONTWRITEBYTECODE="1",
        LC_ALL="C.UTF-8",
    )
    with (work / "run.log").open("w") as log:
        r = subprocess.run(
            [
                "dbus-run-session",
                "--config-file=" + str(work / "bus.conf"),
                "--",
                sys.executable,
                __file__,
                str(work),
                "--inside",
                "--backend",
                str(BACKEND),
                "--frontend",
                str(FRONTEND),
                "--backend-version",
                str(VERSION),
            ],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=240,
        )
    print("result", r.returncode, "log", work / "run.log")
    sys.exit(r.returncode)


if __name__ == "__main__":
    main()
