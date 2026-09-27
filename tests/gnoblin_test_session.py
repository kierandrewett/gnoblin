"""Shared controls for isolated, private Gnoblin integration sessions."""

from __future__ import annotations

import ast
import json
from pathlib import Path
import select
import subprocess
import time

FRAME_ACTION_CLOSE = 2


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
        "if(!a)return null;const w=a.meta_window,r=w.get_frame_rect(),p=a.get_transformed_position();"
        "return {x:r.x,y:r.y,width:r.width,height:r.height,minimized:w.minimized,mapped:a.is_mapped(),"
        "actor_position:[Math.round(p[0]),Math.round(p[1])],"
        "ready:w.is_ready(),fullscreen:w.fullscreen,maximized:!!w.get_maximize_flags(),"
        "layout:imports.gi.Meta.gnoblin_window_frame_get(w).recursiveUnpack()};})()"
    )


def shell_windows() -> list[dict]:
    result = eval_shell(
        "(()=>global.get_window_actors().map(a=>{const w=a.meta_window,r=w.get_frame_rect();"
        "return {sequence:w.get_stable_sequence(),title:w.get_title(),wm_class:w.get_wm_class(),"
        "pid:w.get_pid(),type:w.get_window_type(),x:r.x,y:r.y,width:r.width,height:r.height,"
        "ready:w.is_ready(),mapped:a.is_mapped(),focused:global.display.focus_window===w,"
        "minimized:w.minimized,fullscreen:w.fullscreen,"
        "maximized:!!w.get_maximize_flags(),"
        "can_move:w.allows_move(),can_resize:w.allows_resize(),"
        "can_maximize:w.can_maximize(),can_minimize:w.can_minimize()};}))()"
    )
    return result or []


def application_window_candidates(
    windows: list[dict], baseline: set[int], splashscreen_type: int, modal_dialog_type: int
) -> list[dict]:
    """Return mapped app toplevels, testing modal dialogs before their parents."""
    candidates = [
        window
        for window in windows
        if window["sequence"] not in baseline
        and window["title"]
        and window["type"] != splashscreen_type
        and window["ready"]
        and window["mapped"]
    ]
    return sorted(
        candidates,
        key=lambda window: (
            window["type"] != modal_dialog_type,
            not window.get("focused", False),
            window["sequence"],
        ),
    )


def gnoblin_frame_visible(state: dict) -> bool:
    """Report a native frame from its presentation state, not its border width."""
    layout = state.get("layout", {})
    presentation = layout.get("presentation", {})
    return bool(layout.get("native") and presentation.get("visible"))


def constrain_move_to_monitor(state: dict, x: int, y: int) -> tuple[int, int]:
    """Keep as much of an E2E move target as possible within the current monitor."""
    monitor = state["monitor_rect"]
    left = monitor["x"]
    top = monitor["y"]
    right = max(left, left + monitor["width"] - state["width"])
    bottom = max(top, top + monitor["height"] - state["height"])
    return max(left, min(x, right)), max(top, min(y, bottom))


def frame_button_center(state: dict, action: int) -> tuple[int, int]:
    """Return a native-frame button center in stage coordinates."""
    regions = state["layout"]["presentation"]["regions"]
    region = next((item for item in regions if item[0] == action), None)
    if region is None:
        raise RuntimeError(f"frame button action {action} has no input region")
    # Mutter's frame rectangle can differ from the presented window actor after
    # state transitions (for example, unfullscreening at the work-area origin).
    # Regions are local to the frame actor, so use its transformed stage origin.
    origin_x, origin_y = state.get("actor_position", (state["x"], state["y"]))
    return (
        origin_x + region[1] + region[3] // 2,
        origin_y + region[2] + region[4] // 2,
    )


def close_target_state_ready(state: dict | None) -> bool:
    """Reject stale fullscreen geometry until a requested native frame returns."""
    if state is None or state.get("fullscreen"):
        return False
    layout = state.get("layout") or {}
    if not layout.get("supported") or not layout.get("native"):
        return True
    presentation = layout.get("presentation") or {}
    return bool(
        presentation.get("visible")
        and any(
            region[0] == FRAME_ACTION_CLOSE and region[3] > 0 and region[4] > 0
            for region in presentation.get("regions", [])
            if len(region) == 5
        )
    )


def wait_for_settled_close_target(get_state, stable_seconds: float = 0.15, timeout: float = 4) -> dict | None:
    """Wait for nonfullscreen geometry and its close target to remain stable."""
    last_signature = None
    stable_since = None

    def settled_state() -> dict | None:
        nonlocal last_signature, stable_since
        state = get_state()
        now = time.monotonic()
        if not close_target_state_ready(state):
            last_signature = None
            stable_since = None
            return None

        layout = state.get("layout") or {}
        presentation = layout.get("presentation") or {}
        signature = (
            *(state.get(key) for key in ("sequence", "x", "y", "width", "height", "fullscreen")),
            tuple(state.get("actor_position") or ()),
            layout.get("supported"),
            layout.get("native"),
            layout.get("mode"),
            tuple(layout.get("border") or ()),
            presentation.get("visible"),
            tuple(tuple(region) for region in presentation.get("regions", [])),
            presentation.get("serial"),
        )
        if signature != last_signature:
            last_signature = signature
            stable_since = now
            return None
        if stable_since is not None and now - stable_since >= stable_seconds:
            return state
        return None

    return wait_for(settled_state, "settled window close target", timeout=timeout)


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
