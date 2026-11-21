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
    MoveBinding,
    Repositioning,
    RequestListener,
    ResizeBinding,
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

say("running")
