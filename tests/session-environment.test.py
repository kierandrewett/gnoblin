#!/usr/bin/env python3
"""Check that the packaged login entry is the C session supervisor."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class SessionEnvironmentTests(unittest.TestCase):
    def test_desktop_installs_the_c_guardian_as_the_login_command(self):
        cmake = (ROOT / "CMakeLists.txt").read_text()
        installer = (ROOT / "cmake/install-session.cmake").read_text()
        desktop = (ROOT / "src/data/session/gnoblin.desktop").read_text()
        runtime = (ROOT / "src/session/gnoblin-runtime.c").read_text()
        portal_policy = (ROOT / "src/permissions/gnoblin-portal-policy.c").read_text()
        portal_session = portal_policy[
            portal_policy.index("static gboolean is_gnoblin_session(") : portal_policy.index(
                "GnoblinPermission gnoblin_permission_check("
            )
        ]
        self.assertIn('g_getenv("XDG_CURRENT_DESKTOP")', portal_session)
        self.assertNotIn("GNOME_SHELL_SESSION_MODE", portal_session)
        self.assertLess(
            runtime.index('g_unsetenv("GNOME_SHELL_SESSION_MODE")'), runtime.index("run_activation_update(sync)")
        )
        self.assertLess(runtime.index("clear_session_mode[]"), runtime.index("run_activation_update(sync)"))
        self.assertLess(runtime.index("run_activation_update(sync)"), runtime.index("reset_online_accounts_services()"))
        self.assertIn('"dbus-*-org.gnome.OnlineAccounts@*.service"', runtime)
        self.assertIn('"dbus-*-org.gnome.Identity@*.service"', runtime)
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
        self.assertIn("return ok;", runtime)
        self.assertNotIn("return ok || !required;", runtime)
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
            guardian.index("if (!devkit && !activate_session())"),
        )
        self.assertLess(
            guardian.index("if (!devkit && !activate_session())"),
            guardian.index("spawn_compositor(compositor_path"),
        )
        worker = runtime[
            runtime.index("static int runtime_worker_main(") : runtime.index("static GPid spawn_runtime_worker(")
        ]
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
