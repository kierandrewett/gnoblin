#!/usr/bin/env python3
"""Check Gnoblin's XSettings scale, live reload, and selection-loss cleanup."""

import ctypes
import ctypes.util
import json
import os
import pathlib
import re
import struct
import subprocess
import time


runtime_log = pathlib.Path(os.environ["GNOBLIN_DEVKIT_RUNTIME_LOG"])
log = runtime_log.read_text(errors="replace")
display_lines = [
    line.split("Using public X11 display ", 1)[1].split(",", 1)[0]
    for line in log.splitlines()
    if "Using public X11 display " in line
]
if not display_lines:
    raise SystemExit("Gnoblin did not report its nested X11 display")
display_name = display_lines[-1]


def nested_xwayland_auth():
    for cmdline in pathlib.Path("/proc").glob("[0-9]*/cmdline"):
        try:
            argv = cmdline.read_bytes().split(b"\0")
        except OSError:
            continue
        if not argv or pathlib.Path(os.fsdecode(argv[0])).name != "Xwayland":
            continue
        if os.fsencode(display_name) not in argv:
            continue
        try:
            return os.fsdecode(argv[argv.index(b"-auth") + 1])
        except (ValueError, IndexError):
            continue
    return None


x11 = ctypes.CDLL(ctypes.util.find_library("X11"))
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XDefaultScreen.argtypes = [ctypes.c_void_p]
x11.XDefaultScreen.restype = ctypes.c_int
x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
x11.XInternAtom.restype = ctypes.c_ulong
x11.XGetSelectionOwner.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XGetSelectionOwner.restype = ctypes.c_ulong
x11.XCreateSimpleWindow.argtypes = [
    ctypes.c_void_p,
    ctypes.c_ulong,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_uint,
    ctypes.c_uint,
    ctypes.c_uint,
    ctypes.c_ulong,
    ctypes.c_ulong,
]
x11.XCreateSimpleWindow.restype = ctypes.c_ulong
x11.XSetSelectionOwner.argtypes = [
    ctypes.c_void_p,
    ctypes.c_ulong,
    ctypes.c_ulong,
    ctypes.c_ulong,
]
x11.XChangeProperty.argtypes = [
    ctypes.c_void_p,
    ctypes.c_ulong,
    ctypes.c_ulong,
    ctypes.c_ulong,
    ctypes.c_int,
    ctypes.c_int,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int,
]
x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
x11.XDestroyWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XGetWindowProperty.argtypes = [
    ctypes.c_void_p,
    ctypes.c_ulong,
    ctypes.c_ulong,
    ctypes.c_long,
    ctypes.c_long,
    ctypes.c_int,
    ctypes.c_ulong,
    ctypes.POINTER(ctypes.c_ulong),
    ctypes.POINTER(ctypes.c_int),
    ctypes.POINTER(ctypes.c_ulong),
    ctypes.POINTER(ctypes.c_ulong),
    ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte)),
]
x11.XGetWindowProperty.restype = ctypes.c_int
x11.XFree.argtypes = [ctypes.c_void_p]
x11.XCloseDisplay.argtypes = [ctypes.c_void_p]


# Xwayland starts on demand. An unauthenticated connection request is enough to
# trigger startup; it fails harmlessly while Xwayland prepares its private auth.
os.environ["DISPLAY"] = display_name
os.environ.pop("XAUTHORITY", None)
authority = None
for _ in range(100):
    saved_stderr = os.dup(2)
    null_fd = os.open(os.devnull, os.O_WRONLY)
    try:
        os.dup2(null_fd, 2)
        request = x11.XOpenDisplay(display_name.encode())
    finally:
        os.dup2(saved_stderr, 2)
        os.close(saved_stderr)
        os.close(null_fd)
    if request:
        x11.XCloseDisplay(request)
    authority = nested_xwayland_auth()
    if authority and pathlib.Path(authority).is_file():
        break
    time.sleep(0.1)
if not authority or not pathlib.Path(authority).is_file():
    raise SystemExit("could not find the nested Xwayland authority file")
os.environ["XAUTHORITY"] = authority

display = x11.XOpenDisplay(None)
if not display:
    raise SystemExit("could not open the nested X display")


def property_bytes(window, atom):
    actual_type = ctypes.c_ulong()
    actual_format = ctypes.c_int()
    count = ctypes.c_ulong()
    remaining = ctypes.c_ulong()
    data = ctypes.POINTER(ctypes.c_ubyte)()
    status = x11.XGetWindowProperty(
        display,
        window,
        atom,
        0,
        65536,
        0,
        0,
        ctypes.byref(actual_type),
        ctypes.byref(actual_format),
        ctypes.byref(count),
        ctypes.byref(remaining),
        ctypes.byref(data),
    )
    if status != 0:
        raise SystemExit(f"XGetWindowProperty failed: {status}")
    try:
        if remaining.value:
            raise SystemExit("X property exceeds test limit")
        return actual_type.value, actual_format.value, bytes(data[: count.value])
    finally:
        if data:
            x11.XFree(data)


def settings_values(owner, settings_atom):
    settings_type, settings_format, blob = property_bytes(owner, settings_atom)
    if settings_type != settings_atom or settings_format != 8 or len(blob) < 12:
        raise SystemExit("XSettings property has an invalid type, format, or header")
    # The XSETTINGS specification stores the X11 byte-order constants: LSBFirst is 0 and MSBFirst is 1. GTK clients
    # warn "Invalid XSETTINGS" when the marker is an ASCII letter.
    if blob[0:1] == b"\x00":
        endian = "<"
    elif blob[0:1] == b"\x01":
        endian = ">"
    else:
        raise SystemExit("XSettings property has an invalid byte-order marker")

    serial, setting_count = struct.unpack_from(endian + "II", blob, 4)
    position = 12
    values = {}
    for _ in range(setting_count):
        if position + 4 > len(blob):
            raise SystemExit("XSettings property ends inside a setting header")
        setting_type = blob[position]
        name_length = struct.unpack_from(endian + "H", blob, position + 2)[0]
        position += 4
        if position + name_length > len(blob):
            raise SystemExit("XSettings property ends inside a setting name")
        name = blob[position : position + name_length].decode("ascii")
        position += name_length
        position = (position + 3) & ~3
        if position + 8 > len(blob):
            raise SystemExit("XSettings property ends inside a setting value")
        _changed_serial, value = struct.unpack_from(endian + "Ii", blob, position)
        position += 8
        if setting_type != 0:
            raise SystemExit(f"unexpected XSettings value type for {name}")
        values[name] = value

    if position != len(blob):
        raise SystemExit("XSettings property has trailing bytes")
    expected_names = {
        "Gdk/WindowScalingFactor",
        "Gdk/UnscaledDPI",
        "Xft/DPI",
    }
    if set(values) != expected_names:
        raise SystemExit(f"unexpected XSettings entries: {sorted(values)}")
    return serial, values


screen = x11.XDefaultScreen(display)
selection_name = f"_XSETTINGS_S{screen}".encode()
selection_atom = x11.XInternAtom(display, selection_name, 0)
settings_atom = x11.XInternAtom(display, b"_XSETTINGS_SETTINGS", 0)
owner = x11.XGetSelectionOwner(display, selection_atom)
if not owner:
    raise SystemExit("Gnoblin does not own the XSettings manager selection")

serial, values = settings_values(owner, settings_atom)

resource_atom = x11.XInternAtom(display, b"RESOURCE_MANAGER", 0)
resource_type, resource_format, resource_blob = property_bytes(x11.XDefaultRootWindow(display), resource_atom)
if resource_type == 0:
    resources = b""
elif resource_format == 8:
    resources = resource_blob
else:
    raise SystemExit("RESOURCE_MANAGER has an invalid format")

scale = values["Gdk/WindowScalingFactor"]
expected_scale = int(os.environ.get("GNOBLIN_EXPECTED_SCALE", "1"))
if scale != expected_scale:
    raise SystemExit(f"expected XSettings scale {expected_scale}, got {scale}")
if scale < 1 or values["Gdk/UnscaledDPI"] != 96 * 1024:
    raise SystemExit(f"unexpected scale/DPI values: {values}")
if values["Xft/DPI"] != 96 * 1024 * scale:
    raise SystemExit(f"unexpected scaled Xft/DPI: {values}")
if f"Xft.dpi:\t{96 * scale}".encode() not in resources:
    raise SystemExit(f"RESOURCE_MANAGER is missing scaled Xft.dpi: {resources!r}")

config_path = pathlib.Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin" / "init.lua"
new_scale = expected_scale + 1
config = config_path.read_text()
config, replacements = re.subn(
    r"(scaling_factor\s*=\s*)[-+]?\d+(?:\.\d+)?",
    rf"\g<1>{new_scale}",
    config,
    count=1,
)
if replacements != 1:
    raise SystemExit("could not find the configured XWayland scaling factor")
config_path.write_text(config)
subprocess.run(["gnoblinctl", "config", "reload"], check=True, timeout=10)

scale_reloaded = False
for _ in range(100):
    owner = x11.XGetSelectionOwner(display, selection_atom)
    if owner:
        serial, values = settings_values(owner, settings_atom)
        resource_type, resource_format, resource_blob = property_bytes(x11.XDefaultRootWindow(display), resource_atom)
        if (
            values["Gdk/WindowScalingFactor"] == new_scale
            and values["Xft/DPI"] == 96 * 1024 * new_scale
            and resource_format == 8
            and f"Xft.dpi:\t{96 * new_scale}".encode() in resource_blob
        ):
            scale_reloaded = True
            resources = resource_blob
            break
    time.sleep(0.05)

if not scale_reloaded:
    raise SystemExit("XSettings did not follow a live Gnoblin scaling-factor reload")
scale = new_scale

root = x11.XDefaultRootWindow(display)
string_atom = x11.XInternAtom(display, b"STRING", 0)
resource_line = f"Xft.dpi:\t{96 * scale}\n".encode()
preserved_line = b"GnoblinTest.resource: preserved\n"
external_resources = resource_line + preserved_line
external_resources_buffer = (ctypes.c_ubyte * len(external_resources)).from_buffer_copy(external_resources)
x11.XChangeProperty(
    display,
    root,
    resource_atom,
    string_atom,
    8,
    0,
    external_resources_buffer,
    len(external_resources),
)
competitor = x11.XCreateSimpleWindow(display, root, 0, 0, 1, 1, 0, 0, 0)
x11.XSetSelectionOwner(display, selection_atom, competitor, 0)
x11.XSync(display, 0)

selection_loss_handled = False
for _ in range(100):
    if x11.XGetSelectionOwner(display, selection_atom) != competitor:
        time.sleep(0.01)
        continue
    resource_type, resource_format, resource_blob = property_bytes(root, resource_atom)
    if resource_format == 8 and b"Xft.dpi:" not in resource_blob and preserved_line in resource_blob:
        selection_loss_handled = True
        break
    time.sleep(0.01)

if not selection_loss_handled:
    raise SystemExit("XSettings owner loss did not restore DPI and preserve other resources")

print(
    json.dumps(
        {
            "selection_owner": owner,
            "serial": serial,
            "scale": scale,
            "settings": values,
            "resource_manager": resources.decode("ascii", "replace"),
            "scale_reload_handled": scale_reloaded,
            "selection_loss_handled": selection_loss_handled,
        },
        sort_keys=True,
    )
)
x11.XDestroyWindow(display, competitor)
x11.XCloseDisplay(display)
