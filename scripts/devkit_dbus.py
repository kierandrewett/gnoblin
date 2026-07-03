#!/usr/bin/env python3
"""Write the isolated DBus config used by gnoblin devkit runs.

The visible devkit and the headless harness both need xdg-desktop-portal for
Mutter ScreenCast, but must not inherit the host session's full DBus service
directory. In particular, the real document portal tries to mount FUSE at the
user's real /run/user/.../doc. This helper writes a small per-run service dir
with only the portal services the devkit needs plus gnoblin's document stub.
"""

from __future__ import annotations

import html
import pathlib
import shlex
import shutil
import sys

DBUS_SERVICES = (
    "org.freedesktop.portal.Desktop",
    "org.freedesktop.impl.portal.PermissionStore",
    "org.freedesktop.impl.portal.desktop.gnome",
    "org.freedesktop.impl.portal.desktop.gtk",
    "org.gnome.Settings.GlobalShortcutsProvider",
)


def write_config(tmp: pathlib.Path, repo_root: pathlib.Path) -> pathlib.Path:
    tmp = tmp.resolve()
    repo_root = repo_root.resolve()
    service_dir = tmp / "dbus-services"
    service_dir.mkdir(parents=True, exist_ok=True)
    system_service_dir = pathlib.Path("/usr/share/dbus-1/services")
    for name in DBUS_SERVICES:
        src = system_service_dir / f"{name}.service"
        if not src.exists():
            raise RuntimeError(f"missing required DBus service: {src}")
        shutil.copy2(src, service_dir / src.name)

    doc_mount = tmp / "doc"
    doc_mount.mkdir(parents=True, exist_ok=True)
    doc_stub = repo_root / "scripts" / "devkit-document-portal-stub.py"
    (service_dir / "org.freedesktop.portal.Documents.service").write_text(
        "[D-BUS Service]\n"
        "Name=org.freedesktop.portal.Documents\n"
        f"Exec=/usr/bin/env python3 {shlex.quote(str(doc_stub))} {shlex.quote(str(doc_mount))}\n"
    )

    conf = tmp / "dbus-session.conf"
    service_dir_xml = html.escape(str(service_dir), quote=False)

    # Also expose gnoblin's own installed D-Bus services (gnome-shell's
    # dbusServices: notifications, screencast, calendar, …) so tests can activate
    # them on demand. On-demand only — nothing auto-starts by adding the dir.
    prefix_service_dir = repo_root / "install" / "share" / "dbus-1" / "services"
    prefix_service_dir_xml = (
        f"  <servicedir>{html.escape(str(prefix_service_dir), quote=False)}</servicedir>\n"
        if prefix_service_dir.is_dir()
        else ""
    )
    conf.write_text(
        '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"\n'
        ' "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">\n'
        "<busconfig>\n"
        "  <type>session</type>\n"
        "  <keep_umask/>\n"
        "  <listen>unix:tmpdir=/tmp</listen>\n"
        "  <auth>EXTERNAL</auth>\n"
        f"  <servicedir>{service_dir_xml}</servicedir>\n"
        f"{prefix_service_dir_xml}"
        '  <policy context="default">\n'
        '    <allow send_destination="*" eavesdrop="true"/>\n'
        '    <allow eavesdrop="true"/>\n'
        '    <allow own="*"/>\n'
        "  </policy>\n"
        '  <limit name="max_incoming_bytes">1000000000</limit>\n'
        '  <limit name="max_incoming_unix_fds">250000000</limit>\n'
        '  <limit name="max_outgoing_bytes">1000000000</limit>\n'
        '  <limit name="max_outgoing_unix_fds">250000000</limit>\n'
        '  <limit name="max_message_size">1000000000</limit>\n'
        '  <limit name="service_start_timeout">120000</limit>\n'
        '  <limit name="auth_timeout">240000</limit>\n'
        '  <limit name="pending_fd_timeout">150000</limit>\n'
        '  <limit name="max_completed_connections">100000</limit>\n'
        '  <limit name="max_incomplete_connections">10000</limit>\n'
        '  <limit name="max_connections_per_user">100000</limit>\n'
        '  <limit name="max_pending_service_starts">10000</limit>\n'
        '  <limit name="max_names_per_connection">50000</limit>\n'
        '  <limit name="max_match_rules_per_connection">50000</limit>\n'
        '  <limit name="max_replies_per_connection">50000</limit>\n'
        "</busconfig>\n"
    )
    return conf


def main() -> int:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} TMPDIR REPO_ROOT", file=sys.stderr)
        return 2
    try:
        conf = write_config(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]))
    except Exception as exc:
        print(f"devkit-dbus: {exc}", file=sys.stderr)
        return 1
    print(conf)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
