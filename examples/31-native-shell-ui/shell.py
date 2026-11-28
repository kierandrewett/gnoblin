"""Example 31 in Python: the parts of the stock shell above the toolkit, from libgnoblin-std.

    gnoblin ./shell.py

The classes are generated from the catalog the library serves, so ``gnoblin.objects.std`` has one for each of them. This shell:

- starts the not-responding alert, which looks at the windows and opens a card for one that stops answering the compositor's pings: Escape
  waits, Tab moves to Force quit, and Force quit takes a press only after ``arm_ms``;
- puts the stock lock screen to work: a clock and a password entry over the screen while the session is locked;
- opens the launcher with ``<Super>space``: Return runs the command typed and closes it, Escape closes it;
- flips the overview with ``<Super>o``;
- shows a toast with ``<Super>t``;
- calls the toolkit's two functions that need no canvas: ``Ui.estimate_text`` and ``Ui.encode_paint_list``.

Every line it logs starts with ``shell-ui:``, spelt as the Rust shell of the same example (``31-native-shell-ui``) spells it, so that one test
reads all three.
"""

import struct

from gnoblin import gnoblin
from gnoblin.std import Launcher, LockScreen, NotRespondingAlerts, Overview, PaintKind, PaintOp, Toast, Ui


def say(text):
    gnoblin.log(f"shell-ui: {text}")


# A long delay, so that a Return pressed at once finds Force quit still off.
alerts = NotRespondingAlerts.get()
alerts.arm_ms = 3000.0
alerts.on("answered", lambda client, answer: say(f"the alert for window {client} ended"))
alerts.start()

# Making the screen puts it to work for the generation: it is up while the session is locked.
lock = LockScreen(LockScreen.default_lock_style())
say(f"the lock screen is ready, shown {str(lock.shown).lower()}")

overview = Overview.get()


def flip():
    say(f"the overview is {'on' if overview.toggle() else 'off'}")


gnoblin.bind("<Super>o", flip)


def open_launcher():
    Launcher.open(lambda command: say(f"the launcher ran {command}"))
    say("the launcher is open")


gnoblin.bind("<Super>space", open_launcher)


# A toast fades in for 20 ms, holds for 400 ms and fades out for 20 ms, then removes itself.
def show_toast():
    Toast.show("Toast of the shell", 20.0, 400.0, 20.0)
    say("the toast is up")


gnoblin.bind("<Super>t", show_toast)

# What a text is taken to measure until it is measured: 5 pixels for each of 5 characters at size 10, a line of 15, a baseline at 11.
estimate = Ui.estimate_text("Hello", 10.0)
say(f"estimate {estimate.width:g}x{estimate.height:g} ascent {estimate.ascent:g}")

# The bytes of a paint list: the magic ``GNPL``, a version, a count, then the operations. A list with one bad operation is refused whole.
# The line's width is its ``width``, as it is in the TypeScript library: the bytes carry it as a little-endian f32 after the line's four ends.
listing = Ui.encode_paint_list(
    [
        PaintOp(op=PaintKind.PUSH_OPACITY, alpha=0.5),
        PaintOp(op=PaintKind.LINE, x1=0.0, y1=0.0, x2=5.0, y2=6.0, width=3.0),
        PaintOp(op=PaintKind.POP_OPACITY),
    ]
)
line_width = struct.unpack_from("<f", listing, 31)[0]
say(f"paint list of {len(listing)} bytes {bytes(listing[:4]).decode()}, line width {line_width:g}")
try:
    Ui.encode_paint_list([PaintOp(op=PaintKind.POP_OPACITY), PaintOp(op=PaintKind.RECT, fill="red")])
except Exception as error:
    say(f"bad paint list refused: {error}")

say("running")
