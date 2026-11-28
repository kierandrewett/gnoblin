"""The golden shell of the Python runtime: the hand-written surface of the package, as a script uses it.

It runs in `tests/shells.rs` over the fake compositor, and a type checker holds it to the installed stubs (`pyright`, when the machine has
one), so what the stubs say and what the runtime does are checked against the same script.
"""

from gnoblin import NodeGoneError, gnoblin
from gnoblin.host import Process, ProcessExit, Subscription, Window, WindowRequest


def note(text: str) -> None:
    gnoblin.log("golden " + text)


# A named subscription is a decorator that returns the function; called with its parameters and the function, it returns the Subscription.
@gnoblin.on_client_created
def placed(window: Window) -> None:
    window.x = 40.0
    window.y = 40.0
    note("placed %d %s" % (window.node_id, window.app_id))


gone: Subscription = gnoblin.on_node_destroyed("window", lambda window: note("gone %d" % window.node_id))


@gnoblin.bind("<Super>Return")
def terminal() -> None:
    gnoblin.spawn("foot")


chords: Subscription = gnoblin.bind_events("<Super>e", lambda event: note("chord " + event.chord + " " + event.state))


def requested(window: Window, request: WindowRequest) -> None:
    note("request " + request.name)


gnoblin.on_window_request(requested)

# State kept across a reload, and a timer that never fires within the test.
kept = gnoblin.kept_state("runs")
runs = int(kept[0]) + 1 if kept else 1
gnoblin.keep_state("runs", str(runs).encode(), version=1)
note("runs %d" % runs)
late: Subscription = gnoblin.after(60_000, lambda: note("late"))


def done(result: ProcessExit) -> None:
    note("exit %s %s" % (result.code, result.success))


process: Process = gnoblin.spawn(["sh", "-c", "exit 0"], stdout="capture", on_exit=done)

# The tree: typed windows, a node by id, a failed read as an exception.
for window in gnoblin.windows:
    note("window %s %s" % (window.node_id, window.title))
try:
    gnoblin.node(999_999).get("title")
except NodeGoneError:
    note("node gone")

# A registration is removed by its Subscription, or by the function it was made for.
late.unsubscribe()


def titled(window: Window, title: str) -> None:
    note("titled " + title)


gnoblin.on_title_changed(titled)
note("off %d" % gnoblin.off(titled))
note("info %s" % sorted(gnoblin.info))
