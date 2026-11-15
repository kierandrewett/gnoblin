#!/usr/bin/env python3
"""Drive Mutter's session objects over one bus connection, and record their introspection and the replies.

usage: scripts/reference-session-sessions.py OUT_DIR

Run by scripts/reference-session-record.sh inside the session it records. Mutter ties a ScreenCast, RemoteDesktop or
InputCapture session to the bus connection that created it and ends the session when that connection goes away (its
log says "D-Bus client with active sessions vanished"), so a one-shot `busctl call` cannot show a session's objects.
This script keeps one connection for the whole walk.

It writes into OUT_DIR:

  dbus/sessions.xml      the introspection of every session and stream object while it was alive, one `<!-- :conn path -->`
                         header and one document each, in the form scripts/reference-session-matrix.py reads
  dbus-sessions.txt      every call made: the method, its arguments, and the reply or the D-Bus error, in order

Nothing is retried and no failure is hidden: a call that errors is recorded with the error name and text.
"""

import os
import sys

import dbus

DEST_SCREENCAST = "org.gnome.Mutter.ScreenCast"
DEST_REMOTE = "org.gnome.Mutter.RemoteDesktop"
DEST_INPUT = "org.gnome.Mutter.InputCapture"
INTROSPECTABLE = "org.freedesktop.DBus.Introspectable"


class Recorder:
    def __init__(self, out):
        self.bus = dbus.SessionBus()
        self.xml = open(os.path.join(out, "dbus", "sessions.xml"), "a", encoding="utf-8")
        self.log = open(os.path.join(out, "dbus-sessions.txt"), "w", encoding="utf-8")

    def say(self, text):
        self.log.write(text + "\n")
        self.log.flush()

    def call(self, name, path, interface, method, *args, signature=None):
        """Calls a method and records the call and its outcome. Answers the reply, or None when the call failed."""
        shown = ", ".join(repr(arg) for arg in args)
        self.say(f"$ {name} {path} {interface}.{method}({shown})")
        proxy = self.bus.get_object(name, path, introspect=False)
        try:
            if signature is None:
                reply = proxy.get_dbus_method(method, interface)(*args)
            else:
                reply = proxy.get_dbus_method(method, interface)(*args, signature=signature)
        except dbus.exceptions.DBusException as error:
            self.say(f"  error {error.get_dbus_name()}: {error.get_dbus_message()}")
            return None
        self.say(f"  reply {reply!r}")
        return reply

    def introspect(self, name, path):
        """Records the introspection document of an object while it exists."""
        proxy = self.bus.get_object(name, path, introspect=False)
        try:
            text = proxy.Introspect(dbus_interface=INTROSPECTABLE)
        except dbus.exceptions.DBusException as error:
            self.say(f"  introspect {path} failed: {error.get_dbus_name()}: {error.get_dbus_message()}")
            return
        owner = self.bus.get_name_owner(name)
        self.xml.write(f"<!-- {owner} {path} -->\n{text}\n\n")
        self.xml.flush()

    def get_all(self, name, path, interface):
        self.call(name, path, "org.freedesktop.DBus.Properties", "GetAll", interface)

    def screencast(self):
        session = self.call(
            DEST_SCREENCAST, "/org/gnome/Mutter/ScreenCast", DEST_SCREENCAST, "CreateSession", {}, signature="a{sv}"
        )
        if session is None:
            return
        session = str(session)
        self.introspect(DEST_SCREENCAST, session)
        self.get_all(DEST_SCREENCAST, session, DEST_SCREENCAST + ".Session")
        stream = self.call(
            DEST_SCREENCAST, session, DEST_SCREENCAST + ".Session", "RecordMonitor", "Meta-0", {}, signature="sa{sv}"
        )
        if stream is not None:
            stream = str(stream)
            self.introspect(DEST_SCREENCAST, stream)
            self.get_all(DEST_SCREENCAST, stream, DEST_SCREENCAST + ".Stream")
        self.call(DEST_SCREENCAST, session, DEST_SCREENCAST + ".Session", "Start")
        if stream is not None:
            self.get_all(DEST_SCREENCAST, stream, DEST_SCREENCAST + ".Stream")
        self.call(DEST_SCREENCAST, session, DEST_SCREENCAST + ".Session", "Stop")
        self.introspect(DEST_SCREENCAST, session)

    def remote_desktop(self):
        session = self.call(DEST_REMOTE, "/org/gnome/Mutter/RemoteDesktop", DEST_REMOTE, "CreateSession")
        if session is None:
            return
        session = str(session)
        interface = DEST_REMOTE + ".Session"
        self.introspect(DEST_REMOTE, session)
        self.get_all(DEST_REMOTE, session, interface)
        self.call(DEST_REMOTE, session, interface, "Start")
        self.call(DEST_REMOTE, session, interface, "NotifyPointerMotionRelative", 1.0, 1.0)
        self.call(DEST_REMOTE, session, interface, "NotifyKeyboardKeysym", dbus.UInt32(0x61), True)
        self.call(DEST_REMOTE, session, interface, "NotifyKeyboardKeysym", dbus.UInt32(0x61), False)
        eis = self.call(DEST_REMOTE, session, interface, "ConnectToEIS", {}, signature="a{sv}")
        if eis is not None:
            self.say(f"  (the reply is a file descriptor for the EIS socket: {type(eis).__name__})")
        self.call(DEST_REMOTE, session, interface, "Stop")

    def input_capture(self):
        # SupportedCapabilities is 15: keyboard, pointer, touchscreen and the fourth bit.
        session = self.call(DEST_INPUT, "/org/gnome/Mutter/InputCapture", DEST_INPUT, "CreateSession", dbus.UInt32(3))
        if session is None:
            return
        session = str(session)
        interface = DEST_INPUT + ".Session"
        self.introspect(DEST_INPUT, session)
        self.get_all(DEST_INPUT, session, interface)
        self.call(DEST_INPUT, session, interface, "GetZones")
        self.call(DEST_INPUT, session, interface, "Disable")
        self.call(DEST_INPUT, session, interface, "Close")


def main(argv):
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    recorder = Recorder(argv[1])
    recorder.screencast()
    recorder.remote_desktop()
    recorder.input_capture()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
