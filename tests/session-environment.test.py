#!/usr/bin/env python3
"""Check that the packaged login entry is the C session supervisor."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class SessionEnvironmentTests(unittest.TestCase):
    def test_desktop_installs_the_c_supervisor_as_the_login_command(self):
        cmake = (ROOT / "CMakeLists.txt").read_text()
        installer = (ROOT / "scripts/install-session.sh").read_text()
        desktop = (ROOT / "src/data/session/gnoblin.desktop").read_text()
        runtime = (ROOT / "src/session/gnoblin-runtime.c").read_text()
        self.assertLess(
            runtime.index('g_unsetenv("GNOME_SHELL_SESSION_MODE")'), runtime.index("run_activation_update(sync)")
        )
        self.assertLess(runtime.index("clear_session_mode[]"), runtime.index("run_activation_update(sync)"))
        self.assertLess(runtime.index("run_activation_update(sync)"), runtime.index("reset_online_accounts_services()"))
        self.assertIn('"dbus-*-org.gnome.OnlineAccounts@*.service"', runtime)
        self.assertIn('"dbus-*-org.gnome.Identity@*.service"', runtime)
        self.assertIn("add_executable(gnoblin\n", cmake)
        self.assertNotIn("add_custom_target(gnoblin-runtime ", cmake)
        self.assertIn("gnoblinctl gnoblin", cmake)
        self.assertIn('install -Dm755 "$GNOBLIN_BINARY" "$INSTALL_PREFIX/bin/gnoblin"', installer)
        self.assertIn('"$INSTALL_PREFIX/libexec/gnoblin-runtime"', installer)
        self.assertIn("Exec=gnoblin", desktop)
        self.assertIn("gnoblin: runtime supervisor:", runtime)
        self.assertIn("spawn_compositor(compositor_path", runtime)
        self.assertIn("return ok;", runtime)
        self.assertNotIn("return ok || !required;", runtime)
        host = runtime[runtime.index("static int session_host_main(") : runtime.index("\nint main(")]
        self.assertIn("g_canonicalize_filename(GNOBLIN_DEFAULT_COMPOSITOR, NULL)", host)
        self.assertLess(
            host.index("g_canonicalize_filename(GNOBLIN_DEFAULT_COMPOSITOR"),
            host.index("g_file_test(compositor_path, G_FILE_TEST_IS_EXECUTABLE)"),
        )
        self.assertLess(
            host.index("g_file_test(compositor_path, G_FILE_TEST_IS_EXECUTABLE)"),
            host.index("if (!devkit && !activate_session())"),
        )
        self.assertLess(
            host.index("if (!devkit && !activate_session())"), host.index("spawn_compositor(compositor_path")
        )
        worker = runtime[
            runtime.index("static int runtime_worker_main(") : runtime.index("static GPid spawn_runtime_worker(")
        ]
        self.assertLess(worker.index("gnoblin_config_load_runtime("), worker.index("send_config(&runtime, document"))
        self.assertLess(worker.index("send_config(&runtime, document"), worker.index("const guint8 started = 2;"))
        self.assertNotIn('install -Dm755 "$GNOBLIN_RUNTIME_BINARY"', installer)
        verify = (ROOT / ".github/workflows/verify.yml").read_text()
        self.assertIn('test ! -e "$GNOBLIN_TARBALL_TREE/install/libexec/gnoblin-runtime"', verify)
        self.assertFalse((ROOT / "src/tools/gnoblin").exists())


if __name__ == "__main__":
    unittest.main()
