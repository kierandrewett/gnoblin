"""CLI validation, D-Bus transport and correlated compositor replies."""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

loader = importlib.machinery.SourceFileLoader("gnoblinctl", str(Path(__file__).resolve().parents[1] / "src/tools/gnoblinctl"))
spec = importlib.util.spec_from_loader(loader.name, loader)
ctl = importlib.util.module_from_spec(spec)
loader.exec_module(ctl)


class CliTests(unittest.TestCase):
    def test_global_options_before_or_after_command(self):
        for words in (["--json", "window", "focus", "42"], ["window", "focus", "42", "--json"]):
            args = ctl.parser().parse_args(words)
            self.assertEqual((args.format, args.action, args.id), ("json", "focus", "42"))
        args = ctl.parser().parse_args(["window", "maximize"])
        self.assertEqual(args.id, "active")

    def test_invalid_values_are_rejected_before_transport(self):
        for words in (["window", "resize", "active", "0", "40"], ["--timeout", "0", "ping"],
                      ["window", "workspace", "2", "0"], ["launch", "begin", "token", "app", "-1"],
                      ["feature", "enable"], ["ping", "extra"], ["window", "move", "1", "nan", "2"]):
            with self.subTest(words=words), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                ctl.parser().parse_args(words)

    def test_dbus_arguments_are_separate_and_structures_are_decoded(self):
        run = Mock(return_value=subprocess.CompletedProcess([], 0, '{"type":"b","data":[false]}', ""))
        identity = "name with 'quotes'; $(nothing)"
        self.assertEqual(ctl.dbus("GetFeature", "s", [identity], run=run), [False])
        self.assertEqual(run.call_args.args[0][-2:], ["s", identity])
        self.assertEqual(run.call_count, 1)

    def test_input_fallback_is_only_for_absent_interfaces(self):
        call = Mock(side_effect=[ctl.CommandError("Unknown method ListInputSources"), [[]]])
        self.assertEqual(ctl.input_call("ListInputSources", call=call), [[]])
        self.assertEqual(call.call_args.kwargs["service"], "org.gnoblin.InputSources")
        for error in (ctl.CommandError("Unknown input source: xkb/Unknown method"), ctl.CommandError("Access denied"), subprocess.TimeoutExpired("busctl", 5)):
            call = Mock(side_effect=error)
            with self.assertRaises(type(error)): ctl.input_call("SetInputSource", "ss", ["xkb", "gb"], call=call)
            self.assertEqual(call.call_count, 1)

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
                        reply = json.dumps({"event": "reply", "id": record["id"], "result": {"windows": []}}).encode() + b"\n"
                        connection.sendall(reply[:9]); connection.sendall(reply[9:])
                worker = threading.Thread(target=respond)
                worker.start()
                self.assertEqual(ctl.compositor({"command": "windows"}, path), {"windows": []})
                worker.join()
                self.assertEqual(len(received), 1)
                self.assertEqual(received[0]["op"], "command")

    def test_output_is_safe_for_terminals_and_json_is_lossless(self):
        value = {"title": "text\n\x1b[2J'"}
        with contextlib.redirect_stdout(io.StringIO()) as output: ctl.render(value, "json")
        self.assertEqual(json.loads(output.getvalue()), value)
        with contextlib.redirect_stdout(io.StringIO()) as output: ctl.render(value, "table")
        self.assertNotIn("\x1b", output.getvalue())
        with contextlib.redirect_stdout(io.StringIO()) as output: ctl.render({"windowControlError": "offline"}, "table")
        self.assertEqual(output.getvalue(), "window control error: offline\n")
        with contextlib.redirect_stdout(io.StringIO()) as output: ctl.table([{"appId": "org.example.App"}])
        self.assertIn("APP ID", output.getvalue())

    def test_canonical_groups_are_discoverable_and_flat_commands_are_rejected(self):
        self.assertEqual(set(ctl.subcommands(ctl.parser())), {
            "ping", "version", "status", "reload", "privacy", "permissions", "window", "completion",
            "config", "workspace", "monitor", "input", "feature", "script", "grant", "launch",
        })
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(ctl.main(["feature"]), 0)
        self.assertIn("{list,show,enable,disable}", output.getvalue())
        for words in (("features",), ("reload-config",), ("input-sources",), ("load-config", "file.lua")):
            with self.subTest(words=words), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                ctl.parser().parse_args(words)

    def test_canonical_commands_keep_transport_arguments(self):
        cli = ctl.parser()
        cases = [
            (["config", "reload"], ("ReloadConfig", "", [])),
            (["script", "reload"], ("ReloadScripts", "", [])),
            (["feature", "enable", "osd"], ("SetFeature", "sb", ["osd", "true"])),
            (["grant", "revoke", "screen-cast", "grant-1"], ("RevokePortalGrant", "ss", ["screen-cast", "grant-1"])),
        ]
        for words, expected in cases:
            with self.subTest(words=words):
                args = cli.parse_args(words)
                args.timeout = 5
                args.socket = ""
                call = Mock()
                with patch.object(ctl, "dbus", call):
                    ctl.dispatch(args, cli)
                method, signature, values, service, timeout = call.call_args.args
                self.assertEqual((method, signature, list(values), service, timeout),
                                 (expected[0], expected[1], expected[2], "org.gnoblin.Shell", 5))

    def test_completion_uses_canonical_parser_groups(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            ctl.completions(ctl.parser(), "bash")
        script = output.getvalue()
        self.assertIn("feature:2) words='list show enable disable -j --json --format --timeout --socket'", script)
        self.assertIn("input:2) words='list current select -j --json --format --timeout --socket'", script)
        self.assertIn("window:list:3) words='-j --json --format --timeout --socket --app-id --title --focused'", script)
        self.assertNotIn("reload-config", script)


if __name__ == "__main__": unittest.main()
