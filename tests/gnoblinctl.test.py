"""CLI validation and correlated compositor API replies."""

import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

loader = importlib.machinery.SourceFileLoader(
    "gnoblinctl", str(Path(__file__).resolve().parents[1] / "src/tools/gnoblinctl")
)
spec = importlib.util.spec_from_loader(loader.name, loader)
ctl = importlib.util.module_from_spec(spec)
loader.exec_module(ctl)


class CliTests(unittest.TestCase):
    def test_config_default_prints_installed_example_without_a_running_session(self):
        with tempfile.TemporaryDirectory() as temporary:
            prefix = Path(temporary)
            command = prefix / "bin/gnoblinctl"
            example = prefix / "share/gnoblin/init.lua.example"
            command.parent.mkdir()
            example.parent.mkdir(parents=True)
            command.write_bytes(Path(ctl.__file__).read_bytes())
            example.write_text("-- bundled default\ngnoblin.configure {}\n", encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(command), "config", "default"],
                capture_output=True,
                text=True,
                env={**os.environ, "XDG_DATA_DIRS": str(prefix / "other-share")},
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, example.read_text(encoding="utf-8"))
            self.assertEqual(result.stderr, "")

    def test_shortcut_capture_prints_the_compositor_accelerator_and_waits_by_default(self):
        with (
            contextlib.redirect_stdout(io.StringIO()) as output,
            contextlib.redirect_stderr(io.StringIO()),
            patch.dict(os.environ, {"GNOBLIN_COMPOSITOR_SOCKET": "/run/user/test.sock"}),
            patch.object(ctl, "api_call", return_value={"accelerator": "<Shift><Control>q"}) as call,
        ):
            self.assertEqual(ctl.main(["shortcut", "capture"]), 0)
        self.assertEqual(output.getvalue(), "<Shift><Control>q\n")
        call.assert_called_once_with("shortcut.capture", {"timeout": 30}, "/run/user/test.sock", 32)

    def test_global_options_before_or_after_command(self):
        for words in (["--json", "window", "focus", "42"], ["window", "focus", "42", "--json"]):
            args = ctl.parser().parse_args(words)
            self.assertEqual((args.format, args.action, args.id), ("json", "focus", "42"))
        args = ctl.parser().parse_args(["window", "maximize"])
        self.assertEqual(args.id, "active")

    def test_invalid_values_are_rejected_before_transport(self):
        for words in (
            ["window", "resize", "active", "0", "40"],
            ["--timeout", "0", "ping"],
            ["window", "workspace", "2", "--number", "0"],
            ["launch", "begin", "token", "app", "-1"],
            ["feature", "list"],
            ["ping", "extra"],
            ["window", "move", "1", "nan", "2"],
            ["script", "list"],
        ):
            with self.subTest(words=words), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                ctl.parser().parse_args(words)

    def test_api_arguments_are_json_values_not_shell_fragments(self):
        identity = "name with 'quotes'; $(nothing)"
        with patch.object(ctl, "compositor", return_value={"level": "deny"}) as call:
            self.assertEqual(
                ctl.api_call(
                    "permissions.check",
                    {"capability": "screen-cast", "identity": identity},
                    "/run/user/test.sock",
                ),
                {"level": "deny"},
            )
        self.assertEqual(
            call.call_args.args[0]["arguments"],
            {"capability": "screen-cast", "identity": identity},
        )

    def test_socket_handles_fragments_and_unrelated_events(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = str(Path(temporary) / "compositor.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen()
                received = []

                def respond():
                    with server.accept()[0] as connection:
                        record = json.loads(connection.makefile("rb").readline())
                        received.append(record)
                        connection.sendall(b'{"event":"hello","version":1}\n')
                        reply = (
                            json.dumps({"event": "reply", "id": record["id"], "result": {"windows": []}}).encode()
                            + b"\n"
                        )
                        connection.sendall(reply[:9])
                        connection.sendall(reply[9:])

                worker = threading.Thread(target=respond)
                worker.start()
                self.assertEqual(ctl.compositor({"command": "windows"}, path), {"windows": []})
                worker.join()
                self.assertEqual(len(received), 1)
                self.assertEqual(received[0]["op"], "command")

    def test_animation_get_accepts_a_null_result(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = str(Path(temporary) / "compositor.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen()
                received = []

                def respond():
                    with server.accept()[0] as connection:
                        record = json.loads(connection.makefile("rb").readline())
                        received.append(record)
                        connection.sendall(
                            (json.dumps({"event": "reply", "id": record["id"], "result": None}) + "\n").encode()
                        )

                worker = threading.Thread(target=respond)
                worker.start()
                self.assertIsNone(ctl.api_call("animation.get", {"name": "missing"}, path))
                worker.join()
                self.assertEqual(received[0]["api_version"], {"major": 1, "minor": 18})

            os.unlink(path)
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen()

                def respond_invalid():
                    with server.accept()[0] as connection:
                        record = json.loads(connection.makefile("rb").readline())
                        connection.sendall(
                            (json.dumps({"event": "reply", "id": record["id"], "result": None}) + "\n").encode()
                        )

                worker = threading.Thread(target=respond_invalid)
                worker.start()
                with self.assertRaises(ctl.CommandError):
                    ctl.api_call("animation.list", {}, path)
                worker.join()

    def test_compositor_accepts_direct_array_snapshot_results(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = str(Path(temporary) / "compositor.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen()
                received = []

                def respond():
                    with server.accept()[0] as connection:
                        record = json.loads(connection.makefile("rb").readline())
                        received.append(record)
                        connection.sendall(
                            (json.dumps({"event": "reply", "id": record["id"], "result": []}) + "\n").encode()
                        )

                worker = threading.Thread(target=respond)
                worker.start()
                self.assertEqual(ctl.api_call("focus.history", {}, path), [])
                worker.join()
                self.assertEqual(received[0]["api_version"], {"major": 1, "minor": 19})

    def test_output_is_safe_for_terminals_and_json_is_lossless(self):
        value = {"title": "text\n\x1b[2J'"}
        with contextlib.redirect_stdout(io.StringIO()) as output:
            ctl.render(value, "json")
        self.assertEqual(json.loads(output.getvalue()), value)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            ctl.render(value, "table")
        self.assertNotIn("\x1b", output.getvalue())
        with contextlib.redirect_stdout(io.StringIO()) as output:
            ctl.render({"windowControlError": "offline"}, "table")
        self.assertEqual(output.getvalue(), "window control error: offline\n")

    def test_privacy_uses_version_117_and_human_output_marks_unavailable_sources(self):
        with patch.object(ctl, "compositor", return_value={"available": {}, "revision": 1}) as call:
            self.assertEqual(ctl.api_call("privacy.state", {}, "/run/user/test.sock"), {"available": {}, "revision": 1})
        request = call.call_args.args[0]
        self.assertEqual(request["method"], "privacy.state")
        self.assertEqual(request["api_version"], {"major": 1, "minor": 17})

        value = {
            "screen_sharing": True,
            "microphone_in_use": False,
            "available": {
                "screen_sharing": True,
                "microphone_in_use": True,
                "camera_in_use": False,
                "location_in_use": False,
            },
            "revision": 2,
        }
        with contextlib.redirect_stdout(io.StringIO()) as output:
            ctl.render(value, "table")
        self.assertEqual(
            output.getvalue(),
            "Screen sharing: active\nMicrophone: inactive\nCamera: unavailable\nLocation: unavailable\n",
        )
        with contextlib.redirect_stdout(io.StringIO()) as output:
            ctl.table([{"appId": "org.example.App"}])
        self.assertIn("APP ID", output.getvalue())

    def test_shared_snapshot_reads_use_api_version_119(self):
        methods = [
            ("version", {}),
            ("capabilities.list", {}),
            ("focus.history", {"limit": 7}),
            ("settings", {}),
            ("focus.policy", {}),
        ]
        for method, arguments in methods:
            with self.subTest(method=method), patch.object(ctl, "compositor", return_value=[]) as call:
                self.assertEqual(ctl.api_call(method, arguments, "/run/user/test.sock"), [])
                request = call.call_args.args[0]
                self.assertEqual(request["method"], method)
                self.assertEqual(request["arguments"], arguments)
                self.assertEqual(request["api_version"], {"major": 1, "minor": 19})

    def test_runtime_config_reload_uses_api_version_120(self):
        with patch.object(ctl, "compositor", return_value={"ok": True}) as call:
            self.assertEqual(
                ctl.api_call("runtime.reload_config", {}, "/run/user/test.sock"),
                {"ok": True},
            )
        request = call.call_args.args[0]
        self.assertEqual(request["method"], "runtime.reload_config")
        self.assertEqual(request["api_version"], {"major": 1, "minor": 20})
        self.assertEqual(call.call_args.kwargs["wait_for_operation"], "runtime.reload_config")

    def test_runtime_config_reload_returns_its_completion_result(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = str(Path(temporary) / "compositor.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen()
                received = []

                def respond():
                    with server.accept()[0] as connection:
                        record = json.loads(connection.makefile("rb").readline())
                        received.append(record)
                        connection.sendall(
                            json.dumps(
                                {
                                    "event": "reply",
                                    "id": record["id"],
                                    "result": {"method": "runtime.reload_config", "request_id": 7},
                                }
                            ).encode()
                            + b"\n"
                        )
                        connection.sendall(
                            b'{"event":"gnoblin.operation.completed","operation_id":7,'
                            b'"method":"runtime.reload_config","ok":true,'
                            b'"value":{"ok":true,"action":"applied","runtime_generation":2}}\n'
                        )

                worker = threading.Thread(target=respond)
                worker.start()
                self.assertEqual(
                    ctl.api_call("runtime.reload_config", {}, path, timeout=1),
                    {"ok": True, "action": "applied", "runtime_generation": 2},
                )
                worker.join()
                self.assertEqual(received[0]["method"], "runtime.reload_config")

    def test_snapshot_cli_commands_use_shared_lua_reads(self):
        cli = ctl.parser()
        args = cli.parse_args(["version"])
        args.timeout, args.socket = 5, "/run/user/test.sock"
        with patch.object(ctl, "api_call", return_value={"gnoblin": "0.1.0"}) as call:
            self.assertEqual(ctl.dispatch(args, cli), {"gnoblin": "0.1.0"})
        call.assert_called_once_with("version", {}, "/run/user/test.sock", 5)

        args = cli.parse_args(["config", "show"])
        args.timeout, args.socket = 5, "/run/user/test.sock"
        with patch.object(ctl, "api_call", return_value={"revision": 4}) as call:
            self.assertEqual(ctl.dispatch(args, cli), {"revision": 4})
        call.assert_called_once_with("settings", {}, "/run/user/test.sock", 5)

        args = cli.parse_args(["focus", "history", "--workspace-id", "ws-1", "--limit", "3"])
        args.timeout, args.socket = 5, "/run/user/test.sock"
        with patch.object(ctl, "api_call", return_value=[]) as call:
            self.assertEqual(ctl.dispatch(args, cli), [])
        call.assert_called_once_with("focus.history", {"workspace_id": "ws-1", "limit": 3}, "/run/user/test.sock", 5)

        args = cli.parse_args(["focus", "policy"])
        args.timeout, args.socket = 5, "/run/user/test.sock"
        with patch.object(ctl, "api_call", return_value={"focus_mode": "click"}) as call:
            self.assertEqual(ctl.dispatch(args, cli), {"focus_mode": "click"})
        call.assert_called_once_with("focus.policy", {}, "/run/user/test.sock", 5)

        args = cli.parse_args(["capabilities"])
        args.timeout, args.socket = 5, "/run/user/test.sock"
        with patch.object(ctl, "api_call", return_value=[]) as call:
            self.assertEqual(ctl.dispatch(args, cli), [])
        call.assert_called_once_with("capabilities.list", {}, "/run/user/test.sock", 5)

    def test_canonical_groups_are_discoverable_and_flat_commands_are_rejected(self):
        self.assertEqual(
            set(ctl.subcommands(ctl.parser())),
            {
                "status",
                "ping",
                "version",
                "reload",
                "logout",
                "privacy",
                "permissions",
                "window",
                "layer",
                "completion",
                "config",
                "workspace",
                "monitor",
                "input",
                "grant",
                "launch",
                "animation",
                "focus",
                "capabilities",
                "shortcut",
            },
        )
        for words in (
            ("features",),
            ("feature", "list"),
            ("script", "list"),
            ("reload-config",),
            ("input-sources",),
            ("load-config", "file.lua"),
        ):
            with self.subTest(words=words), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                ctl.parser().parse_args(words)

    def test_canonical_commands_keep_transport_arguments(self):
        cli = ctl.parser()
        cases = [
            (["status"], ("session.status", {})),
            (["logout"], ("session.logout", {})),
            (["reload"], ("runtime.reload_config", {})),
            (["config", "reload"], ("runtime.reload_config", {})),
            (["grant", "revoke", "screen-cast", "grant-1"], ("grant.revoke", {"kind": "screen-cast", "id": "grant-1"})),
        ]
        for words, expected in cases:
            with self.subTest(words=words):
                args = cli.parse_args(words)
                args.timeout = 5
                args.socket = "/run/user/test.sock"
                with patch.object(ctl, "api_call", return_value={"pong": "pong"}) as call:
                    ctl.dispatch(args, cli)
                call.assert_called_once_with(expected[0], expected[1], "/run/user/test.sock", 5)

    def test_ping_dispatch_uses_transport_operation(self):
        cli = ctl.parser()
        args = cli.parse_args(["ping"])
        args.timeout = 5
        args.socket = "/run/user/test.sock"
        with patch.object(ctl, "ping_compositor", return_value="pong") as ping:
            result = ctl.dispatch(args, cli)
        self.assertEqual(result, "pong")
        ping.assert_called_once_with("/run/user/test.sock", 5)

    def test_ping_round_trip_has_no_api_method_or_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = str(Path(temporary) / "compositor.sock")
            received = []
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen()

                def respond():
                    with server.accept()[0] as connection:
                        stream = connection.makefile("rb")
                        request = json.loads(stream.readline())
                        received.append(request)
                        connection.sendall(b'{"event":"hello","version":1,"api_minor":29}\n')
                        connection.sendall(
                            (
                                json.dumps({"event": "reply", "id": request["id"], "result": {"pong": "pong"}}) + "\n"
                            ).encode()
                        )

                worker = threading.Thread(target=respond)
                worker.start()
                result = ctl.ping_compositor(path)
                worker.join()
            self.assertEqual(result, "pong")
            self.assertEqual(received[0]["op"], "ping")
            self.assertNotIn("method", received[0])
            self.assertNotIn("api_version", received[0])
            self.assertNotIn("arguments", received[0])

    def test_status_round_trip_uses_api_129_and_preserves_lock_unavailability(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = str(Path(temporary) / "compositor.sock")
            status = {"state": "running", "lock_available": False}
            received = []
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen()

                def respond():
                    with server.accept()[0] as connection:
                        stream = connection.makefile("rb")
                        request = json.loads(stream.readline())
                        received.append(request)
                        connection.sendall(b'{"event":"hello","version":1,"api_minor":29}\n')
                        connection.sendall(
                            (json.dumps({"event": "reply", "id": request["id"], "result": status}) + "\n").encode()
                        )

                worker = threading.Thread(target=respond)
                worker.start()
                result = ctl.api_call("session.status", {}, path)
                worker.join()
            self.assertEqual(result, status)
            self.assertEqual(received[0]["method"], "session.status")
            self.assertEqual(received[0]["arguments"], {})
            self.assertEqual(received[0]["api_version"], {"major": 1, "minor": 29})

    def test_animation_get_dispatches_canonical_method_with_name(self):
        cli = ctl.parser()
        args = cli.parse_args(["animation", "get", "gnome-open"])
        args.timeout = 5
        args.socket = "/run/user/test.sock"
        with patch.object(ctl, "api_call", return_value={"name": "gnome-open"}) as call:
            result = ctl.dispatch(args, cli)
        self.assertEqual(result, {"name": "gnome-open"})
        call.assert_called_once_with("animation.get", {"name": "gnome-open"}, "/run/user/test.sock", 5)

        with patch.object(ctl, "compositor", return_value={"name": "gnome-open"}) as call:
            ctl.api_call("animation.get", {"name": "gnome-open"}, "/run/user/test.sock")
        self.assertEqual(call.call_args.args[0]["api_version"], {"major": 1, "minor": 18})

        with patch.object(ctl, "compositor", return_value={"state": "running", "lock_available": False}) as call:
            ctl.api_call("session.status", {}, "/run/user/test.sock")
        self.assertEqual(call.call_args.args[0]["api_version"], {"major": 1, "minor": 29})

    def test_logout_and_xdg_focus_use_api_132(self):
        with patch.object(ctl, "compositor", return_value={"accepted": True}) as call:
            ctl.api_call("session.logout", {}, "/run/user/test.sock")
        self.assertEqual(call.call_args.args[0]["api_version"], {"major": 1, "minor": 32})

        with patch.object(ctl, "compositor", return_value={"id": "42"}) as call:
            ctl.api_call(
                "window.focus",
                {"id": "42", "activation_token": "token"},
                "/run/user/test.sock",
            )
        self.assertEqual(call.call_args.args[0]["api_version"], {"major": 1, "minor": 32})

    def test_completion_uses_canonical_parser_groups(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            ctl.completions(ctl.parser(), "bash")
        script = output.getvalue()
        self.assertIn("input:2) words='list current select -j --json --format --timeout --socket'", script)
        self.assertIn("window:list:3) words='-j --json --format --timeout --socket --app-id --title --focused'", script)
        self.assertNotIn("reload-config", script)
        self.assertNotIn("feature", script)
        self.assertNotIn("script", script)


if __name__ == "__main__":
    unittest.main()
