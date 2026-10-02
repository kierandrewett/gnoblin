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

    lua_help = run(binary, "lua", "--help")
    assert lua_help.returncode == 0, lua_help.stderr
    assert "gnoblinctl lua [FILE]" in lua_help.stdout
    lua_repl = subprocess.run(
        [binary, "lua"],
        check=False,
        capture_output=True,
        text=True,
        input='=1 + 1\n={name = "Gnoblin", values = {1, 2}}\n:quit\n',
        timeout=5,
    )
    assert lua_repl.returncode == 0, lua_repl.stderr
    assert "2\n" in lua_repl.stdout
    assert '"name" : "Gnoblin"' in lua_repl.stdout

    interactive_result = run(binary, "window", "interactive-move")
    assert interactive_result.returncode != 0
    assert "require a trusted shell input context" in interactive_result.stderr
    assert "cannot create one" in interactive_result.stderr

    version_result = run(binary, "--version", "--format", "json")
    assert version_result.returncode == 0, version_result.stderr
    identity = json.loads(version_result.stdout)
    for field in (
        "version",
        "gnomeVersion",
        "mutterApi",
        "luaVersion",
        "apiVersion",
        "buildId",
        "gitRemote",
        "gitSha",
    ):
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

    human_version = run(binary, "--version")
    assert human_version.returncode == 0, human_version.stderr
    for label, field in (
        ("Lua", "luaVersion"),
        ("Native API", "apiVersion"),
        ("Build ID", "buildId"),
    ):
        assert f"{label}: {identity[field]}" in human_version.stdout

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
                    for _ in range(40):
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
                            elif request["method"] == "monitors.list":
                                result = [{"id": "HDMI-1", "index": 0, "primary": True, "revision": 5}]
                            elif request["method"] == "windows.list":
                                result = [
                                    {
                                        "id": "42",
                                        "focused": True,
                                        "workspace_id": "workspace-1",
                                        "app_id": "org.example.Editor.desktop",
                                        "gtk_app_id": "org.example.Editor",
                                        "wm_class": "Editor",
                                        "rule_app_id": "org.example.Editor",
                                        "title": "Notes",
                                        "revision": 5,
                                    }
                                ]
                            elif request["method"] == "animation.get":
                                result = None
                            elif request["method"] == "workspace.create":
                                result = {"request_id": 18, "method": "workspace.create"}
                            elif request["method"] == "workspaces.list":
                                result = [
                                    {
                                        "id": "codex-probe",
                                        "number": 1,
                                        "name": "Codex Probe",
                                        "active": True,
                                        "window_count": 2,
                                        "revision": 5,
                                    }
                                ]
                            elif request["method"] == "layers.list":
                                result = [
                                    {
                                        "id": "surface-1",
                                        "title": "Panel",
                                        "namespace": "panel:top",
                                        "layer": "top",
                                        "monitor_id": "HDMI-1",
                                        "revision": 5,
                                    }
                                ]
                            elif request["method"] == "input.sources":
                                result = {
                                    "sources": [
                                        {"type": "xkb", "id": "us", "short_name": "en", "name": "English (US)"}
                                    ],
                                    "revision": 5,
                                }
                            elif request["method"] == "input.current_source":
                                result = {
                                    "available": True,
                                    "source": {"type": "xkb", "id": "us", "short_name": "en", "name": "English (US)"},
                                    "revision": 5,
                                }
                            elif request["method"] == "input.devices":
                                result = {"devices": [{"id": "input:1", "name": "Test keyboard"}], "revision": 11}
                            elif request["method"] == "privacy.state":
                                result = {
                                    "available": {
                                        "screen_sharing": True,
                                        "recording": True,
                                        "microphone_in_use": False,
                                        "camera_in_use": False,
                                        "location_in_use": False,
                                    },
                                    "screen_sharing": True,
                                    "recording": False,
                                    "revision": 42,
                                }
                            elif request["method"] == "input.select":
                                result = {"request_id": 23, "method": "input.select"}
                            elif request["method"] == "window.thumbnail":
                                result = {"request_id": 20, "method": "window.thumbnail"}
                            elif request["method"] == "launches.snapshot":
                                result = {
                                    "launches": [{"token": "one", "application": "app", "state": "pending"}],
                                    "revision": 4,
                                }
                            elif request["method"] == "shortcuts.list":
                                result = [
                                    {
                                        "name": "test.shortcut",
                                        "binding": "<Super>space",
                                        "enabled": True,
                                        "trigger": "press",
                                        "action": "test.action",
                                        "revision": 6,
                                    }
                                ]
                            elif request["method"] == "shortcuts.actions":
                                result = [
                                    {
                                        "id": "wm.close",
                                        "group": "wm",
                                        "key": "close",
                                        "default_bindings": ["<Alt>F4"],
                                    }
                                ]
                            elif request["method"] == "permissions.list":
                                result = {
                                    "policy": {"default": "deny", "rules": []},
                                    "capabilities": ["screen-cast", "remote-desktop"],
                                    "levels": ["deny", "ask", "allow"],
                                    "path": "/tmp/gnoblin-permissions.json",
                                }
                            elif request["method"] == "permissions.check":
                                result = {
                                    "level": "allow",
                                    "rule": "remote-test",
                                    "monitors": [],
                                    "devices": ["keyboard"],
                                    "clipboard": True,
                                    "revision": 9,
                                }
                            elif request["method"] == "permissions.policy":
                                result = {"default": "default", "rules": [], "revision": 9}
                            elif request["method"] == "window.move_to_monitor":
                                result = {"id": "42", "monitor_id": "HDMI-1"}
                            elif request["method"] == "window.restore_or_minimize":
                                result = {"id": "42", "action": "restore"}
                            elif request["method"] == "window.unminimize":
                                result = {"id": "42"}
                            elif request["method"] == "window.minimize":
                                result = {"request_id": 24, "method": "window.minimize"}
                            elif request["method"] == "workspace.switch":
                                result = {"request_id": 25, "method": "workspace.switch"}
                            elif request["method"] == "workspace.rename":
                                result = {"request_id": 26, "method": "workspace.rename"}
                            elif request["method"] == "workspace.move_window":
                                result = {"request_id": 27, "method": "workspace.move_window"}
                            elif request["method"] == "workspace.remove":
                                result = {"request_id": 28, "method": "workspace.remove"}
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
                                "window.thumbnail",
                                "input.select",
                                "window.minimize",
                                "workspace.switch",
                                "workspace.rename",
                                "workspace.move_window",
                                "workspace.remove",
                            }:
                                operation_id = result["request_id"]
                                method = result["method"]
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
                                elif method == "input.select":
                                    value = {"type": "xkb", "id": "us"}
                                elif method == "window.minimize":
                                    value = {"id": "42"}
                                elif method == "workspace.switch":
                                    value = {"id": "codex-probe", "active": True}
                                elif method == "workspace.rename":
                                    value = {"id": "codex-probe", "name": "Renamed"}
                                elif method == "workspace.move_window":
                                    value = {"workspace": "codex-probe", "window": "42"}
                                elif method == "workspace.remove":
                                    value = {"id": "codex-probe"}
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
        assert json.loads(monitor_result.stdout) == {
            "monitors": [{"id": "HDMI-1", "index": 0, "primary": True, "revision": 5}]
        }
        window_list = run(binary, "--socket", socket_path, "--format", "table", "window", "list")
        assert window_list.returncode == 0, window_list.stderr
        assert "APP ID" in window_list.stdout, window_list.stdout
        assert "org.example.Editor" in window_list.stdout
        window_list_json = run(binary, "--socket", socket_path, "--format", "json", "window", "list")
        assert window_list_json.returncode == 0, window_list_json.stderr
        listed_windows = json.loads(window_list_json.stdout)
        assert listed_windows["windows"][0]["id"] == "42"
        assert listed_windows["windows"][0]["workspace_id"] == "workspace-1"
        assert listed_windows["windows"][0]["revision"] == 5
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
        assert json.loads(workspace_list.stdout) == {
            "workspaces": [
                {
                    "id": "codex-probe",
                    "number": 1,
                    "name": "Codex Probe",
                    "active": True,
                    "windows": 2,
                    "revision": 5,
                }
            ]
        }
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
        move_active_monitor = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "window",
            "monitor",
            "active",
            "0",
        )
        assert move_active_monitor.returncode == 0, move_active_monitor.stderr
        assert json.loads(move_active_monitor.stdout) == {"id": "42", "monitor_id": "HDMI-1"}
        restore_or_minimize = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "window",
            "restore-or-minimize",
            "42",
        )
        assert restore_or_minimize.returncode == 0, restore_or_minimize.stderr
        assert json.loads(restore_or_minimize.stdout) == {"id": "42", "action": "restore"}
        layer_list = run(binary, "--socket", socket_path, "--format", "json", "layer", "list")
        assert layer_list.returncode == 0, layer_list.stderr
        assert json.loads(layer_list.stdout) == {
            "layers": [
                {
                    "id": "surface-1",
                    "title": "Panel",
                    "namespace": "panel:top",
                    "layer": "top",
                    "monitor_id": "HDMI-1",
                    "revision": 5,
                }
            ]
        }
        input_list = run(binary, "--socket", socket_path, "--format", "json", "input", "list")
        assert input_list.returncode == 0, input_list.stderr
        assert json.loads(input_list.stdout) == {
            "sources": [{"type": "xkb", "id": "us", "short_name": "en", "name": "English (US)"}],
            "revision": 5,
        }
        input_current = run(binary, "--socket", socket_path, "--format", "json", "input", "current")
        assert input_current.returncode == 0, input_current.stderr
        assert json.loads(input_current.stdout) == {
            "available": True,
            "source": {"type": "xkb", "id": "us", "short_name": "en", "name": "English (US)"},
            "revision": 5,
        }
        input_select = run(binary, "--socket", socket_path, "--format", "json", "input", "select", "xkb", "us")
        assert input_select.returncode == 0, input_select.stderr
        assert json.loads(input_select.stdout) == {"type": "xkb", "id": "us"}
        shortcut_list = run(binary, "--socket", socket_path, "--format", "json", "shortcut", "list")
        assert shortcut_list.returncode == 0, shortcut_list.stderr
        assert json.loads(shortcut_list.stdout) == [
            {
                "name": "test.shortcut",
                "binding": "<Super>space",
                "enabled": True,
                "trigger": "press",
                "action": "test.action",
                "revision": 6,
            }
        ]
        shortcut_actions = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "shortcut",
            "actions",
            "wm",
        )
        assert shortcut_actions.returncode == 0, shortcut_actions.stderr
        assert json.loads(shortcut_actions.stdout) == [
            {
                "id": "wm.close",
                "group": "wm",
                "key": "close",
                "default_bindings": ["<Alt>F4"],
            }
        ]
        permissions_list = run(binary, "--socket", socket_path, "--format", "json", "permissions", "list")
        assert permissions_list.returncode == 0, permissions_list.stderr
        assert json.loads(permissions_list.stdout) == {
            "policy": {"default": "deny", "rules": []},
            "capabilities": ["screen-cast", "remote-desktop"],
            "levels": ["deny", "ask", "allow"],
            "path": "/tmp/gnoblin-permissions.json",
        }
        permissions_check = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "permissions",
            "check",
            "remote-desktop",
            "app-id:org.example.Remote",
        )
        assert permissions_check.returncode == 0, permissions_check.stderr
        assert json.loads(permissions_check.stdout) == {
            "level": "allow",
            "rule": "remote-test",
            "monitors": [],
            "devices": ["keyboard"],
            "clipboard": True,
            "revision": 9,
        }
        permissions_policy = run(binary, "--socket", socket_path, "--format", "json", "permissions", "policy")
        assert permissions_policy.returncode == 0, permissions_policy.stderr
        assert json.loads(permissions_policy.stdout) == {
            "default": "default",
            "rules": [],
            "revision": 9,
        }
        input_devices = run(binary, "--socket", socket_path, "--format", "json", "input", "devices")
        assert input_devices.returncode == 0, input_devices.stderr
        assert json.loads(input_devices.stdout) == {
            "devices": [{"id": "input:1", "name": "Test keyboard"}],
            "revision": 11,
        }
        privacy = run(binary, "--socket", socket_path, "--format", "json", "privacy")
        assert privacy.returncode == 0, privacy.stderr
        assert privacy.stdout, (privacy.returncode, privacy.stderr)
        assert json.loads(privacy.stdout) == {
            "available": {
                "screen_sharing": True,
                "recording": True,
                "microphone_in_use": False,
                "camera_in_use": False,
                "location_in_use": False,
            },
            "screen_sharing": True,
            "recording": False,
            "revision": 42,
        }
        lua_file = Path(temporary) / "inspect.lua"
        lua_file.write_text(
            "local windows = gnoblin.windows.list { focused = true }\n"
            'assert(#windows == 1 and windows[1].id == "42")\n'
            "local window = windows[1]\n"
            'assert(window.title == "Notes")\n'
            'assert(not pcall(function() window.title = "changed" end))\n'
            'assert(gnoblin.windows.focused().id == "42")\n'
            'assert(gnoblin.windows.by_id("42").title == "Notes")\n'
            'assert(gnoblin.windows.by_id("missing") == nil)\n'
            'assert(window:minimize().id == "42")\n'
            "local focus_ok, focus_error = pcall(function() window:focus() end)\n"
            'assert(not focus_ok and focus_error:match("FocusContext"))\n'
            "local workspaces = gnoblin.workspaces.list()\n"
            'assert(#workspaces == 1 and workspaces[1].id == "codex-probe")\n'
            "local workspace = workspaces[1]\n"
            'assert(workspace.name == "Codex Probe")\n'
            'assert(not pcall(function() workspace.name = "changed" end))\n'
            'assert(gnoblin.workspaces.active().id == "codex-probe")\n'
            'assert(gnoblin.workspaces.by_id("codex-probe").number == 1)\n'
            'assert(gnoblin.workspaces.by_id("missing") == nil)\n'
            "assert(workspace:activate().active)\n"
            'assert(workspace:rename("Renamed").name == "Renamed")\n'
            'assert(workspace:move_here(window, {follow = true}).window == "42")\n'
            'assert(workspace:remove().id == "codex-probe")\n'
        )
        lua_api = run(binary, "--socket", socket_path, "lua", str(lua_file))
        assert lua_api.returncode == 0, lua_api.stderr
        unminimize = run(
            binary,
            "--socket",
            socket_path,
            "--format",
            "json",
            "window",
            "unminimize",
            "42",
        )
        assert unminimize.returncode == 0, unminimize.stderr
        assert json.loads(unminimize.stdout) == {"id": "42"}
        server_thread.join(timeout=5)
        assert not server_thread.is_alive(), "mock compositor did not finish CLI requests"
        assert not server_error, repr(server_error)
        assert len(received) == 40
        assert len(subscriptions) == 40
        for subscription in subscriptions:
            assert subscription["op"] == "events"
            assert subscription["api_version"] == {"major": 1, "minor": 11}
            assert subscription["events"] == [
                "gnoblin.operation.completed",
                "gnoblin.api.operation-completed",
            ]
        request = received[0]
        assert request["op"] == "api"
        assert request["method"] == "session.status"
        assert request["api_version"] == {"major": 1, "minor": 29}
        assert request["arguments"] == {}
        monitor_request = received[1]
        assert monitor_request["op"] == "api"
        assert monitor_request["method"] == "monitors.list"
        assert monitor_request["api_version"] == {"major": 1, "minor": 37}
        assert monitor_request["arguments"] == {}
        window_request = received[2]
        assert window_request["method"] == "windows.list"
        assert window_request["api_version"] == {"major": 1, "minor": 37}
        assert window_request["arguments"] == {}
        window_list_json_request = received[3]
        assert window_list_json_request["method"] == "windows.list"
        assert window_list_json_request["arguments"] == {}
        get_request = received[4]
        assert get_request["method"] == "animation.get"
        assert get_request["api_version"] == {"major": 1, "minor": 18}
        assert get_request["arguments"] == {"name": "missing"}
        preview_request = received[5]
        assert preview_request["method"] == "animation.preview"
        assert preview_request["arguments"] == {
            "name": "gnoblin-layer-open",
            "target_type": "namespace",
            "target": "panel:test",
            "autoplay": False,
        }
        create_request = received[6]
        assert create_request["method"] == "workspace.create"
        assert create_request["arguments"] == {
            "id": "codex-probe",
            "name": "Codex Probe",
            "activate": False,
        }
        list_request = received[7]
        assert list_request["method"] == "workspaces.list"
        assert list_request["api_version"] == {"major": 1, "minor": 37}
        assert list_request["arguments"] == {}
        match_request = received[8]
        assert match_request["method"] == "windows.list"
        assert match_request["api_version"] == {"major": 1, "minor": 37}
        assert match_request["arguments"] == {}
        thumbnail_request = received[9]
        assert thumbnail_request["method"] == "window.thumbnail"
        assert thumbnail_request["api_version"] == {"major": 1, "minor": 23}
        assert thumbnail_request["arguments"] == {"id": "42", "width": 64, "height": 64}
        launch_status_request = received[10]
        assert launch_status_request["method"] == "launches.snapshot"
        assert launch_status_request["api_version"] == {"major": 1, "minor": 39}
        assert launch_status_request["arguments"] == {}
        assert received[11]["method"] == "windows.list"
        assert received[11]["api_version"] == {"major": 1, "minor": 37}
        assert received[11]["arguments"] == {"focused": True}
        assert received[12]["method"] == "monitors.list"
        assert received[12]["api_version"] == {"major": 1, "minor": 37}
        assert received[12]["arguments"] == {}
        assert received[13]["method"] == "window.move_to_monitor"
        assert received[13]["arguments"] == {"id": "42", "monitor": "HDMI-1"}
        assert received[14]["method"] == "window.restore_or_minimize"
        assert received[14]["api_version"] == {"major": 1, "minor": 48}
        assert received[14]["arguments"] == {"id": "42"}
        assert received[15]["method"] == "layers.list"
        assert received[15]["api_version"] == {"major": 1, "minor": 37}
        assert received[15]["arguments"] == {}
        assert received[16]["method"] == "input.sources"
        assert received[16]["api_version"] == {"major": 1, "minor": 46}
        assert received[16]["arguments"] == {}
        assert received[17]["method"] == "input.current_source"
        assert received[17]["api_version"] == {"major": 1, "minor": 46}
        assert received[17]["arguments"] == {}
        assert received[18]["method"] == "input.select"
        assert received[18]["api_version"] == {"major": 1, "minor": 6}
        assert received[18]["arguments"] == {"type": "xkb", "id": "us"}
        assert received[19]["method"] == "shortcuts.list"
        assert received[19]["api_version"] == {"major": 1, "minor": 40}
        assert received[19]["arguments"] == {}
        assert received[20]["method"] == "shortcuts.actions"
        assert received[20]["api_version"] == {"major": 1, "minor": 41}
        assert received[20]["arguments"] == {"group": "wm"}
        assert received[21]["method"] == "permissions.list"
        assert received[21]["api_version"] == {"major": 1, "minor": 42}
        assert received[21]["arguments"] == {}
        assert received[22]["method"] == "permissions.check"
        assert received[22]["api_version"] == {"major": 1, "minor": 43}
        assert received[22]["arguments"] == {
            "capability": "remote-desktop",
            "identity": "app-id:org.example.Remote",
        }
        assert received[23]["method"] == "permissions.policy"
        assert received[23]["api_version"] == {"major": 1, "minor": 44}
        assert received[23]["arguments"] == {}
        assert received[24]["method"] == "input.devices"
        assert received[24]["api_version"] == {"major": 1, "minor": 46}
        assert received[24]["arguments"] == {}
        assert received[25]["method"] == "privacy.state"
        assert received[25]["api_version"] == {"major": 1, "minor": 47}
        assert received[25]["arguments"] == {}
        lua_request = received[26]
        assert lua_request["op"] == "api"
        assert lua_request["method"] == "windows.list"
        assert lua_request["arguments"] == {"focused": True}
        assert "source" not in lua_request and "code" not in lua_request
        assert received[27]["method"] == "windows.list"
        assert received[27]["arguments"] == {"focused": True}
        assert received[28]["method"] == "windows.list"
        assert received[28]["arguments"] == {}
        assert received[29]["method"] == "windows.list"
        assert received[29]["arguments"] == {}
        assert received[30]["method"] == "window.minimize"
        assert received[30]["api_version"] == {"major": 1, "minor": 64}
        assert received[30]["arguments"] == {"id": "42"}
        assert received[31]["method"] == "workspaces.list"
        assert received[31]["arguments"] == {}
        assert received[32]["method"] == "workspaces.list"
        assert received[32]["arguments"] == {}
        assert received[33]["method"] == "workspaces.list"
        assert received[33]["arguments"] == {}
        assert received[34]["method"] == "workspaces.list"
        assert received[34]["arguments"] == {}
        assert received[35]["method"] == "workspace.switch"
        assert received[35]["api_version"] == {"major": 1, "minor": 64}
        assert received[35]["arguments"] == {"id": "codex-probe"}
        assert received[36]["method"] == "workspace.rename"
        assert received[36]["arguments"] == {"id": "codex-probe", "name": "Renamed"}
        assert received[37]["method"] == "workspace.move_window"
        assert received[37]["arguments"] == {
            "window": "42",
            "workspace": {"id": "codex-probe"},
            "follow": True,
        }
        assert received[38]["method"] == "workspace.remove"
        assert received[38]["arguments"] == {"id": "codex-probe"}
        unminimize_request = received[39]
        assert unminimize_request["method"] == "window.unminimize"
        assert unminimize_request["arguments"] == {"id": "42"}

    print("compiled gnoblinctl CLI smoke checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
