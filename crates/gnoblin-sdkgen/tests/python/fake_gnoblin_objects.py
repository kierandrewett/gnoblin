"""A stand-in for the built-in module `_gnoblin_objects`, which the Python runtime registers: it keeps what a class writes and hands it back the
way the library does (a record as a tuple of its fields, an enum as its id), so the generated classes can be run, and the values that cross
inspected, with no library."""

import enum

from gnoblin.errors import GnoblinError

_next = [1]


class Namespace:
    def __init__(self, name):
        self.name = name
        self.store = {}
        self.calls = []
        self.listeners = {}
        self.implemented = {}
        self.classes = {}
        # The code of the failure the next call of the library raises, once: what a real failure of the library carries in `code`.
        self.refuse = None

    def fail(self):
        if self.refuse is not None:
            code, self.refuse = self.refuse, None
            error = GnoblinError(f"the library refused the call ({code})")
            error.code = code
            raise error

    def ids(self, class_name, names):
        class_id = self.classes.setdefault(class_name, 100 + len(self.classes))
        return [class_id] + [200 + index for index, _ in enumerate(names)]

    def _handle(self, class_id):
        _next[0] += 1
        return Handle(self, class_id, _next[0])

    def create(self, class_id, inits):
        self.fail()
        handle = self._handle(class_id)
        for member, value in inits:
            self.store[(handle.obj, member)] = convert(value)
        return handle

    def call_static(self, class_id, member, args):
        self.fail()
        self.calls.append(("static", member, [convert(arg) for arg in args]))
        return self.answer(member, args)

    def implement(self, class_id, members, function):
        handle = self._handle(class_id)
        self.implemented[handle.obj] = (list(members), function)
        return handle

    def unlisten(self, registration):
        self.listeners.pop(registration, None)

    def answer(self, member, args):
        # The members the check calls: `member - 200 + 1` is the place of the name in the class's list.
        return convert(args[0]) if args else None


class Handle:
    def __init__(self, namespace, class_id, obj):
        self.namespace = namespace
        self.class_id = class_id
        self.obj = obj

    @property
    def class_name(self):
        return next(name for name, class_id in self.namespace.classes.items() if class_id == self.class_id)

    def get(self, member):
        self.namespace.fail()
        return self.namespace.store.get((self.obj, member))

    def set(self, member, value):
        self.namespace.fail()
        self.namespace.store[(self.obj, member)] = convert(value)

    def set_timed(self, member, value, timing):
        self.namespace.fail()
        self.namespace.calls.append(("timed", member, convert(value), timing))

    def call(self, member, args):
        self.namespace.fail()
        self.namespace.calls.append(("call", member, [convert(arg) for arg in args]))
        return self.namespace.answer(member, args)

    def listen(self, member, function):
        self.namespace.fail()
        registration = len(self.namespace.listeners) + 1
        self.namespace.listeners[registration] = (member, function)
        return registration

    def destroy(self):
        self.namespace.calls.append(("destroy", self.obj))


def convert(value):
    """What the library keeps of a value: a record as a tuple, an enum as its id, a list as a list, anything else as it is."""
    if isinstance(value, enum.Enum):
        return value.value
    if isinstance(value, dict):
        return tuple(convert(field) for field in value.values())
    if isinstance(value, (list, tuple)):
        return [convert(item) for item in value]
    if isinstance(value, Handle):
        return value
    return value


def open(namespace):
    return Namespace(namespace)
