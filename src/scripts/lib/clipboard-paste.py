#!/usr/bin/env python3
"""Temporary X11 clipboard owner. Preserve every offered data format in memory.

Protocol: READY -> CHECK -> ARMED -> PASTE or ABORT -> DONE. After restoration, keep
serving the saved clipboard until another application takes ownership.
"""

import ctypes as C
import json
import signal
import sys

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402 - Select GI versions before importing their modules.


class Paste:
    def __init__(self, text):
        self.clipboard = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
        self.text = text.encode("utf-8")
        self.saved = []
        self.total = 0
        self.stage = "capture"
        self.changed = False
        self.finished = False
        self.payload = []
        self.clipboard.connect("owner-change", self.owner_changed)
        # Gtk omits set_with_data from introspection because it has two callbacks.
        # Keep both C callbacks and their backing bytes alive while we own it.
        self.gtk = C.CDLL("libgtk-3.so.0")
        self.gdk = C.CDLL("libgdk-3.so.0")
        self.gdk.gdk_atom_intern.restype = C.c_void_p
        self.gdk.gdk_atom_intern.argtypes = [C.c_char_p, C.c_int]
        self.gtk.gtk_selection_data_set.argtypes = [C.c_void_p, C.c_void_p, C.c_int, C.c_void_p, C.c_int]
        self.get_callback = C.CFUNCTYPE(None, C.c_void_p, C.c_void_p, C.c_uint, C.c_void_p)(self.provide)
        self.clear_callback = C.CFUNCTYPE(None, C.c_void_p, C.c_void_p)(self.cleared)

        class Target(C.Structure):
            _fields_ = [("target", C.c_char_p), ("flags", C.c_uint), ("info", C.c_uint)]

        self.Target = Target
        self.gtk.gtk_clipboard_set_with_data.argtypes = [
            C.c_void_p,
            C.POINTER(Target),
            C.c_uint,
            type(self.get_callback),
            type(self.clear_callback),
            C.c_void_p,
        ]
        self.gtk.gtk_clipboard_set_with_data.restype = C.c_int
        C.pythonapi.PyCapsule_GetPointer.restype = C.c_void_p
        C.pythonapi.PyCapsule_GetPointer.argtypes = [C.py_object, C.c_char_p]
        self.ptr = C.pythonapi.PyCapsule_GetPointer(self.clipboard.__gpointer__, None)
        GLib.timeout_add(3000, self.timeout)
        GLib.io_add_watch(sys.stdin, GLib.IO_IN | GLib.IO_HUP, self.command)
        self.clipboard.request_targets(self.targets)

    def emit(self, event, message=""):
        try:
            print(json.dumps({"event": event, "message": message}), flush=True)
        except BrokenPipeError:
            pass

    def owner_changed(self, *_args):
        if self.stage == "capture":
            self.changed = True

    def targets(self, _clipboard, atoms, *_args):
        self.pending = [
            a for a in (atoms or []) if a.name() not in {"TARGETS", "MULTIPLE", "TIMESTAMP", "SAVE_TARGETS"}
        ]
        if len(self.pending) > 64:
            self.fail("Too many clipboard formats to preserve safely.")
        else:
            self.capture_next()

    def capture_next(self):
        if self.stage != "capture":
            return
        if self.changed:
            return self.fail("Clipboard changed before paste. Please try again.")
        if self.pending:
            atom = self.pending.pop(0)
            self.clipboard.request_contents(atom, self.captured, atom.name())
            return
        self.stage = "temporary"
        if not self.own(
            [
                ("UTF8_STRING", "UTF8_STRING", 8, self.text),
                ("text/plain;charset=utf-8", "UTF8_STRING", 8, self.text),
                ("text/plain", "UTF8_STRING", 8, self.text),
            ]
        ):
            return self.fail("Cannot set the temporary clipboard.")
        self.emit("ready")

    def captured(self, _clipboard, data, name):
        if data.get_length() < 0:
            return self.fail("Cannot preserve clipboard format: " + name)
        value = data.get_data()
        self.total += len(value)
        if self.total > 64 * 1024 * 1024:
            return self.fail("Clipboard is too large to preserve safely.")
        self.saved.append((name, data.get_data_type().name(), data.get_format(), value))
        self.capture_next()

    def own(self, payload):
        self.payload = payload
        self.entries = (self.Target * len(payload))(
            *[self.Target(row[0].encode(), 0, i) for i, row in enumerate(payload)]
        )
        return self.gtk.gtk_clipboard_set_with_data(
            self.ptr, self.entries, len(payload), self.get_callback, self.clear_callback, None
        )

    def provide(self, _clipboard, data, index, _user):
        _name, kind, fmt, value = self.payload[index]
        atom = self.gdk.gdk_atom_intern(kind.encode(), 0)
        self.gtk.gtk_selection_data_set(data, atom, fmt, value, len(value))

    def cleared(self, *_args):
        if self.stage == "temporary":
            self.stage = "replaced"
            self.finish()
        elif self.stage == "saved":
            Gtk.main_quit()

    def command(self, _stream, condition):
        line = sys.stdin.readline().strip()
        if line == "CHECK" and self.stage == "temporary":
            self.emit("armed")
            return True
        if line == "PASTE" and self.stage == "temporary":
            GLib.timeout_add(600, self.finish)
        else:
            self.finish("Paste cancelled.")
        return False

    def timeout(self):
        if not self.finished:
            self.finish("Paste timed out.")
        return False

    def fail(self, message):
        self.finish(message)

    def finish(self, message=""):
        if self.finished:
            return False
        self.finished = True
        if self.stage == "temporary":
            self.stage = "restoring"
            if self.saved:
                if not self.own(self.saved):
                    message = "Could not restore the clipboard."
                self.stage = "saved"
            else:
                self.clipboard.clear()
                self.stage = "empty"
        self.emit("error" if message else "done", message)
        if self.stage != "saved":
            GLib.idle_add(Gtk.main_quit)
        return False


if __name__ == "__main__":
    text = json.loads(sys.stdin.readline(1024))
    if not text or len(text.encode("utf-16-le")) > 128 or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in text):
        raise SystemExit("Invalid paste text")
    Gtk.init([])
    paste = Paste(text)
    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, lambda: paste.finish("Paste cancelled."))
    Gtk.main()
