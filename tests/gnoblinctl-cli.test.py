#!/usr/bin/env python3
"""Exercise the compiled gnoblinctl client without a running desktop session."""

import json
import socket
import subprocess
import sys
import tempfile
import threading
from pathlib import Path


def run(binary: str, *arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [binary, *arguments],
        check=False,
        capture_output=True,
        text=True,
        timeout=5,
    )


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: gnoblinctl-cli.test.py BINARY BUILD_DIRECTORY")

    binary, build_directory = sys.argv[1:]

    help_result = run(binary, "--help")
    assert help_result.returncode == 0, help_result.stderr
    assert "Usage: gnoblinctl" in help_result.stdout

    version_result = run(binary, "--version", "--format", "json")
    assert version_result.returncode == 0, version_result.stderr
    identity = json.loads(version_result.stdout)
    for field in ("version", "gnomeVersion", "mutterApi", "gitRemote", "gitSha"):
        assert isinstance(identity.get(field), str) and identity[field], field
    assert isinstance(identity.get("sourceModified"), bool)
    components = identity.get("components")
    assert isinstance(components, dict)
    for component in ("mutter", "xdg-desktop-portal-gnome"):
        assert isinstance(components.get(component), str) and components[component], component
    component_commits = identity.get("componentCommits")
    assert isinstance(component_commits, dict)
    for component in ("mutter", "xdg-desktop-portal-gnome"):
        assert isinstance(component_commits.get(component), str) and component_commits[component], component

    invalid_result = run(binary, "not-a-command")
    assert invalid_result.returncode != 0
    assert "unknown command:" in invalid_result.stderr

    with tempfile.TemporaryDirectory(prefix="gnoblinctl-", dir=build_directory) as temporary:
        socket_path = str(Path(temporary) / "compositor.sock")
        received: list[dict[str, object]] = []
        server_error: list[BaseException] = []
        ready = threading.Event()

        def serve_once() -> None:
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                    server.bind(socket_path)
                    server.listen(2)
                    ready.set()
                    for _ in range(2):
                        connection, _ = server.accept()
                        with connection:
                            stream = connection.makefile("rwb")
                            request = json.loads(stream.readline())
                            received.append(request)
                            if request["method"] == "session.status":
                                result = {"session": "test-session", "locked": False}
                            else:
                                result = {"monitors": [{"id": "HDMI-1", "primary": True}]}
                            response = {"event": "reply", "id": request["id"], "result": result}
                            stream.write(json.dumps(response).encode() + b"\n")
                            stream.flush()
            except BaseException as error:  # propagate background-thread failures
                server_error.append(error)

        server_thread = threading.Thread(target=serve_once, daemon=True)
        server_thread.start()
        assert ready.wait(timeout=5), repr(server_error)
        result = run(binary, "--socket", socket_path, "--format", "json", "status")
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout) == {"session": "test-session", "locked": False}
        monitor_result = run(binary, "--socket", socket_path, "--format", "json", "monitor", "list")
        assert monitor_result.returncode == 0, monitor_result.stderr
        assert json.loads(monitor_result.stdout) == {"monitors": [{"id": "HDMI-1", "primary": True}]}
        server_thread.join(timeout=5)
        assert not server_thread.is_alive(), "mock compositor did not finish monitor request"
        assert not server_error, repr(server_error)
        assert len(received) == 2
        request = received[0]
        assert request["op"] == "api"
        assert request["method"] == "session.status"
        assert request["api_version"] == {"major": 1, "minor": 29}
        assert request["arguments"] == {}
        monitor_request = received[1]
        assert monitor_request["op"] == "api"
        assert monitor_request["method"] == "monitor.list"
        assert monitor_request["arguments"] == {}

    print("compiled gnoblinctl CLI smoke checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
