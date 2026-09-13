#!/usr/bin/env python3
"""Regression test for Gnoblin's D-Bus activation session handoff."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class SessionEnvironmentTests(unittest.TestCase):
    def test_activation_environment_is_synced_before_stale_goa_is_stopped(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            prefix = base / "runtime"
            (prefix / "bin").mkdir(parents=True)
            (prefix / "libexec").mkdir()
            shutil.copy2(ROOT / "src/tools/gnoblin-session", prefix / "bin/gnoblin-session")
            shutil.copy2(ROOT / "src/tools/gnoblin-env.sh", prefix / "libexec/gnoblin-env.sh")

            fake_dbus = prefix / "bin/dbus-update-activation-environment"
            fake_dbus.write_text(
                "#!/bin/sh\n"
                'printf \'dbus %s desktop=%s mode=%s\n\' "$*" "$XDG_CURRENT_DESKTOP" '
                '"$GNOME_SHELL_SESSION_MODE" >> "$GNOBLIN_TEST_LOG"\n'
            )
            fake_dbus.chmod(0o755)

            fake_systemctl = prefix / "bin/systemctl"
            fake_systemctl.write_text(
                "#!/bin/sh\n"
                'case "$*" in\n'
                "  *'list-units'*) printf '%s\\n' "
                "'dbus-:1.2-org.gnome.OnlineAccounts@0.service' "
                "'dbus-:1.2-org.gnome.Identity@0.service' ;;\n"
                "  *'stop'*) printf 'stop %s\\n' \"$*\" >> \"$GNOBLIN_TEST_LOG\" ;;\n"
                '  *) printf \'unexpected systemctl %s\\n\' "$*" >> "$GNOBLIN_TEST_LOG"; exit 1 ;;\n'
                "esac\n"
            )
            fake_systemctl.chmod(0o755)

            fake_session = prefix / "bin/gnome-session"
            fake_session.write_text(
                "#!/bin/sh\n"
                "printf 'session desktop=%s mode=%s args=%s\\n' \"$XDG_CURRENT_DESKTOP\" "
                '"$GNOME_SHELL_SESSION_MODE" "$*" >> "$GNOBLIN_TEST_LOG"\n'
            )
            fake_session.chmod(0o755)

            log = base / "events"
            environment = {
                **os.environ,
                "PATH": f"{prefix / 'bin'}:/usr/local/bin:/usr/bin:/bin",
                "GNOBLIN_TEST_LOG": str(log),
                "XDG_SESSION_DESKTOP": "gnoblin",
                "XDG_SESSION_TYPE": "wayland",
                "WAYLAND_DISPLAY": "wayland-9",
            }
            subprocess.run(
                [str(prefix / "bin/gnoblin-session"), "--test-argument"],
                env=environment,
                check=True,
            )

            events = log.read_text().splitlines()
            self.assertEqual(
                events[0],
                "dbus --systemd GNOME_SHELL_SESSION_MODE XDG_CURRENT_DESKTOP "
                "XDG_SESSION_DESKTOP XDG_SESSION_TYPE WAYLAND_DISPLAY DISPLAY XAUTHORITY "
                "desktop=GNOME:Gnoblin mode=gnoblin",
            )
            self.assertIn("stop --user stop dbus-:1.2-org.gnome.OnlineAccounts@0.service", events[1])
            self.assertIn("stop --user stop dbus-:1.2-org.gnome.Identity@0.service", events[2])
            self.assertEqual(
                events[3],
                "session desktop=GNOME:Gnoblin mode=gnoblin args=--no-reexec --session=gnoblin --test-argument",
            )


if __name__ == "__main__":
    unittest.main()
