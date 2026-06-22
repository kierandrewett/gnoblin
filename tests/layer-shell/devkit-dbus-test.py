#!/usr/bin/env python3
# Regression for scripts/devkit_dbus.py.
#
# The helper writes a DBus .service file whose Exec= line includes the repo path
# and a per-run document mount path. Keep this working when paths contain spaces:
# DBus activation must start the Documents stub, and GetMountPoint must return
# the exact stub mount path.
import importlib.util, pathlib, shutil, subprocess, sys, tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("devkit_dbus", ROOT / "scripts" / "devkit_dbus.py")
devkit_dbus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(devkit_dbus)

try:
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio
except Exception:
    print("SKIP: needs python gi")
    sys.exit(0)


def main():
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="gnoblin dbus helper."))
    try:
        conf = devkit_dbus.write_config(tmp, ROOT)
        service = (tmp / "dbus-services" / "org.freedesktop.portal.Documents.service").read_text()
        if "gnoblin dbus helper." not in service:
            print("FAIL: test did not exercise a path containing spaces")
            return 1

        code = """
import gi, sys
gi.require_version('Gio', '2.0')
from gi.repository import Gio
p = Gio.DBusProxy.new_for_bus_sync(
    Gio.BusType.SESSION,
    Gio.DBusProxyFlags.NONE,
    None,
    'org.freedesktop.portal.Documents',
    '/org/freedesktop/portal/documents',
    'org.freedesktop.portal.Documents',
    None,
)
mount = bytes(p.call_sync('GetMountPoint', None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0])
sys.stdout.buffer.write(mount)
"""
        r = subprocess.run(
            ["dbus-run-session", f"--config-file={conf}", "--", "python3", "-c", code],
            capture_output=True,
            text=False,
            timeout=10,
        )
        if r.returncode != 0:
            print(r.stderr.decode(errors="replace").rstrip())
            print("FAIL: DBus did not activate the Documents stub")
            return 1
        expected = str(tmp / "doc").encode() + b"\0"
        if r.stdout != expected:
            print(f"FAIL: mount mismatch: got={r.stdout!r} expected={expected!r}")
            return 1
        print("PASS: devkit DBus helper activates Documents stub with spaced paths")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
