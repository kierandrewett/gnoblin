"""Runs the generated Python classes of the `probe` catalog against `fake_gnoblin_objects`, and checks the values that cross: what a class
sends (a record as a dict of its fields, an enum member, a handle, a closure) and what it makes of what comes back."""

import sys

from gnoblin.errors import GnoblinError
from gnoblin.objects import probe as p
from gnoblin.objects._base import Listener, Timing

failures = []


def check(what, got, wanted):
    if got != wanted:
        failures.append(f"{what}: got {got!r}, wanted {wanted!r}")


ns = p._NS

# An enum member, by name or by value; a name the catalog does not have is a TypeError that lists the ones it does.
probe = p.Probe(3)
check("a constructor argument is a property that starts set", probe._h.get(ns.ids("probe", ["p:count"])[1]), 3)
probe.shade = p.Shade.HALF_TONE
check("an enum goes in as its id", probe._h.get(ns.ids("probe", ["p:count", "p:shade"])[2]), 3)
probe.shade = "dark"
check("an enum goes in by its name", probe._h.get(ns.ids("probe", ["p:count", "p:shade"])[2]), 2)
try:
    probe.shade = "sideways"
    failures.append("a name that is not an enum value was accepted")
except TypeError as error:
    check("the error names the values", "light, dark, half_tone" in str(error), True)

# A record goes in as a dict of its fields in the catalog's order with the defaults filled, and comes back as a dataclass.
probe.origin = p.Point(x=3, y=4, label="here")
check("a record round trips", probe.origin, p.Point(x=3, y=4, label="here", shade=p.Shade.DARK))
check("a record that was never set is absent", p.Probe(0).origin, None)
check(
    "a list of records goes in", probe.total([p.Point(x=1.5), p.Point(x=2)]), [(1.5, 0.0, None, 2), (2, 0.0, None, 2)]
)

# The options of a constructor are keyword-only: each sets the property of its name in the call that makes the object, and one that is
# left out (or None) is not sent, so the library keeps its default.
count_id, shade_id, origin_id, names_id = ns.ids("probe", ["p:count", "p:shade", "p:origin", "p:names"])[1:]
made = p.Probe(2, shade="dark", names=["a", "b"])
check("a positional property is unchanged", made._h.get(count_id), 2)
check("an option is a property that starts set", made._h.get(shade_id), 2)
check("an option that is a list", made._h.get(names_id), ["a", "b"])
check("an option that is left out is not sent", (made._h.obj, origin_id) in ns.store, False)
check("an option that is None is not sent", (p.Probe(1, shade=None)._h.obj, shade_id) in ns.store, False)
check("the constructor without options is unchanged", p.Probe(5)._h.get(count_id), 5)
try:
    p.Probe(1, "dark")
    failures.append("an option was accepted as a positional argument")
except TypeError:
    pass
try:
    p.Probe(1, count=3)
    failures.append("a property of the constructor was accepted as an option")
except TypeError:
    pass

# A required field must be given.
try:
    p._enc_rec_span(p.Span(start=1, end=None))
    failures.append("a record with a missing required field was accepted")
except TypeError as error:
    check("the missing field is named", "Span.end is required" in str(error), True)

# The family of a tagged value: each variant is a class, the record goes in with the family's fields (the tag, then what the variant
# carries), and what comes back is the variant its tag says. A zero is not the default.
shapes = [
    p.CircleShape(),
    p.CircleShape(radius=0),
    p.PolygonShape(points=[0, 1, 2]),
    p.PolygonShape(points=[], closed=False),
]
check("a family round trips as its variants", probe.echo_shapes(shapes), shapes)
check("a circle starts at radius 1", p.CircleShape().radius, 1.0)
check("a polygon starts closed", p.PolygonShape(points=[]).closed, True)
check(
    "a variant goes in as the family's record",
    p._enc_rec_shape(shapes[2]),
    {"form": p.Form.POLYGON, "size": None, "outline": {"points": [0, 1, 2], "closed": True}},
)
try:
    p._enc_rec_shape({"form": "circle"})
    failures.append("a plain dict was accepted for a family")
except TypeError as error:
    check("the error names the variants", "CircleShape, PolygonShape" in str(error), True)
try:
    p._dec_rec_shape((9, None, None))
    failures.append("a record of a kind no variant has was accepted")
except ValueError:
    pass

# A closure property is called with decoded arguments, and its answer is encoded.
probe.on_tick = lambda x: x * 2
check(
    "a closure property is set",
    callable(probe._h.get(ns.ids("probe", ["p:count", "p:shade", "p:origin", "p:names", "p:on_tick"])[5])),
    True,
)
run = probe._h.get(ns.ids("probe", ["p:count", "p:shade", "p:origin", "p:names", "p:on_tick"])[5])
check("a closure property answers", run(0, [4]), 8)


# An interface the script subclasses: only the methods it defined are implemented, and the library's call reaches them.
class Red(p.Paint):
    def colour(self):
        return "red"

    def size(self):
        return 3.0


probe.painter = Red()
painter = probe.painter
implemented, function = next(iter(ns.implemented.values()))
check("only the methods the subclass defined are implemented", len(implemented), 2)
check("a method is called with the library's arguments", function(implemented[0], []), "red")
check("an interface's object reads back", isinstance(painter, p.Paint), True)


class Plain(p.Paint):
    def colour(self):
        return "teal"


probe.painter = Plain()
implemented, function = list(ns.implemented.values())[-1]
check("an optional method that is absent is not implemented", len(implemented), 1)
try:
    probe.painter = p.Paint()
    implemented, function = list(ns.implemented.values())[-1]
    check("a base class with no method implements none", len(implemented), 0)
except TypeError:
    failures.append("an interface instance with no methods was refused by the script, which is the library's to refuse")
try:
    probe.painter = 3
    failures.append("a number was accepted as an object")
except TypeError:
    pass

# A listener lasts until it is stopped, and an event is a signal or a property's change.
heard = []
listener = probe.on("ticked", heard.append)
check("on returns a listener", isinstance(listener, Listener), True)
member, function = ns.listeners[1]
function(member, [4])
check("a signal's arguments are decoded", heard, [4])
changed = []


@probe.on("count_changed")
def _(count):
    changed.append(count)


member, function = ns.listeners[2]
function(member, [7])
check("a property's change is an event", changed, [7])
listener.stop()
listener.stop()
check("a stopped listener is gone", 1 in ns.listeners, False)
try:
    probe.on("nothing", print)
    failures.append("an event the class does not have was accepted")
except TypeError as error:
    check("the events are named", "ticked" in str(error), True)

# A static function and a call with an optional argument.
check("a static function", p.Probe.clamp(15, 0, 10), 15)
check("an optional argument that is left out is none", probe.greet("ada"), "ada")
check("an optional argument that is given", probe.greet("ada", True), "ada")


# A failure of the library is a GnoblinError that names the call and keeps the library's code. Each member kind names itself; a function and
# a constructor have no object, so no `#id`. A bad argument is a TypeError and carries no code.
def refused(what, call, target, code="not_found"):
    ns.refuse = code
    try:
        call()
        failures.append(f"{what}: the refused call returned")
    except GnoblinError as error:
        check(f"{what}: code", error.code, code)
        check(f"{what}: target", error.target, target)
        check(f"{what}: the words begin with the target", str(error).startswith(f"{target}: "), True)
    except Exception as error:
        failures.append(f"{what}: raised {type(error).__name__}, not a GnoblinError")
    ns.refuse = None


def write_count():
    probe.count = 1


refused("a getter", lambda: probe.count, "Probe.count")
refused("a setter", write_count, "Probe.count", "read_only")
refused("a method", lambda: probe.add(1, 2), "Probe.add", "invalid_state")
refused("a function", lambda: p.Probe.clamp(1, 0, 2), "Probe.clamp", "unavailable")
refused("a constructor", lambda: p.Probe(1), "Probe.new", "already_exists")
refused("a listener", lambda: probe.on("ticked", print), "Probe.ticked", "not_found")
try:
    probe.on("nothing", print)
except TypeError as error:
    check("a bad argument has no code", getattr(error, "code", None), None)

# A reference is ended by destroy.
probe.destroy()
check("destroy reaches the library", ("destroy", probe._h.obj) in ns.calls, True)
check("timing", Timing(seconds=1.0) == Timing(seconds=1.0, delay=0.0, tween=0, samples=[]), True)

if failures:
    print("\n".join(failures))
    sys.exit(1)
print("probe classes ok")
