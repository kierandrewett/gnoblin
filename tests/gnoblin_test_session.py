"""Shared controls for isolated, private Gnoblin integration sessions."""

from __future__ import annotations

import ast
import json
from pathlib import Path
import select
import subprocess
import time


def eval_shell(code: str, timeout: float = 5) -> object:
    result = subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnome.Shell",
            "--object-path",
            "/org/gnome/Shell",
            "--method",
            "org.gnome.Shell.Eval",
            code,
        ],
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if result.returncode:
        raise RuntimeError(f"Shell Eval failed: {result.stderr.strip() or result.stdout.strip()}")
    try:
        ok, value = ast.literal_eval(
            result.stdout.strip().replace("(true,", "(True,", 1).replace("(false,", "(False,", 1)
        )
    except (SyntaxError, ValueError) as error:
        raise RuntimeError(f"cannot parse Shell Eval reply: {result.stdout.strip()}") from error
    if not ok:
        raise RuntimeError(f"Shell Eval rejected expression: {value}")
    return json.loads(value) if value else None


def window_by_title_expression(title: str) -> str:
    return f"global.get_window_actors().find(a=>a.meta_window.title==={json.dumps(title)})?.meta_window"


def window_by_sequence_expression(sequence: int) -> str:
    return f"global.get_window_actors().find(a=>a.meta_window.get_stable_sequence()==={sequence})?.meta_window"


def shell_window(title: str) -> dict | None:
    return eval_shell(
        f"(()=>{{const a=global.get_window_actors().find(a=>a.meta_window.title==={json.dumps(title)});"
        "if(!a)return null;const w=a.meta_window,r=w.get_frame_rect();"
        "return {x:r.x,y:r.y,width:r.width,height:r.height,minimized:w.minimized,mapped:a.is_mapped(),"
        "fullscreen:w.fullscreen,maximized:w.maximized_horizontally&&w.maximized_vertically,"
        "layout:imports.gi.Meta.gnoblin_window_frame_get(w).recursiveUnpack()};})()"
    )


def shell_windows() -> list[dict]:
    result = eval_shell(
        "(()=>global.get_window_actors().map(a=>{const w=a.meta_window,r=w.get_frame_rect();"
        "return {sequence:w.get_stable_sequence(),title:w.get_title(),wm_class:w.get_wm_class(),"
        "pid:w.get_pid(),type:w.get_window_type(),x:r.x,y:r.y,width:r.width,height:r.height,"
        "minimized:w.minimized,fullscreen:w.fullscreen,"
        "maximized:w.maximized_horizontally&&w.maximized_vertically,"
        "can_move:w.allows_move(),can_resize:w.allows_resize(),"
        "can_maximize:w.can_maximize(),can_minimize:w.can_minimize()};}))()"
    )
    return result or []


def wait_for(predicate, description: str, timeout: float = 5) -> object:
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.04)
    raise TimeoutError(f"timed out waiting for {description}; last state={last!r}")


def send_pointer(kind: str, x: int, y: int, button: int = 1) -> None:
    pressed = f"p.notify_button(G.get_monotonic_time(),{button},C.ButtonState.PRESSED);"
    released = f"p.notify_button(G.get_monotonic_time(),{button},C.ButtonState.RELEASED);"
    if kind == "click":
        event = pressed + released
    elif kind == "press":
        event = pressed
    elif kind == "release":
        event = released
    elif kind == "move":
        event = ""
    else:
        raise ValueError(f"unsupported pointer action: {kind}")
    eval_shell(
        "(()=>{const C=imports.gi.Clutter,G=imports.gi.GLib;"
        "global.lifecycleFuzzPointer??=global.stage.context.get_backend().get_default_seat()"
        ".create_virtual_device(C.InputDeviceType.POINTER_DEVICE);"
        "const p=global.lifecycleFuzzPointer;p.notify_absolute_motion(G.get_monotonic_time(),"
        f"{x},{y});{event}return true;}})()"
    )


def compile_minimal_testing_shell(build_dir: Path) -> Path:
    """Build the real layer-shell panel shared by headless compositor tests."""
    root = Path(__file__).resolve().parents[1]
    build_dir.mkdir(parents=True, exist_ok=True)
    generated = build_dir / "generated"
    generated.mkdir(exist_ok=True)
    layer_xml = root / "src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
    layer_header = generated / "wlr-layer-shell-unstable-v1-client-protocol.h"
    layer_code = generated / "wlr-layer-shell-unstable-v1-protocol.c"
    xdg_header = generated / "xdg-shell-client-protocol.h"
    xdg_code = generated / "xdg-shell-protocol.c"
    wayland_protocols = subprocess.check_output(
        ["pkg-config", "--variable=pkgdatadir", "wayland-protocols"], text=True
    ).strip()
    commands = [
        ["wayland-scanner", "client-header", str(layer_xml), str(layer_header)],
        ["wayland-scanner", "private-code", str(layer_xml), str(layer_code)],
        [
            "wayland-scanner",
            "client-header",
            f"{wayland_protocols}/stable/xdg-shell/xdg-shell.xml",
            str(xdg_header),
        ],
        [
            "wayland-scanner",
            "private-code",
            f"{wayland_protocols}/stable/xdg-shell/xdg-shell.xml",
            str(xdg_code),
        ],
    ]
    for command in commands:
        subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
    compiler_flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "wayland-client"], text=True).split()
    binary = build_dir / "minimal-testing-shell"
    subprocess.run(
        [
            "cc",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            f"-I{generated}",
            str(root / "tests/e2e/minimal-testing-shell.c"),
            str(layer_code),
            str(xdg_code),
            *compiler_flags,
            "-o",
            str(binary),
        ],
        check=True,
    )
    return binary


def start_minimal_testing_shell(build_dir: Path, log_path: Path) -> subprocess.Popen:
    """Start the panel and wait for its first layer-surface commit."""
    binary = compile_minimal_testing_shell(build_dir)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("w") as log:
        process = subprocess.Popen([str(binary)], stdout=subprocess.PIPE, stderr=log, text=True, bufsize=1)
    assert process.stdout is not None
    readable, _, _ = select.select([process.stdout], [], [], 12)
    line = process.stdout.readline().strip() if readable else ""
    if line == "GNOBLIN_TEST_SHELL_READY":
        return process
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=2)
    raise RuntimeError(f"minimal layer-shell panel did not map: {line!r} {log_path.read_text()}")


def set_frame(title: str, policy: list[int]) -> None:
    window = window_by_title_expression(title)
    eval_shell(
        f"(()=>{{const w={window};if(!w)throw new Error('test target disappeared');"
        "return imports.gi.Meta.gnoblin_window_frame_set(w,"
        f"new imports.gi.GLib.Variant('(iiiiiiiii)',{json.dumps(policy)}));}})()"
    )
