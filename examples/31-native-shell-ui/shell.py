"""Example 31 in Python: the parts of the stock shell above the toolkit, from libgnoblin-std.

    gnoblin ./shell.py

The classes are generated from the catalog the library serves, so ``gnoblin.objects.std`` has one for each of them. This shell:

- starts the not-responding alert, which looks at the windows and opens a card for one that stops answering the compositor's pings: Escape
  waits, Tab moves to Force quit, and Force quit takes a press only after ``arm_ms``;
- puts the stock lock screen to work: a clock and a password entry over the screen while the session is locked;
- opens the launcher with ``<Super>space``: Return runs the command typed and closes it, Escape closes it;
- flips the overview with ``<Super>o``.

Every line it logs starts with ``shell-ui:``, spelt as the Rust shell of the same example (``31-native-shell-ui``) spells it, so that one test
reads all three.
"""

from gnoblin import gnoblin
from gnoblin.objects.std import Launcher, LockScreen, NotRespondingAlerts, Overview


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

say("running")
