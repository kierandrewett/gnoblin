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
from unittest.mock import Mock

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
                      ["window", "workspace", "2", "0"], ["launch-begin", "token", "app", "-1"],
                      ["enable"], ["ping", "extra"], ["window", "move", "1", "nan", "2"]):
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


if __name__ == "__main__": unittest.main()
