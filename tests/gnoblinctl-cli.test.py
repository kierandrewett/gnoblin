#!/usr/bin/env python3
"""Exercise the compiled gnoblinctl client without a running desktop session."""

import base64
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
from pathlib import Path


def run(
    binary: str,
    *arguments: str,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [binary, *arguments],
        check=False,
        capture_output=True,
        text=True,
        timeout=5,
        env=env,
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

    config_path = Path(build_directory) / "test-config" / "init.lua"
    config_result = run(
        binary,
        "--format",
        "json",
        "config",
        "path",
        env={**os.environ, "GNOBLIN_CONFIG": str(config_path)},
    )
    assert config_result.returncode == 0, config_result.stderr
    assert json.loads(config_result.stdout) == str(config_path)

    legacy_config_directory = Path(build_directory) / "legacy-config-home" / "gnoblin"
    legacy_config_directory.mkdir(parents=True, exist_ok=True)
    legacy_config = legacy_config_directory / "gnoblin.toml"
    legacy_config.touch()
    (legacy_config_directory / "gnoblin.conf").touch()
    legacy_config_result = run(
        binary,
        "--format",
        "json",
        "config",
        "path",
        env={
            **os.environ,
            "GNOBLIN_CONFIG": "",
            "XDG_CONFIG_HOME": str(legacy_config_directory.parent),
        },
    )
    assert legacy_config_result.returncode == 0, legacy_config_result.stderr
    assert json.loads(legacy_config_result.stdout) == str(legacy_config)

    invalid_result = run(binary, "not-a-command")
    assert invalid_result.returncode != 0
    assert "unknown command:" in invalid_result.stderr

    with tempfile.TemporaryDirectory(prefix="gnoblinctl-", dir=build_directory) as temporary:
        socket_path = str(Path(temporary) / "compositor.sock")
        received: list[dict[str, object]] = []
        subscriptions: list[dict[str, object]] = []
        server_error: list[BaseException] = []
        ready = threading.Event()

        def serve_once() -> None:
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                    server.bind(socket_path)
                    server.listen(9)
                    ready.set()
                    for _ in range(9):
                        connection, _ = server.accept()
                        with connection:
                            stream = connection.makefile("rwb")
                            request = json.loads(stream.readline())
                            if request["op"] == "events":
                                subscriptions.append(request)
                                response = {
                                    "event": "reply",
                                    "id": request["id"],
                                    "result": {"subscribed": True},
                                }
                                stream.write(json.dumps(response).encode() + b"\n")
                                stream.flush()
                                request = json.loads(stream.readline())
                            received.append(request)
                            if request["method"] == "session.status":
                                result = {"session": "test-session", "locked": False}
                            elif request["method"] == "monitor.list":
                                result = {"monitors": [{"id": "HDMI-1", "primary": True}]}
                            elif request["method"] == "animation.get":
                                result = None
                            elif request["method"] == "workspace.create":
                                result = {"request_id": 18, "method": "workspace.create"}
                            elif request["method"] == "workspace.list":
                                result = {"request_id": 19, "method": "workspace.list"}
                            elif request["method"] == "window.thumbnail":
                                result = {"request_id": 20, "method": "window.thumbnail"}
                            elif request["method"] == "window.match":
                                result = {
                                    "id": "42",
                                    "app_id": "org.example.Editor.desktop",
                                    "gtk_app_id": "org.example.Editor",
                                    "wm_class": "Editor",
                                    "rule_app_id": "org.example.Editor",
                                    "match": {
                                        "type": "window",
                                        "app_id": "org.example.Editor",
                                        "title": "Notes",
                                        "focused": True,
                                    },
                                }
                            elif request["method"] == "launch.status":
                                result = {
                                    "launches": [{"token": "one", "application": "app", "state": "pending"}],
                                    "revision": 4,
                                }
                            else:
                                result = {
                                    "request_id": 17,
                                    "method": "animation.preview",
                                    "target_type": "namespace",
                                    "target": "panel:test",
                                }
                            response = {"event": "reply", "id": request["id"], "result": result}
                            stream.write(json.dumps(response).encode() + b"\n")
                            stream.flush()
                            if request.get("method") in {
                                "animation.preview",
                                "workspace.create",
                                "workspace.list",
                                "window.thumbnail",
                            }:
                                operation_id = result["request_id"]
                                method = request["method"]
                                if method == "animation.preview":
                                    value = {"session": "preview-17"}
                                elif method == "workspace.create":
                                    value = {"id": "codex-probe", "name": "Codex Probe"}
                                elif method == "window.thumbnail":
                                    value = {
                                        "window_id": "42",
                                        "width": 1,
                                        "height": 1,
                                        "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p1sAAAAASUVORK5CYII=",
                                    }
                                else:
                                    value = {"workspaces": [{"id": "codex-probe", "name": "Codex Probe"}]}
                                completion = {
                                    "event": "gnoblin.operation.completed",
                                    "operation_id": operation_id,
                                    "method": method,
                                    "ok": True,
                                    "value": value,
                                }
                                stream.write(json.dumps(completion).encode() + b"\n")
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
        animation_get = run(binary, "--socket", socket_path, "--format", "json", "animation", "get", "missing")
        assert animation_get.returncode == 0, animation_get.stderr
        assert json.loads(animation_get.stdout) is None
        animation_preview = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "animation",
            "preview",
            "gnoblin-layer-open",
            "--namespace",
            "panel:test",
        )
        assert animation_preview.returncode == 0, animation_preview.stderr
        assert json.loads(animation_preview.stdout) == {"session": "preview-17"}
        workspace_create = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "workspace",
            "create",
            "--id",
            "codex-probe",
            "--name",
            "Codex Probe",
        )
        assert workspace_create.returncode == 0, workspace_create.stderr
        assert json.loads(workspace_create.stdout) == {
            "id": "codex-probe",
            "name": "Codex Probe",
        }
        workspace_list = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "workspace",
            "list",
        )
        assert workspace_list.returncode == 0, workspace_list.stderr
        assert json.loads(workspace_list.stdout) == {"workspaces": [{"id": "codex-probe", "name": "Codex Probe"}]}
        window_match = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "window",
            "match",
            "42",
        )
        assert window_match.returncode == 0, window_match.stderr
        assert json.loads(window_match.stdout) == {
            "id": "42",
            "app_id": "org.example.Editor.desktop",
            "gtk_app_id": "org.example.Editor",
            "wm_class": "Editor",
            "rule_app_id": "org.example.Editor",
            "match": {
                "type": "window",
                "app_id": "org.example.Editor",
                "title": "Notes",
                "focused": True,
            },
        }
        thumbnail_path = Path(temporary) / "window.png"
        thumbnail = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "window",
            "thumbnail",
            "42",
            "--output",
            str(thumbnail_path),
            "--width",
            "64",
            "--height",
            "64",
        )
        assert thumbnail.returncode == 0, thumbnail.stderr
        assert json.loads(thumbnail.stdout) == {
            "path": str(thumbnail_path),
            "width": 1,
            "height": 1,
        }
        assert thumbnail_path.read_bytes() == base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p1sAAAAASUVORK5CYII="
        )
        launch_status = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "launch",
            "status",
        )
        assert launch_status.returncode == 0, launch_status.stderr
        assert json.loads(launch_status.stdout) == {
            "launches": [{"token": "one", "application": "app", "state": "pending"}],
            "revision": 4,
        }
        server_thread.join(timeout=5)
        assert not server_thread.is_alive(), "mock compositor did not finish CLI requests"
        assert not server_error, repr(server_error)
        assert len(received) == 9
        assert len(subscriptions) == 9
        for subscription in subscriptions:
            assert subscription["op"] == "events"
            assert subscription["api_version"] == {"major": 1, "minor": 34}
            assert subscription["events"] == ["gnoblin.api.operation-completed"]
        request = received[0]
        assert request["op"] == "api"
        assert request["method"] == "session.status"
        assert request["api_version"] == {"major": 1, "minor": 29}
        assert request["arguments"] == {}
        monitor_request = received[1]
        assert monitor_request["op"] == "api"
        assert monitor_request["method"] == "monitor.list"
        assert monitor_request["arguments"] == {}
        get_request = received[2]
        assert get_request["method"] == "animation.get"
        assert get_request["api_version"] == {"major": 1, "minor": 18}
        assert get_request["arguments"] == {"name": "missing"}
        preview_request = received[3]
        assert preview_request["method"] == "animation.preview"
        assert preview_request["arguments"] == {
            "name": "gnoblin-layer-open",
            "target_type": "namespace",
            "target": "panel:test",
            "autoplay": False,
        }
        create_request = received[4]
        assert create_request["method"] == "workspace.create"
        assert create_request["arguments"] == {
            "id": "codex-probe",
            "name": "Codex Probe",
            "activate": False,
        }
        list_request = received[5]
        assert list_request["method"] == "workspace.list"
        assert list_request["arguments"] == {}
        match_request = received[6]
        assert match_request["method"] == "window.match"
        assert "api_version" not in match_request
        assert match_request["arguments"] == {"window": "42"}
        thumbnail_request = received[7]
        assert thumbnail_request["method"] == "window.thumbnail"
        assert thumbnail_request["api_version"] == {"major": 1, "minor": 23}
        assert thumbnail_request["arguments"] == {"id": "42", "width": 64, "height": 64}
        launch_status_request = received[8]
        assert launch_status_request["method"] == "launch.status"
        assert launch_status_request["api_version"] == {"major": 1, "minor": 8}
        assert launch_status_request["arguments"] == {}

    print("compiled gnoblinctl CLI smoke checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
