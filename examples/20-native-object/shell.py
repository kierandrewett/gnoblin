"""Example 20 in Python: the object classes of libgnoblin-std.

    gnoblin ./shell.py

The classes of the ``gnoblin.std`` catalog (an entry model, the focus policy, the records and enums they take) are not written by hand: they
are generated from the catalog the library serves and call it through one object table, so ``gnoblin.objects.std`` has a class for each of
them and the stubs come from the same catalog. This shell uses them the way a toolkit would:

- makes an ``EntryModel`` and reads its properties (``text``, ``length``, ``caret``, ``selection``);
- calls its methods (``insert``, ``select_all``), which change it at once, and listens to its ``changed`` signal, which the library tells
  from its next turn and never inside the call that caused it;
- calls a static function of the focus policy with records and an enum, and reads the record it answers with (a field the library leaves
  out is ``None``);
- keeps the entry in the closure of a chord: ``<Super>e`` types one more ``!`` into it.

Every line it logs starts with ``object:``, spelt as the Rust shell of the same example (``20-native-object``) spells it, so that one test
reads all three.
"""

import json

from gnoblin import gnoblin
from gnoblin.std import EntryModel, FocusEvent, FocusEventKind, FocusMode, FocusPolicy, FocusSettings


def say(text):
    gnoblin.log(f"object: {text}")


def flag(value):
    return "true" if value else "false"


# Made with its text; the caret starts at the end, and positions count characters, not bytes.
entry = EntryModel("héllo")
say(f"made an entry of {entry.length} characters with the caret at {entry.caret}")

# The library tells ``changed`` from its next turn, so these lines come after the ones above and below.
heard = 0


@entry.on("changed")
def changed():
    global heard
    heard += 1
    say(f"heard changed #{heard}")


entry.insert(", world")
entry.select_all()
selection = entry.selection
described = "None" if selection is None else f"Some(TextRange {{ start: {selection.start}, end: {selection.end} }})"
say(
    f"text {json.dumps(entry.text, ensure_ascii=False)} selected {json.dumps(entry.selected_text, ensure_ascii=False)} as {described}"
)

# Typing over a selection replaces it, so put the caret at the end first: ``extend`` is an optional argument and is left out.
entry.end()

# A function of the focus policy: records and an enum go in, a record comes out, and a field the caller leaves out has its default.
action = FocusPolicy.focus_decision(
    FocusSettings(mode=FocusMode.SLOPPY),
    FocusEvent(kind=FocusEventKind.ENTER, held=False),
)
say(f"a pointer entering a window under sloppy focus: focus {flag(action.focus)}, raise {flag(action.raise_)}")


def type_one_more():
    before = heard
    entry.insert("!")
    say(f"typed, text is {json.dumps(entry.text, ensure_ascii=False)}, heard {before} so far")


gnoblin.bind("<Super>e", type_one_more)
say("running")
