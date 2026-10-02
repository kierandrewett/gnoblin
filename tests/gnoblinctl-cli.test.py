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
        monitor_request_count = 0
        status_request_count = 0

        def serve_once() -> None:
            nonlocal monitor_request_count, status_request_count
            try:
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                    server.bind(socket_path)
                    server.listen(9)
                    ready.set()
                    for _ in range(84):
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
                            if request["method"] == "version":
                                result = {
                                    "gnoblin": "0.2.0",
                                    "gnome": "51.0",
                                    "mutter": "51.0",
                                    "lua": "Lua 5.4",
                                    "api": "1.64",
                                    "git_remote": "https://example.invalid/gnoblin.git",
                                    "git_sha": "0123456789abcdef",
                                    "build_id": "20261002.1",
                                }
                            elif request["method"] == "session.status":
                                status_request_count += 1
                                result = {"state": "running", "lock_available": False}
                                if status_request_count > 2:
                                    result["lock_available"] = True
                                    result["lock_state"] = "covering"
                            elif request["method"] == "session.activity":
                                result = {
                                    "available": True,
                                    "idle": True,
                                    "threshold_ms": 300000,
                                    "idle_for_ms": 1000,
                                    "revision": 7,
                                }
                            elif request["method"] == "monitors.list":
                                monitor_request_count += 1
                                if monitor_request_count >= 6:
                                    result = []
                                elif monitor_request_count <= 2:
                                    result = [{"id": "HDMI-1", "index": 0, "primary": True, "revision": 5}]
                                else:
                                    result = [
                                        {
                                            "id": "HDMI-1",
                                            "index": 0,
                                            "name": "Test Display",
                                            "make": "Acme",
                                            "model": "Panel 1",
                                            "serial": "ABC123",
                                            "primary": True,
                                            "enabled": True,
                                            "x": 10,
                                            "y": 20,
                                            "width": 1920,
                                            "height": 1080,
                                            "scale": 1.5,
                                            "refresh_rate": 144.0,
                                            "transform": "normal",
                                            "revision": 5,
                                        }
                                    ]
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
                                        "frame": {"x": 12, "y": 24, "width": 800, "height": 600},
                                        "revision": 5,
                                    }
                                ]
                            elif request["method"] == "animation.surfaces":
                                result = {
                                    "surfaces": [
                                        {
                                            "id": "42",
                                            "namespace": "bingux-panel",
                                            "title": "Panel",
                                        }
                                    ]
                                }
                            elif request["method"] == "animation.list":
                                result = {
                                    "animations": [
                                        {
                                            "name": "fade",
                                            "enable": True,
                                            "event": "open",
                                            "duration": 150,
                                            "ease": "ease-out-expo",
                                            "builtin": True,
                                            "previewable": True,
                                            "from": {"opacity": 0.0},
                                            "to": {"opacity": 1.0},
                                            "origin": "center",
                                            "target": "none",
                                            "revision": 5,
                                        }
                                    ]
                                }
                            elif request["method"] == "animation.get":
                                result = (
                                    {
                                        "name": "fade",
                                        "enable": True,
                                        "event": "open",
                                        "duration": 150,
                                        "ease": "ease-out-expo",
                                        "builtin": True,
                                        "previewable": True,
                                        "from": {"opacity": 0.0},
                                        "to": {"opacity": 1.0},
                                        "origin": "center",
                                        "target": "none",
                                        "revision": 5,
                                    }
                                    if request["arguments"].get("name") == "fade"
                                    else None
                                )
                            elif request["method"] in {
                                "animation.preview",
                                "animation.seek",
                                "animation.step",
                                "animation.play",
                                "animation.pause",
                                "animation.stop",
                            }:
                                result = {"request_id": 17, "method": request["method"]}
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
                                        "keyboard_interactive": "on_demand",
                                        "exclusive_zone": 32,
                                        "anchor": ["top", "left", "right"],
                                        "geometry": {"x": 0, "y": 0, "width": 1920, "height": 32},
                                        "mapped": True,
                                        "revision": 5,
                                    }
                                ]
                            elif request["method"] == "input.sources":
                                result = {
                                    "sources": [
                                        {
                                            "type": "xkb",
                                            "id": "us",
                                            "short_name": "en",
                                            "name": "English (US)",
                                            "current": True,
                                        }
                                    ],
                                    "revision": 5,
                                }
                            elif request["method"] == "input.current_source":
                                result = {
                                    "available": True,
                                    "source": {
                                        "type": "xkb",
                                        "id": "us",
                                        "short_name": "en",
                                        "name": "English (US)",
                                        "current": True,
                                    },
                                    "revision": 5,
                                }
                            elif request["method"] == "input.devices":
                                result = {
                                    "devices": [
                                        {
                                            "id": "input:1",
                                            "name": "Test keyboard",
                                            "device_type": "keyboard",
                                            "capabilities": ["keyboard"],
                                        }
                                    ],
                                    "revision": 11,
                                }
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
                            elif request["method"] == "capabilities.list":
                                result = [
                                    {
                                        "id": "microphone-monitor",
                                        "description": "PipeWire microphone activity monitoring",
                                        "available": False,
                                        "reason": "pipewire_unavailable",
                                        "revision": 18,
                                    }
                                ]
                            elif request["method"] == "input.select":
                                result = {"request_id": 23, "method": "input.select"}
                            elif request["method"] == "window.thumbnail":
                                result = {"request_id": 20, "method": "window.thumbnail"}
                            elif request["method"] == "launches.snapshot":
                                result = {
                                    "launches": [
                                        {
                                            "token": "one",
                                            "application": "app",
                                            "started_at": 1720000000123,
                                            "timeout_ms": 3000,
                                            "state": "pending",
                                            "revision": 4,
                                        }
                                    ],
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
                            elif request["method"] == "focus.policy":
                                result = {
                                    "focus_mode": "sloppy",
                                    "focus_new_windows": "smart",
                                    "raise_on_click": False,
                                    "auto_raise": True,
                                    "focus_change_on_pointer_rest": True,
                                    "auto_raise_delay": 750,
                                    "revision": 42,
                                }
                            elif request["method"] == "settings":
                                result = {
                                    "revision": 43,
                                    "window_management": {"focus_mode": "sloppy"},
                                    "shortcuts": {"terminal": {"binding": "<Super>Return"}},
                                }
                            elif request["method"] == "focus.history":
                                result = [
                                    {
                                        "id": "42",
                                        "title": "Notes",
                                        "app_id": "org.example.Editor",
                                        "focused": True,
                                        "geometry": {"x": 8, "y": 12, "width": 640, "height": 480},
                                    }
                                ]
                            elif request["method"] == "layer.animation_policy":
                                result = {
                                    "namespace": "bingux-panel",
                                    "enter": {
                                        "animation": "fade",
                                        "duration": 240,
                                        "easing": {"type": "cubic-bezier", "x1": 0.2, "y1": 0.0, "x2": 0.0, "y2": 1.0},
                                    },
                                    "exit": {"animation": "slide"},
                                    "window_shadow": {"opacity": 0.4},
                                    "revision": 19,
                                }
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
                                result = {
                                    "default": "default",
                                    "rules": [
                                        {
                                            "name": "remote-example",
                                            "match": "^app%-id:org%.example%.Remote$",
                                            "capabilities": ["remote-desktop"],
                                            "level": "allow",
                                        }
                                    ],
                                    "revision": 9,
                                }
                            elif request["method"] == "portals.grants":
                                result = [
                                    {
                                        "id": "grant-17",
                                        "kind": "remote-desktop",
                                        "requester": "app-id:org.example.Remote",
                                        "devices": ["keyboard", "pointer"],
                                        "clipboard": True,
                                        "has_screen_streams": True,
                                        "created_at": 1720000000123,
                                        "revision": 12,
                                    }
                                ]
                            elif request["method"] == "grant.revoke":
                                result = {"request_id": 29, "method": "grant.revoke"}
                            elif request["method"] == "shortcut.capture":
                                result = {"request_id": 30, "method": "shortcut.capture"}
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
                                "animation.seek",
                                "animation.step",
                                "animation.play",
                                "animation.pause",
                                "animation.stop",
                                "workspace.create",
                                "window.thumbnail",
                                "input.select",
                                "window.minimize",
                                "workspace.switch",
                                "workspace.rename",
                                "workspace.move_window",
                                "workspace.remove",
                                "grant.revoke",
                                "shortcut.capture",
                            }:
                                operation_id = result["request_id"]
                                method = result["method"]
                                if method == "animation.preview":
                                    if request.get("arguments", {}).get("target_type") == "namespace":
                                        value = {"session": "preview-17"}
                                    else:
                                        value = {
                                            "id": "preview-17",
                                            "session": "preview-17",
                                            "name": "gnoblin-window-open",
                                            "event": "open",
                                            "target": "42",
                                            "target_type": "window",
                                            "progress": 0.0,
                                            "playing": False,
                                            "revision": 5,
                                        }
                                elif method == "animation.seek":
                                    value = {
                                        "id": "preview-17",
                                        "session": "preview-17",
                                        "name": "gnoblin-window-open",
                                        "event": "open",
                                        "target": "42",
                                        "target_type": "window",
                                        "progress": request["arguments"]["progress"],
                                        "playing": False,
                                        "revision": 5,
                                    }
                                elif method == "animation.step":
                                    value = {
                                        "id": "preview-17",
                                        "session": "preview-17",
                                        "name": "gnoblin-window-open",
                                        "event": "open",
                                        "target": "42",
                                        "target_type": "window",
                                        "progress": 0.75,
                                        "playing": False,
                                        "revision": 5,
                                    }
                                elif method == "animation.play":
                                    value = {
                                        "id": "preview-17",
                                        "session": "preview-17",
                                        "name": "gnoblin-window-open",
                                        "event": "open",
                                        "target": "42",
                                        "target_type": "window",
                                        "progress": 0.75,
                                        "playing": True,
                                        "revision": 5,
                                    }
                                elif method == "animation.pause":
                                    value = {
                                        "id": "preview-17",
                                        "session": "preview-17",
                                        "name": "gnoblin-window-open",
                                        "event": "open",
                                        "target": "42",
                                        "target_type": "window",
                                        "progress": 0.75,
                                        "playing": False,
                                        "revision": 5,
                                    }
                                elif method == "animation.stop":
                                    value = {"ok": True, "session": "preview-17"}
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
                                    value = {
                                        "type": "xkb",
                                        "id": "us",
                                        "short_name": "en",
                                        "name": "English (US)",
                                        "current": True,
                                        "revision": 5,
                                    }
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
                                elif method == "grant.revoke":
                                    value = {"ok": True, "id": request["arguments"]["id"]}
                                elif method == "shortcut.capture":
                                    value = {"accelerator": "<Super>Return"}
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
        assert json.loads(result.stdout) == {"state": "running", "lock_available": False}
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
            "launches": [
                {
                    "token": "one",
                    "application": "app",
                    "started_at": 1720000000123,
                    "timeout_ms": 3000,
                    "state": "pending",
                    "revision": 4,
                }
            ],
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
                    "keyboard_interactive": "on_demand",
                    "exclusive_zone": 32,
                    "anchor": ["top", "left", "right"],
                    "geometry": {"x": 0, "y": 0, "width": 1920, "height": 32},
                    "mapped": True,
                    "revision": 5,
                }
            ]
        }
        input_list = run(binary, "--socket", socket_path, "--format", "json", "input", "list")
        assert input_list.returncode == 0, input_list.stderr
        assert json.loads(input_list.stdout) == {
            "sources": [
                {
                    "type": "xkb",
                    "id": "us",
                    "short_name": "en",
                    "name": "English (US)",
                    "current": True,
                }
            ],
            "revision": 5,
        }
        input_current = run(binary, "--socket", socket_path, "--format", "json", "input", "current")
        assert input_current.returncode == 0, input_current.stderr
        assert json.loads(input_current.stdout) == {
            "available": True,
            "source": {
                "type": "xkb",
                "id": "us",
                "short_name": "en",
                "name": "English (US)",
                "current": True,
            },
            "revision": 5,
        }
        input_select = run(binary, "--socket", socket_path, "--format", "json", "input", "select", "xkb", "us")
        assert input_select.returncode == 0, input_select.stderr
        assert json.loads(input_select.stdout) == {
            "type": "xkb",
            "id": "us",
            "short_name": "en",
            "name": "English (US)",
            "current": True,
            "revision": 5,
        }
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
            "rules": [
                {
                    "name": "remote-example",
                    "match": "^app%-id:org%.example%.Remote$",
                    "capabilities": ["remote-desktop"],
                    "level": "allow",
                }
            ],
            "revision": 9,
        }
        input_devices = run(binary, "--socket", socket_path, "--format", "json", "input", "devices")
        assert input_devices.returncode == 0, input_devices.stderr
        assert json.loads(input_devices.stdout) == {
            "devices": [
                {
                    "id": "input:1",
                    "name": "Test keyboard",
                    "device_type": "keyboard",
                    "capabilities": ["keyboard"],
                }
            ],
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
            "assert(window.frame.x == 12 and window.frame.width == 800)\n"
            "assert(not pcall(function() window.frame.x = 99 end))\n"
            'assert(not pcall(function() rawset(window.frame, "x", 99) end))\n'
            "assert(window.frame.x == 12)\n"
            "local frame_fields = {}\n"
            "for key, value in pairs(window.frame) do frame_fields[key] = value end\n"
            "assert(frame_fields.x == 12 and frame_fields.height == 600)\n"
            "local window_fields = {}\n"
            "for key, value in pairs(window) do window_fields[key] = value end\n"
            'assert(window_fields.title == "Notes" and window_fields.frame.x == 12)\n'
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
            "local monitors = gnoblin.monitors.list()\n"
            'assert(#monitors == 1 and monitors[1].id == "HDMI-1")\n'
            "local monitor = monitors[1]\n"
            'assert(monitor.name == "Test Display" and monitor.make == "Acme")\n'
            'assert(monitor.model == "Panel 1" and monitor.serial == "ABC123")\n'
            "assert(monitor.primary and monitor.enabled)\n"
            "assert(monitor.x == 10 and monitor.y == 20)\n"
            "assert(monitor.width == 1920 and monitor.height == 1080)\n"
            "assert(monitor.scale == 1.5 and monitor.refresh_rate == 144.0)\n"
            'assert(monitor.transform == "normal" and monitor.revision == 5)\n'
            'assert(not pcall(function() monitor.id = "DP-1" end))\n'
            "assert(not pcall(function() monitor.missing = true end))\n"
            'assert(tostring(monitor) == "Monitor<HDMI-1>")\n'
            "local compatibility_monitors = gnoblin.monitor.list()\n"
            'assert(#compatibility_monitors == 1 and compatibility_monitors[1].id == "HDMI-1")\n'
            'assert(not pcall(function() compatibility_monitors[1].id = "DP-1" end))\n'
            'assert(gnoblin.monitors.primary().id == "HDMI-1")\n'
            "assert(gnoblin.monitors.primary() == nil)\n"
            "local layers = gnoblin.layers.list()\n"
            'assert(#layers == 1 and layers[1].id == "surface-1")\n'
            "local layer = layers[1]\n"
            'assert(layer.title == "Panel" and layer.namespace == "panel:top")\n'
            'assert(layer.layer == "top" and layer.monitor_id == "HDMI-1")\n'
            'assert(layer.keyboard_interactive == "on_demand" and layer.exclusive_zone == 32)\n'
            'assert(layer.anchor[1] == "top" and layer.anchor[2] == "left" and layer.anchor[3] == "right")\n'
            'assert(not pcall(function() layer.anchor[1] = "bottom" end))\n'
            'assert(not pcall(function() rawset(layer.anchor, 1, "bottom") end))\n'
            'assert(layer.anchor[1] == "top" and #layer.anchor == 3)\n'
            "local anchors = {}\n"
            "for index, anchor in ipairs(layer.anchor) do anchors[index] = anchor end\n"
            'assert(anchors[1] == "top" and anchors[2] == "left" and anchors[3] == "right")\n'
            "assert(layer.geometry.x == 0 and layer.geometry.y == 0)\n"
            "assert(layer.geometry.width == 1920 and layer.geometry.height == 32)\n"
            "assert(not pcall(function() layer.geometry.width = 1 end))\n"
            'assert(not pcall(function() rawset(layer.geometry, "width", 1) end))\n'
            "assert(layer.geometry.width == 1920)\n"
            "assert(layer.mapped and layer.revision == 5)\n"
            'assert(not pcall(function() layer.title = "changed" end))\n'
            "assert(not pcall(function() layer.missing = true end))\n"
            'assert(tostring(layer) == "LayerSurface<surface-1>")\n'
            'local filtered_layers = gnoblin.layers.list { monitor_id = "HDMI-1", namespace = "panel:top", layer = "top" }\n'
            'assert(#filtered_layers == 1 and filtered_layers[1].id == "surface-1")\n'
            'assert(not pcall(function() gnoblin.layers.list { app_id = "org.example.Panel" } end))\n'
            "assert(not pcall(function() gnoblin.layers.list { layer = 2 } end))\n"
            'assert(not pcall(function() gnoblin.layers.list { [1] = "top" } end))\n'
            "local compatibility_layers = gnoblin.layer.list()\n"
            'assert(#compatibility_layers == 1 and compatibility_layers[1].id == "surface-1")\n'
            'assert(not pcall(function() compatibility_layers[1].id = "changed" end))\n'
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
        animation_lua_file = Path(temporary) / "animation-preview.lua"
        animation_lua_file.write_text(
            'local preview = gnoblin.animations.preview { name = "gnoblin-window-open", '
            'event = "open", target_type = "window", target = "42", autoplay = false }\n'
            'assert(tostring(preview) == "AnimationPreview<preview-17>")\n'
            'assert(preview.id == "preview-17" and preview.name == "gnoblin-window-open")\n'
            'assert(preview.event == "open" and preview.target == "42" and preview.target_type == "window")\n'
            "assert(preview.progress == 0 and not preview.playing and preview.revision == 5)\n"
            "assert(preview.session == nil)\n"
            "assert(not pcall(function() preview.progress = 0.5 end))\n"
            "assert(not pcall(function() preview.missing = true end))\n"
            "local sought = preview:seek(0.5)\n"
            'assert(sought.progress == 0.5 and not sought.playing and type(sought.step) == "function")\n'
            "local stepped = sought:step(250)\n"
            'assert(stepped.progress == 0.75 and type(stepped.play) == "function")\n'
            "local playing = stepped:play()\n"
            'assert(playing.playing and type(playing.pause) == "function")\n'
            "local paused = playing:pause()\n"
            'assert(not paused.playing and type(paused.stop) == "function")\n'
            "local stopped = paused:stop()\n"
            'assert(stopped.ok and stopped.session == "preview-17")\n'
            "assert(not pcall(function() preview:seek(1.1) end))\n"
            "assert(not pcall(function() preview:step(0) end))\n"
            "assert(not pcall(function() preview:play(true) end))\n",
            encoding="utf-8",
        )
        animation_lua = run(binary, "--socket", socket_path, "lua", str(animation_lua_file))
        assert animation_lua.returncode == 0, animation_lua.stderr
        grants_lua_file = Path(temporary) / "portal-grants.lua"
        grants_lua_file.write_text(
            "local grants = gnoblin.portals.grants()\n"
            'assert(#grants == 1 and tostring(grants[1]) == "PortalGrant<grant-17>")\n'
            'assert(grants[1].kind == "remote-desktop" and grants[1].requester == "app-id:org.example.Remote")\n'
            'assert(grants[1].devices[1] == "keyboard" and grants[1].devices[2] == "pointer")\n'
            "assert(grants[1].clipboard and grants[1].has_screen_streams)\n"
            "assert(grants[1].created_at == 1720000000123 and grants[1].revision == 12)\n"
            'assert(not pcall(function() grants[1].kind = "screen-cast" end))\n'
            'assert(not pcall(function() grants[1].devices[1] = "touchscreen" end))\n'
            'assert(not pcall(function() rawset(grants[1].devices, 1, "touchscreen") end))\n'
            'assert(not pcall(function() gnoblin.portals.grants { requester = "invalid" } end))\n'
            'local grant = gnoblin.grant.list { kind = "remote-desktop" }[1]\n'
            'assert(grant.id == "grant-17" and type(grant.revoke) == "function")\n'
            "local revoked = grant:revoke()\n"
            'assert(revoked.ok and revoked.id == "grant-17")\n'
            "assert(not pcall(function() grant:revoke(true) end))\n",
            encoding="utf-8",
        )
        grants_lua = run(binary, "--socket", socket_path, "lua", str(grants_lua_file))
        assert grants_lua.returncode == 0, grants_lua.stderr
        input_lua_file = Path(temporary) / "input-snapshots.lua"
        input_lua_file.write_text(
            "local devices = gnoblin.input.devices()\n"
            'assert(#devices == 1 and tostring(devices[1]) == "InputDevice<input:1>")\n'
            'assert(devices[1].name == "Test keyboard" and devices[1].device_type == "keyboard")\n'
            'assert(devices[1].capabilities[1] == "keyboard" and devices[1].revision == 11)\n'
            'assert(not pcall(function() devices[1].name = "changed" end))\n'
            'assert(not pcall(function() devices[1].capabilities[1] = "pointer" end))\n'
            "local sources = gnoblin.input.sources()\n"
            'assert(#sources == 1 and tostring(sources[1]) == "InputSource<us>")\n'
            'assert(sources[1].type == "xkb" and sources[1].short_name == "en" and sources[1].current)\n'
            "assert(sources[1].revision == 5 and not pcall(function() sources[1].current = false end))\n"
            "local current = gnoblin.input.current_source()\n"
            'assert(current.id == "us" and current.current and current.revision == 5)\n'
            'local selected = gnoblin.input.select_source {type = "xkb", id = "us"}\n'
            'assert(selected.id == "us" and selected.current and selected.revision == 5)\n'
            'assert(not pcall(function() gnoblin.input.select_source {type = "xkb", id = "us", extra = true} end))\n',
            encoding="utf-8",
        )
        input_lua = run(binary, "--socket", socket_path, "lua", str(input_lua_file))
        assert input_lua.returncode == 0, input_lua.stderr
        shortcuts_lua_file = Path(temporary) / "shortcut-snapshots.lua"
        shortcuts_lua_file.write_text(
            "local shortcuts = gnoblin.shortcuts.list()\n"
            'assert(#shortcuts == 1 and tostring(shortcuts[1]) == "ShortcutState<test.shortcut>")\n'
            'assert(shortcuts[1].binding == "<Super>space" and shortcuts[1].enabled)\n'
            'assert(shortcuts[1].action == "test.action" and shortcuts[1].revision == 6)\n'
            "assert(not pcall(function() shortcuts[1].enabled = false end))\n"
            'local actions = gnoblin.shortcuts.actions("wm")\n'
            'assert(#actions == 1 and tostring(actions[1]) == "ShortcutAction<wm.close>")\n'
            'assert(actions[1].group == "wm" and actions[1].key == "close")\n'
            'assert(actions[1].default_bindings[1] == "<Alt>F4")\n'
            'assert(not pcall(function() actions[1].group = "wayland" end))\n'
            'assert(not pcall(function() actions[1].default_bindings[1] = "<Alt>Tab" end))\n'
            'assert(not pcall(function() rawset(actions[1].default_bindings, 1, "<Alt>Tab") end))\n'
            "assert(not pcall(function() gnoblin.shortcuts.list(true) end))\n"
            'assert(not pcall(function() gnoblin.shortcuts.actions({group = "wm"}) end))\n',
            encoding="utf-8",
        )
        shortcuts_lua = run(binary, "--socket", socket_path, "lua", str(shortcuts_lua_file))
        assert shortcuts_lua.returncode == 0, shortcuts_lua.stderr
        focus_policy_file = Path(temporary) / "focus-policy.lua"
        focus_policy_file.write_text(
            "local policy = gnoblin.focus.policy\n"
            'assert(tostring(policy) == "FocusPolicy")\n'
            'assert(policy.focus_mode == "sloppy" and policy.focus_new_windows == "smart")\n'
            "assert(not policy.raise_on_click and policy.auto_raise)\n"
            "assert(policy.focus_change_on_pointer_rest and policy.auto_raise_delay == 750)\n"
            "assert(policy.revision == 42)\n"
            'assert(not pcall(function() policy.focus_mode = "click" end))\n'
            'assert(not pcall(function() rawset(policy, "focus_mode", "click") end))\n'
            "assert(not pcall(function() policy() end))\n",
            encoding="utf-8",
        )
        focus_policy = run(binary, "--socket", socket_path, "lua", str(focus_policy_file))
        assert focus_policy.returncode == 0, focus_policy.stderr
        settings_file = Path(temporary) / "settings-property.lua"
        settings_file.write_text(
            "local settings = gnoblin.settings\n"
            'assert(tostring(settings) == "Settings" and settings.revision == 43)\n'
            'assert(settings.window_management.focus_mode == "sloppy")\n'
            'assert(settings.shortcuts.terminal.binding == "<Super>Return")\n'
            "assert(not pcall(function() settings.revision = 44 end))\n"
            'assert(not pcall(function() settings.window_management.focus_mode = "click" end))\n'
            'assert(not pcall(function() rawset(settings.shortcuts.terminal, "binding", "x") end))\n'
            "assert(not pcall(function() settings() end))\n",
            encoding="utf-8",
        )
        settings = run(binary, "--socket", socket_path, "lua", str(settings_file))
        assert settings.returncode == 0, settings.stderr
        focus_history_file = Path(temporary) / "focus-history.lua"
        focus_history_file.write_text(
            "local history = gnoblin.focus.history { limit = 1 }\n"
            'assert(#history == 1 and tostring(history[1]) == "Window<42>")\n'
            'assert(history[1].title == "Notes" and history[1].focused)\n'
            'assert(history[1].geometry.width == 640 and type(history[1].minimize) == "function")\n'
            'assert(not pcall(function() history[1].title = "Changed" end))\n'
            "assert(not pcall(function() history[1].geometry.width = 1 end))\n",
            encoding="utf-8",
        )
        focus_history = run(binary, "--socket", socket_path, "lua", str(focus_history_file))
        assert focus_history.returncode == 0, focus_history.stderr
        layer_policy_file = Path(temporary) / "layer-animation-policy.lua"
        layer_policy_file.write_text(
            'local policy = gnoblin.layers.animation_policy("bingux-panel")\n'
            'assert(tostring(policy) == "LayerAnimationPolicy<bingux-panel>")\n'
            'assert(policy.namespace == "bingux-panel" and policy.revision == 19)\n'
            'assert(policy.enter.animation == "fade" and policy.enter.duration == 240)\n'
            'assert(policy.enter.easing.type == "cubic-bezier" and policy.enter.easing.x2 == 0)\n'
            'assert(policy.exit.animation == "slide" and policy.window_shadow.opacity == 0.4)\n'
            'assert(not pcall(function() policy.enter.animation = "slide" end))\n'
            'assert(not pcall(function() rawset(policy.enter.easing, "x1", 0) end))\n'
            "assert(not pcall(function() policy.window_shadow.opacity = 0 end))\n"
            "assert(not pcall(function() gnoblin.layers.animation_policy() end))\n"
            "assert(not pcall(function() gnoblin.layers.animation_policy(2) end))\n"
            'assert(not pcall(function() gnoblin.layers.animation_policy(string.rep("x", 129)) end))\n',
            encoding="utf-8",
        )
        layer_policy = run(binary, "--socket", socket_path, "lua", str(layer_policy_file))
        assert layer_policy.returncode == 0, layer_policy.stderr
        privacy_file = Path(temporary) / "privacy-state.lua"
        privacy_file.write_text(
            "local state = gnoblin.privacy.state()\n"
            'assert(tostring(state) == "PrivacyState" and state.revision == 42)\n'
            "assert(state.available.screen_sharing and state.screen_sharing)\n"
            "assert(state.available.recording and not state.recording)\n"
            "assert(not pcall(function() state.revision = 1 end))\n"
            "assert(not pcall(function() state.available.screen_sharing = false end))\n"
            'assert(not pcall(function() rawset(state.available, "recording", false) end))\n'
            "assert(not pcall(function() gnoblin.privacy.state(true) end))\n",
            encoding="utf-8",
        )
        privacy = run(binary, "--socket", socket_path, "lua", str(privacy_file))
        assert privacy.returncode == 0, privacy.stderr
        capabilities_file = Path(temporary) / "capabilities.lua"
        capabilities_file.write_text(
            "local capabilities = gnoblin.capabilities.list()\n"
            'assert(#capabilities == 1 and tostring(capabilities[1]) == "Capability<microphone-monitor>")\n'
            'assert(capabilities[1].description == "PipeWire microphone activity monitoring")\n'
            'assert(not capabilities[1].available and capabilities[1].reason == "pipewire_unavailable")\n'
            "assert(capabilities[1].revision == 18)\n"
            "assert(not pcall(function() capabilities[1].available = true end))\n"
            'assert(not pcall(function() rawset(capabilities[1], "reason", "changed") end))\n'
            "assert(not pcall(function() gnoblin.capabilities.list(true) end))\n",
            encoding="utf-8",
        )
        capability_result = run(binary, "--socket", socket_path, "lua", str(capabilities_file))
        assert capability_result.returncode == 0, capability_result.stderr
        policy_file = Path(temporary) / "permission-policy.lua"
        policy_file.write_text(
            "local policy = gnoblin.permissions.policy()\n"
            'assert(tostring(policy) == "PermissionPolicy" and policy.default == "default")\n'
            "assert(policy.revision == 9 and #policy.rules == 1)\n"
            'assert(policy.rules[1].name == "remote-example" and policy.rules[1].level == "allow")\n'
            'assert(policy.rules[1].capabilities[1] == "remote-desktop")\n'
            'assert(not pcall(function() policy.default = "deny" end))\n'
            'assert(not pcall(function() policy.rules[1].level = "deny" end))\n'
            'assert(not pcall(function() rawset(policy.rules[1].capabilities, 1, "access") end))\n'
            "assert(not pcall(function() gnoblin.permissions.policy(true) end))\n",
            encoding="utf-8",
        )
        policy_result = run(binary, "--socket", socket_path, "lua", str(policy_file))
        assert policy_result.returncode == 0, policy_result.stderr
        session_status_file = Path(temporary) / "session-status.lua"
        session_status_file.write_text(
            "local unavailable = gnoblin.session.status()\n"
            'assert(tostring(unavailable) == "SessionStatus" and unavailable.state == "running")\n'
            "assert(not unavailable.lock_available and unavailable.lock_state == nil)\n"
            'assert(not pcall(function() unavailable.lock_state = "unlocked" end))\n'
            'assert(not pcall(function() rawset(unavailable, "lock_state", "unlocked") end))\n'
            "local available = gnoblin.session.status()\n"
            'assert(available.lock_available and available.lock_state == "covering")\n'
            'assert(not pcall(function() available.lock_state = "unlocked" end))\n'
            "assert(not pcall(function() gnoblin.session.status(true) end))\n",
            encoding="utf-8",
        )
        session_status_result = run(binary, "--socket", socket_path, "lua", str(session_status_file))
        assert session_status_result.returncode == 0, session_status_result.stderr
        session_activity_file = Path(temporary) / "session-activity.lua"
        session_activity_file.write_text(
            "local activity = gnoblin.session.activity()\n"
            'assert(tostring(activity) == "SessionActivity" and activity.available and activity.idle)\n'
            "assert(activity.threshold_ms == 300000 and activity.idle_for_ms >= 1000)\n"
            "assert(activity.revision == 7)\n"
            "assert(not pcall(function() activity.idle = false end))\n"
            'assert(not pcall(function() rawset(activity, "idle", false) end))\n'
            "assert(not pcall(function() gnoblin.session.activity(true) end))\n",
            encoding="utf-8",
        )
        session_activity_result = run(binary, "--socket", socket_path, "lua", str(session_activity_file))
        assert session_activity_result.returncode == 0, session_activity_result.stderr
        permission_decision_file = Path(temporary) / "permission-decision.lua"
        permission_decision_file.write_text(
            'local decision = gnoblin.permissions.check {capability = "remote-desktop", identity = "app-id:org.example.Remote"}\n'
            'assert(tostring(decision) == "PermissionDecision" and decision.level == "allow")\n'
            'assert(decision.rule == "remote-test" and decision.devices[1] == "keyboard")\n'
            "assert(decision.clipboard and decision.revision == 9)\n"
            'assert(not pcall(function() decision.level = "deny" end))\n'
            'assert(not pcall(function() rawset(decision.devices, 1, "pointer") end))\n'
            'local positional = gnoblin.permissions.check("remote-desktop", "app-id:org.example.Remote")\n'
            'assert(positional.level == "allow" and positional.revision == 9)\n'
            "assert(not pcall(function() gnoblin.permissions.check() end))\n"
            'assert(not pcall(function() gnoblin.permissions.check {capability = "remote-desktop", identity = "app-id:test", extra = true} end))\n',
            encoding="utf-8",
        )
        permission_decision_result = run(binary, "--socket", socket_path, "lua", str(permission_decision_file))
        assert permission_decision_result.returncode == 0, permission_decision_result.stderr
        animation_reads_file = Path(temporary) / "animation-reads.lua"
        animation_reads_file.write_text(
            "local animations = gnoblin.animations.list()\n"
            'assert(#animations == 1 and animations[1].name == "fade")\n'
            "assert(animations[1].from.opacity == 0 and animations[1].revision == 5)\n"
            'assert(not pcall(function() animations[1].name = "changed" end))\n'
            "assert(not pcall(function() animations[1].from.opacity = 0.5 end))\n"
            'assert(not pcall(function() rawset(animations[1].from, "opacity", 0.5) end))\n'
            'local fade = gnoblin.animations.get("fade")\n'
            'assert(fade.name == "fade" and fade.to.opacity == 1)\n'
            "assert(not pcall(function() fade.enable = false end))\n"
            'assert(gnoblin.animations.get("missing") == nil)\n'
            "assert(not pcall(function() gnoblin.animations.list(true) end))\n"
            "assert(not pcall(function() gnoblin.animations.get() end))\n"
            "assert(not pcall(function() gnoblin.animations.get(2) end))\n",
            encoding="utf-8",
        )
        animation_reads_result = run(binary, "--socket", socket_path, "lua", str(animation_reads_file))
        assert animation_reads_result.returncode == 0, animation_reads_result.stderr
        launches_file = Path(temporary) / "launches.lua"
        launches_file.write_text(
            "local launches = gnoblin.launches.list()\n"
            'assert(#launches == 1 and launches[1].token == "one")\n'
            'assert(launches[1].application == "app" and launches[1].state == "pending")\n'
            "assert(launches[1].started_at == 1720000000123 and launches[1].timeout_ms == 3000)\n"
            'assert(not pcall(function() launches[1].state = "ended" end))\n'
            "local snapshot = gnoblin.launches.snapshot()\n"
            'assert(snapshot.revision == 4 and snapshot.launches[1].token == "one")\n'
            "assert(not pcall(function() snapshot.revision = 5 end))\n"
            'assert(not pcall(function() rawset(snapshot.launches[1], "token", "changed") end))\n'
            "assert(not pcall(function() gnoblin.launches.list(true) end))\n"
            "assert(not pcall(function() gnoblin.launches.snapshot({}) end))\n",
            encoding="utf-8",
        )
        launches_result = run(binary, "--socket", socket_path, "lua", str(launches_file))
        assert launches_result.returncode == 0, launches_result.stderr
        permission_list_file = Path(temporary) / "permission-list.lua"
        permission_list_file.write_text(
            "local permissions = gnoblin.permissions.list()\n"
            'assert(permissions.policy.default == "deny")\n'
            'assert(permissions.capabilities[1] == "screen-cast" and permissions.levels[2] == "ask")\n'
            'assert(permissions.path == "/tmp/gnoblin-permissions.json")\n'
            'assert(not pcall(function() permissions.policy.default = "allow" end))\n'
            'assert(not pcall(function() rawset(permissions.capabilities, 1, "changed") end))\n'
            "assert(not pcall(function() gnoblin.permissions.list(true) end))\n",
            encoding="utf-8",
        )
        permission_list_result = run(binary, "--socket", socket_path, "lua", str(permission_list_file))
        assert permission_list_result.returncode == 0, permission_list_result.stderr
        version_file = Path(temporary) / "version.lua"
        version_file.write_text(
            "local version = gnoblin.version()\n"
            'assert(version.gnoblin == "0.2.0" and version.gnome == "51.0")\n'
            'assert(version.mutter == "51.0" and version.lua == "Lua 5.4" and version.api == "1.64")\n'
            'assert(version.git_remote == "https://example.invalid/gnoblin.git")\n'
            'assert(version.git_sha == "0123456789abcdef" and version.build_id == "20261002.1")\n'
            'assert(not pcall(function() version.git_sha = "changed" end))\n'
            "assert(not pcall(function() gnoblin.version(true) end))\n",
            encoding="utf-8",
        )
        version_result = run(binary, "--socket", socket_path, "lua", str(version_file))
        assert version_result.returncode == 0, version_result.stderr
        shortcut_capture_file = Path(temporary) / "shortcut-capture.lua"
        shortcut_capture_file.write_text(
            "local captured = gnoblin.shortcuts.capture()\n"
            'assert(captured.accelerator == "<Super>Return")\n'
            "local timed = gnoblin.shortcuts.capture {timeout = 10}\n"
            'assert(timed.accelerator == "<Super>Return")\n'
            "assert(not pcall(function() gnoblin.shortcuts.capture {timeout = 0} end))\n"
            "assert(not pcall(function() gnoblin.shortcuts.capture {timeout = 61} end))\n"
            "assert(not pcall(function() gnoblin.shortcuts.capture {timeout = 1.5} end))\n"
            "assert(not pcall(function() gnoblin.shortcuts.capture {timeout = true} end))\n"
            "assert(not pcall(function() gnoblin.shortcuts.capture {unexpected = true} end))\n"
            'assert(not pcall(function() gnoblin.shortcuts.capture("10") end))\n',
            encoding="utf-8",
        )
        shortcut_capture_result = run(binary, "--socket", socket_path, "lua", str(shortcut_capture_file))
        assert shortcut_capture_result.returncode == 0, shortcut_capture_result.stderr
        animation_surfaces_file = Path(temporary) / "animation-surfaces.lua"
        animation_surfaces_file.write_text(
            "local snapshot = gnoblin.animations.surfaces()\n"
            "local surface = snapshot.surfaces[1]\n"
            'assert(surface.id == "42" and surface.namespace == "bingux-panel" and surface.title == "Panel")\n'
            'assert(not pcall(function() surface.namespace = "changed" end))\n'
            'assert(not pcall(function() rawset(surface, "namespace", "changed") end))\n'
            "assert(not pcall(function() rawset(snapshot.surfaces, 1, {}) end))\n"
            "assert(not pcall(function() gnoblin.animations.surfaces(true) end))\n",
            encoding="utf-8",
        )
        animation_surfaces_result = run(binary, "--socket", socket_path, "lua", str(animation_surfaces_file))
        assert animation_surfaces_result.returncode == 0, animation_surfaces_result.stderr
        server_thread.join(timeout=5)
        assert not server_thread.is_alive(), "mock compositor did not finish CLI requests"
        assert not server_error, repr(server_error)
        assert len(received) == 84
        assert len(subscriptions) == 84
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
        animation_calls = received[47:53]
        assert [call["method"] for call in animation_calls] == [
            "animation.preview",
            "animation.seek",
            "animation.step",
            "animation.play",
            "animation.pause",
            "animation.stop",
        ]
        assert animation_calls[0]["arguments"] == {
            "name": "gnoblin-window-open",
            "event": "open",
            "target_type": "window",
            "target": "42",
            "autoplay": False,
        }
        assert animation_calls[1]["arguments"] == {"session": "preview-17", "progress": 0.5}
        assert animation_calls[2]["arguments"] == {"session": "preview-17", "milliseconds": 250}
        for call in animation_calls[3:]:
            assert call["arguments"] == {"session": "preview-17"}
        launch_calls = [call for call in received if call["method"] == "launches.snapshot"][-2:]
        assert [call["method"] for call in launch_calls] == [
            "launches.snapshot",
            "launches.snapshot",
        ]
        assert all(call["api_version"] == {"major": 1, "minor": 39} for call in launch_calls)
        permission_list_request = [call for call in received if call["method"] == "permissions.list"][-1]
        assert permission_list_request["api_version"] == {"major": 1, "minor": 42}
        assert permission_list_request["arguments"] == {}
        version_request = [call for call in received if call["method"] == "version"][-1]
        assert version_request["api_version"] == {"major": 1, "minor": 19}
        assert version_request["arguments"] == {}
        assert received[53]["method"] == "portals.grants"
        assert received[53]["api_version"] == {"major": 1, "minor": 45}
        assert received[53]["arguments"] == {}
        assert received[54]["method"] == "portals.grants"
        assert received[54]["api_version"] == {"major": 1, "minor": 45}
        assert received[54]["arguments"] == {"kind": "remote-desktop"}
        assert received[55]["method"] == "grant.revoke"
        assert received[55]["api_version"] == {"major": 1, "minor": 14}
        assert received[55]["arguments"] == {
            "id": "grant-17",
            "kind": "remote-desktop",
            "created_at": 1720000000123,
        }
        assert received[56]["method"] == "input.devices"
        assert received[56]["api_version"] == {"major": 1, "minor": 46}
        assert received[56]["arguments"] == {}
        assert received[57]["method"] == "input.sources"
        assert received[57]["api_version"] == {"major": 1, "minor": 46}
        assert received[57]["arguments"] == {}
        assert received[58]["method"] == "input.current_source"
        assert received[58]["api_version"] == {"major": 1, "minor": 46}
        assert received[58]["arguments"] == {}
        assert received[59]["method"] == "input.select"
        assert received[59]["api_version"] == {"major": 1, "minor": 6}
        assert received[59]["arguments"] == {"type": "xkb", "id": "us"}
        assert received[60]["method"] == "shortcuts.list"
        assert received[60]["api_version"] == {"major": 1, "minor": 40}
        assert received[60]["arguments"] == {}
        assert received[61]["method"] == "shortcuts.actions"
        assert received[61]["api_version"] == {"major": 1, "minor": 41}
        assert received[61]["arguments"] == {"group": "wm"}
        assert received[62]["method"] == "focus.policy"
        assert received[62]["api_version"] == {"major": 1, "minor": 19}
        assert received[62]["arguments"] == {}
        assert received[63]["method"] == "settings"
        assert received[63]["api_version"] == {"major": 1, "minor": 19}
        assert received[63]["arguments"] == {}
        assert received[64]["method"] == "focus.history"
        assert received[64]["api_version"] == {"major": 1, "minor": 19}
        assert received[64]["arguments"] == {"limit": 1}
        assert received[65]["method"] == "layer.animation_policy"
        assert received[65]["api_version"] == {"major": 1, "minor": 31}
        assert received[65]["arguments"] == {"namespace": "bingux-panel"}
        assert received[66]["method"] == "privacy.state"
        assert received[66]["api_version"] == {"major": 1, "minor": 47}
        assert received[66]["arguments"] == {}
        assert received[67]["method"] == "capabilities.list"
        assert received[67]["api_version"] == {"major": 1, "minor": 19}
        assert received[67]["arguments"] == {}
        assert received[68]["method"] == "permissions.policy"
        assert received[68]["api_version"] == {"major": 1, "minor": 44}
        assert received[68]["arguments"] == {}
        assert received[69]["method"] == "session.status"
        assert received[69]["api_version"] == {"major": 1, "minor": 29}
        assert received[69]["arguments"] == {}
        assert received[70]["method"] == "session.status"
        assert received[70]["api_version"] == {"major": 1, "minor": 29}
        assert received[70]["arguments"] == {}
        assert received[71]["method"] == "session.activity"
        assert received[71]["api_version"] == {"major": 1, "minor": 24}
        assert received[71]["arguments"] == {}
        assert received[72]["method"] == "permissions.check"
        assert received[72]["api_version"] == {"major": 1, "minor": 43}
        assert received[72]["arguments"] == {
            "capability": "remote-desktop",
            "identity": "app-id:org.example.Remote",
        }
        assert received[73]["method"] == "permissions.check"
        assert received[73]["api_version"] == {"major": 1, "minor": 43}
        assert received[73]["arguments"] == {
            "capability": "remote-desktop",
            "identity": "app-id:org.example.Remote",
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
        assert received[39]["method"] == "monitors.list"
        assert received[39]["api_version"] == {"major": 1, "minor": 37}
        assert received[39]["arguments"] == {}
        assert received[40]["method"] == "monitors.list"
        assert received[40]["arguments"] == {}
        assert received[41]["method"] == "monitors.list"
        assert received[41]["arguments"] == {}
        assert received[42]["method"] == "monitors.list"
        assert received[42]["arguments"] == {}
        assert received[43]["method"] == "layers.list"
        assert received[43]["api_version"] == {"major": 1, "minor": 37}
        assert received[43]["arguments"] == {}
        assert received[44]["method"] == "layers.list"
        assert received[44]["arguments"] == {
            "monitor_id": "HDMI-1",
            "namespace": "panel:top",
            "layer": "top",
        }
        assert received[45]["method"] == "layers.list"
        assert received[45]["arguments"] == {}
        unminimize_request = received[46]
        assert unminimize_request["method"] == "window.unminimize"
        assert unminimize_request["arguments"] == {"id": "42"}
        capture_calls = [call for call in received if call["method"] == "shortcut.capture"]
        assert len(capture_calls) == 2
        assert all(call["api_version"] == {"major": 1, "minor": 8} for call in capture_calls)
        assert capture_calls[0]["arguments"] == {}
        assert capture_calls[1]["arguments"] == {"timeout": 10}
        surfaces_calls = [call for call in received if call["method"] == "animation.surfaces"]
        assert len(surfaces_calls) == 1
        assert surfaces_calls[0]["api_version"] == {"major": 1, "minor": 18}
        assert surfaces_calls[0]["arguments"] == {}

    print("compiled gnoblinctl CLI smoke checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
