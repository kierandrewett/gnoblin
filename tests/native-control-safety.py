#!/usr/bin/env python3
"""Run the native control safety contracts as behaviour, without a compositor.

Two contracts, each with a real code path behind it:

* After a Lua runtime abort, the native control is freed only after compositor teardown has
  finished and the last async callback has returned (native_control_maybe_free_stopped,
  native_control_finish_teardown, native_runtime_abort, power_finish).
* Ending a shortcut session leaves the binding with its owner, registered and armed, so the
  next activation reaches the same owner (dynamic_shortcut_end_session, shortcut.session.end for
  Lua and socket owners, dispatch_dynamic_shortcut_activated).

The production functions and structs are extracted verbatim from src/ and compiled with real
GLib and JSON-GLib; the Mutter, Clutter and Lua-runtime boundaries are stubs. A production edit
that renames a field or a callee the extracted code uses fails to compile here.

Run: python3 tests/native-control-safety.py [BUILD_DIR]
Keep the binary: pass BUILD_DIR. Sanitizers (ASan and UBSan) are used when the toolchain links
them, so a use after free in the teardown path aborts the test.
"""

import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CONTROL = ROOT / "src/control"
INPUT = ROOT / "src/input"


def function(source, signature):
    match = re.search(re.escape(signature) + r"[^;{]*\{", source)
    if not match:
        raise ValueError(f"Missing definition: {signature}")
    depth = 1
    for end in range(match.end(), len(source)):
        depth += (source[end] == "{") - (source[end] == "}")
        if depth == 0:
            return source[match.start() : end + 1]
    raise ValueError(f"Unterminated definition: {signature}")


def block(source, opener, closer):
    start = source.index(opener)
    return source[start : source.index(closer, start) + len(closer)]


def typedef_block(source, name):
    end = source.index("} " + name + ";")
    start = source.rindex("typedef struct {", 0, end)
    return source[start : end + len(name) + 3]


def define_line(source, name):
    return re.search(r"^#define " + re.escape(name) + r" .*$", source, re.M).group(0)


def sanitizer_flags():
    """ASan and UBSan when the toolchain can link them: a use after free then aborts the test.
    Without them the test still fails on an early free, because the free is observed at the
    moment it happens."""
    flags = ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
    cc = shlex.split(os.environ.get("CC", "cc"))
    with tempfile.TemporaryDirectory(prefix="gnoblin-sanitizer-probe-") as probe:
        source = Path(probe) / "probe.c"
        source.write_text("int main(void) { return 0; }\n")
        linked = subprocess.run(
            cc + flags + [str(source), "-o", str(Path(probe) / "probe")],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    return flags if linked.returncode == 0 else []


def build(directory):
    directory.mkdir(parents=True, exist_ok=True)
    cache = {}

    def read(path):
        if path not in cache:
            cache[path] = path.read_text()
        return cache[path]

    def functions(path, signatures):
        source = read(path)
        chunks = []
        for signature in signatures:
            body = function(source, signature)
            line = source[: source.index(body)].count("\n") + 1
            chunks.append(f'#line {line} "{path}"\n{body}\n')
        return "\n".join(chunks)

    internal = read(CONTROL / "gnoblin-control-internal.h")
    private = read(CONTROL / "gnoblin-native-control-private.h")
    shortcuts = INPUT / "gnoblin-control-shortcuts.c"
    focus = CONTROL / "gnoblin-control-focus.c"

    types = "\n".join(
        [
            define_line(private, "MAX_FOCUS_CONTEXTS"),
            define_line(private, "FOCUS_CONTEXT_LIFETIME_US"),
            define_line(read(shortcuts), "MAX_DYNAMIC_SHORTCUTS_PER_CLIENT"),
            define_line(read(shortcuts), "MAX_DYNAMIC_SHORTCUTS"),
            "typedef struct _NativeDynamicShortcut NativeDynamicShortcut;",
            "typedef struct _NativeCornerToolkitCache NativeCornerToolkitCache;",
            "typedef struct _GnoblinControlPromptBroker GnoblinControlPromptBroker;",
            typedef_block(private, "NativeFocusContext"),
            typedef_block(private, "NativeFocusGrant"),
            block(internal, "typedef struct {\n    GSocketConnection* connection;", "} Client;"),
            block(internal, "struct _NativeDynamicShortcut {", "\n};"),
            block(internal, "struct _GnoblinNativeControl {", "\n};"),
        ]
    )
    (directory / "production-types.inc").write_text(types + "\n")

    code = [
        # Teardown ordering: abort, the async completion, the end of compositor teardown.
        functions(
            CONTROL / "gnoblin-control-dispatch.c",
            [
                "void native_control_maybe_free_stopped(",
                "void native_control_finish_teardown(",
                "void gnoblin_control_maybe_free_stopped(",
            ],
        ),
        functions(CONTROL / "gnoblin-native-control.c", ["void native_runtime_abort("]),
        typedef_block(read(CONTROL / "gnoblin-control-power.c"), "PowerCall") + "\n",
        functions(CONTROL / "gnoblin-control-power.c", ["static void power_call_free(", "static void power_finish("]),
        # Shortcut bindings and sessions.
        functions(
            CONTROL / "gnoblin-control-wire.c",
            ["JsonNode* json_from_variant(", "char* encode_response(", "char* gnoblin_control_encode_response("],
        ),
        functions(
            focus,
            [
                "static gboolean dynamic_shortcut_owner_subscribed(",
                "static gboolean dynamic_shortcut_routes_to_lua_runtime(",
                "void dynamic_shortcut_publish_event(",
                "void dynamic_shortcut_end_session(",
                "void gnoblin_control_dynamic_shortcut_end_session(",
                "gboolean native_shortcut_session_timeout(",
                "void dispatch_dynamic_shortcut_activated(",
            ],
        ),
        functions(
            shortcuts,
            [
                "void gnoblin_control_dynamic_shortcut_free(",
                "guint gnoblin_control_dynamic_shortcut_count(",
                "void gnoblin_control_remove_dynamic_shortcut(",
                "void gnoblin_control_clear_runtime_dynamic_shortcuts(",
                "NativeDynamicShortcut* gnoblin_control_find_dynamic_shortcut(",
                "NativeDynamicShortcut* gnoblin_control_find_runtime_dynamic_shortcut(",
                "gboolean gnoblin_control_dynamic_shortcut_id_valid(",
                "NativeDynamicShortcut* gnoblin_control_arm_bare_super_shortcut(",
                "char* gnoblin_control_dynamic_shortcut_chord(",
                "static gboolean dynamic_shortcut_arguments_have_only(",
                "char* gnoblin_control_dynamic_shortcut_bind(",
                "char* gnoblin_control_dynamic_shortcut_request_end_session(",
            ],
        ),
        functions(
            CONTROL / "gnoblin-control-runtime-channel.c", ["static GVariant* native_runtime_shortcut_operation("]
        ),
    ]
    (directory / "production.inc").write_text("\n".join(code))

    binary = directory / "native-control-safety"
    flags = shlex.split(
        subprocess.check_output(
            ["pkg-config", "--cflags", "--libs", "gio-2.0", "json-glib-1.0", "xkbcommon"], text=True
        )
    )
    command = shlex.split(os.environ.get("CC", "cc")) + [
        "-std=gnu11",
        "-O1",
        "-g",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Wno-unused-parameter",
        "-Wno-unused-function",
        *sanitizer_flags(),
        "-I",
        str(directory),
        "-I",
        str(CONTROL),
        str(ROOT / "tests/native-control-safety-test.c"),
        "-o",
        str(binary),
        *flags,
    ]
    subprocess.run(command, check=True)
    return binary


def main():
    if len(sys.argv) > 2:
        sys.exit(__doc__)
    if len(sys.argv) == 2:
        binary = build(Path(sys.argv[1]))
        os.execv(binary, [str(binary)])
    with tempfile.TemporaryDirectory(prefix="gnoblin-control-safety-") as temporary:
        binary = build(Path(temporary))
        subprocess.run([str(binary)], check=True)


if __name__ == "__main__":
    main()
