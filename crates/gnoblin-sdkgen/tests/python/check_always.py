"""Runs the generated Python class `Thing` of a catalog with an object property that may be absent (`plain`) and one the catalog says the
library always has an object for (`held`), against `fake_gnoblin_objects`: what each reads when the library has no object, and when it has."""

import sys

from gnoblin.errors import GnoblinError
from gnoblin.objects import held as h

failures = []


def check(what, got, wanted):
    if got != wanted:
        failures.append(f"{what}: got {got!r}, wanted {wanted!r}")


thing = h.Thing("a")
other = h.Thing("b")

# An object property that may be absent reads None while the library has no object, and the object once it has one (the same Python
# object it was set from: an object handed over twice is one object).
check("an absent object reads None", thing.plain, None)
thing.plain = other
check("an object that is set reads back as itself", thing.plain is other, True)

# An `always` property is read as an object: the library having none is its fault, and the error names the property.
try:
    thing.held
    failures.append("an `always` property with no object read as None")
except GnoblinError as error:
    check("the error names the property", "Thing.held" in str(error), True)
    check("the error says why", "always" in str(error), True)
thing.held = other
check("an `always` property with an object reads it", thing.held is other, True)

# Nothing else changes: the member in front of it still reads, so a refusal is the property's, not the class's.
check("a plain property still reads", thing.size, None)

if failures:
    print("\n".join(failures))
    sys.exit(1)
