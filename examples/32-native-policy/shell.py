"""Example 32 in Python: the window policy of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

Starts the focus policy, the stock answers to the requests of windows, the keyboard move and resize, the move and resize drags and the
three-finger workspace slide. A listener of this shell hears every request of a window first and logs it; it refuses ``fullscreen`` itself, so
the stock answer does not run. Every line starts with ``policy:``, as the Rust shell of the same example spells it.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.std import (
    ClientRequestKind,
    FocusPolicy,
    Gestures,
    MoveBinding,
    PinchActionBinding,
    Repositioning,
    RequestListener,
    ResizeBinding,
    SwipeActionBinding,
    WindowState,
    WorkspaceSlideBinding,
)


def say(text):
    session.log(f"policy: {text}")


FocusPolicy.get().start()


class Listener(RequestListener):
    def on_request(self, request):
        say(f"request {request.kind.name.lower()} of window {request.client}")
        if request.kind == ClientRequestKind.FULLSCREEN:
            request.handle()
            say("fullscreen refused")


state = WindowState.get()
state.add_listener(Listener())
state.start()

Repositioning.get().start()

MoveBinding().install()
ResizeBinding().install()
WorkspaceSlideBinding().install()

# The gestures that run actions: a four-finger swipe up maximises (or restores) the window with the keyboard, a swipe left runs a function, and
# a pinch in minimises. The ways and the actions are checked when they are set.
swipe = SwipeActionBinding()
swipe.trigger = "Swipe4"
swipe.set_action("up", "window.toggle-maximise")
swipe.set_function("left", lambda event: say(f"swipe left went {event.dx:g}"))
swipe.install()
pinch = PinchActionBinding()
pinch.set_action("in", "window.minimise")
pinch.set_function("out", lambda event: say(f"pinch out to {event.scale:g}"))
pinch.install()
say("the gesture bindings are installed")

# The way a gesture has gone, by its longer axis, and none while it has gone less than the threshold.
for dx, dy, threshold in ((-100.0, 5.0, None), (1.0, 1.0, None), (1.0, 1.0, 0.5)):
    way = Gestures.direction(dx, dy, threshold)
    say(f"gesture direction {way.way.name.lower() if way is not None else 'none'}")
try:
    swipe.set_action("down", "window.fly")
except Exception:
    say("an action that is not one is refused")

say("running")
