#!/usr/bin/env python3
"""Check the packaged supervisor and its environment handoff without services."""

import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _sources import RUNTIME_SOURCES  # noqa: E402


ROOT = Path(__file__).resolve().parents[1]


class SessionActivationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        directory = tempfile.TemporaryDirectory(prefix="gnoblin-session-environment-")
        cls.addClassCleanup(directory.cleanup)
        cls.binary = Path(directory.name) / "environment"
        flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-unix-2.0"], text=True))
        subprocess.run(
            [
                *shlex.split(os.environ.get("CC", "cc")),
                "-Wall",
                "-Werror",
                "-ffunction-sections",
                "-fdata-sections",
                str(ROOT / "tests/session-environment-fixture.c"),
                "-Wl,--gc-sections",
                "-o",
                str(cls.binary),
                *flags,
            ],
            check=True,
        )

    def activation(self, manager):
        result = subprocess.run([str(self.binary), manager], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        activation, marker, _cleanup = result.stdout.partition("ACTIVATED\n")
        self.assertTrue(marker, result.stdout)
        return [line.split("\t") for line in activation.splitlines()]

    def test_clears_stale_environment_before_sync_and_service_restarts(self):
        calls = self.activation("systemd")
        commands = [call[1:] for call in calls]
        sync = next(i for i, cmd in enumerate(commands) if "XDG_CURRENT_DESKTOP" in cmd)
        self.assertEqual(commands[sync][:2], ["dbus-update-activation-environment", "--systemd"])
        self.assertEqual(calls[sync][0], "<unset>")
        for variable in ("WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY", "GNOME_SETUP_DISPLAY"):
            self.assertTrue(
                any(cmd[:2] == commands[sync][:2] and f"{variable}=" in cmd for cmd in commands[:sync]),
                f"D-Bus activation retained {variable}",
            )
        for variable in ("WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY", "GNOME_SETUP_DISPLAY", "GNOME_SHELL_SESSION_MODE"):
            self.assertTrue(
                any(
                    cmd[:3] == ["systemctl", "--user", "unset-environment"] and variable in cmd
                    for cmd in commands[:sync]
                ),
                f"systemd retained {variable} before environment sync",
            )
        restart = commands.index(["systemctl", "--user", "try-restart", "xdg-desktop-portal.service"])
        self.assertLess(sync, restart)
        stops = [(i, cmd[3:]) for i, cmd in enumerate(commands) if cmd[:3] == ["systemctl", "--user", "stop"]]
        self.assertEqual(
            {unit for _, units in stops for unit in units},
            {
                "dbus-1-org.gnome.OnlineAccounts@1.service",
                "dbus-2-org.gnome.Identity@2.service",
                "dbus-3-org.a11y.atspi.Registry@3.service",
                "org.gnome.SettingsDaemon.ScreensaverProxy.service",
            },
        )
        for index, _units in stops:
            self.assertLess(sync, index)

    def test_dbus_only_session_does_not_invoke_systemd(self):
        calls = self.activation("no-systemd")
        self.assertTrue(calls)
        for _mode, command, *args in calls:
            self.assertEqual(command, "dbus-update-activation-environment")
            self.assertNotIn("--systemd", args)
        sync = next(call for call in calls if "XDG_CURRENT_DESKTOP" in call)
        self.assertEqual(sync[0], "<unset>")

    def test_failed_commands_report_failure_even_when_optional(self):
        for failure in ("spawn", "exit", "signal"):
            with self.subTest(failure=failure):
                result = subprocess.run(
                    [str(self.binary), f"fixture-{failure}-failure"], capture_output=True, text=True, timeout=10
                )
                self.assertEqual(result.returncode, 0, result.stderr)


class SessionEnvironmentTests(unittest.TestCase):
    def test_desktop_installs_the_c_guardian_as_the_login_command(self):
        cmake = (ROOT / "CMakeLists.txt").read_text()
        installer = (ROOT / "cmake/install-session.cmake").read_text()
        desktop = (ROOT / "src/data/session/gnoblin.desktop").read_text()
        runtime = RUNTIME_SOURCES.read_text()
        # Meson builds the compositor and the guardian as one executable through the
        # monolithic patch, so CMake defines no gnoblin target and the installer
        # only removes the old separate runtime helper.
        self.assertNotIn("add_executable(gnoblin\n", cmake)
        self.assertNotIn("add_custom_target(gnoblin-runtime ", cmake)
        monolithic = (ROOT / "patches/mutter/zz-monolithic-compositor/0001-build-monolithic-gnoblin.patch").read_text()
        self.assertIn("gnoblin_root / 'src/session/gnoblin-runtime.c'", monolithic)
        self.assertIn("gnoblin = executable('gnoblin',", monolithic)
        self.assertIn("libexec/gnoblin-runtime", installer)
        self.assertIn("Exec=gnoblin", desktop)
        self.assertIn("gnoblin: runtime supervisor:", runtime)
        self.assertIn("spawn_compositor(compositor_path", runtime)
        guardian = runtime[
            runtime.index("static int session_guardian_main(") : runtime.index("\nint gnoblin_runtime_main(")
        ]
        self.assertIn("g_canonicalize_filename(GNOBLIN_DEFAULT_COMPOSITOR, NULL)", guardian)
        self.assertLess(
            guardian.index("g_canonicalize_filename(GNOBLIN_DEFAULT_COMPOSITOR"),
            guardian.index("g_file_test(compositor_path, G_FILE_TEST_IS_EXECUTABLE)"),
        )
        self.assertLess(
            guardian.index("g_file_test(compositor_path, G_FILE_TEST_IS_EXECUTABLE)"),
            guardian.index("spawn_compositor(compositor_path"),
        )
        # Prepare the child's environment before spawn. Service activation can
        # overlap compositor startup, but must finish before the supervisor
        # starts configuration and autostart handling.
        self.assertLess(
            guardian.index("if (!devkit && !prepare_session_environment())"),
            guardian.index("spawn_compositor(compositor_path"),
        )
        self.assertLess(
            guardian.index("if (!devkit && !activate_session())"),
            guardian.index("spawn_session_supervisor("),
        )
        activation_failure = guardian[guardian.index("if (!devkit && !activate_session()) {") :]
        activation_failure = activation_failure[: activation_failure.index("\n    }")]
        self.assertIn("terminate_and_reap(&guardian);", activation_failure)
        self.assertIn("return EXIT_FAILURE;", activation_failure)
        worker = runtime[runtime.index("int runtime_worker_main(") : runtime.index("GPid spawn_runtime_worker(")]
        self.assertLess(
            worker.index("gnoblin_config_load_runtime_salvaged("), worker.index("send_config(&runtime, document")
        )
        self.assertLess(worker.index("send_config(&runtime, document"), worker.index("const guint8 started = 2;"))
        self.assertNotIn("GNOBLIN_RUNTIME_BINARY", installer)
        verify = (ROOT / ".github/workflows/verify.yml").read_text()
        self.assertIn('test ! -e "$GNOBLIN_TARBALL_TREE/install/libexec/gnoblin-runtime"', verify)
        self.assertFalse((ROOT / "src/tools/gnoblin").exists())


if __name__ == "__main__":
    unittest.main()
