"""A stand-in for the module `gnoblin.errors`, which the Python runtime fills: the generated classes name a failure of the library by
setting `target` on a `GnoblinError`, and a library's failure carries its `code`."""


class GnoblinError(Exception):
    code = None
    target = None
