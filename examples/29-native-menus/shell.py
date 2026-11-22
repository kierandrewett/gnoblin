"""Example 29 in Python: the window menu of libgnoblin-std, through the classes generated from its catalog.

    gnoblin ./shell.py

Starts the window menu, gives it three entries of this shell's own (a ``MenuProvider``) that log what they run, marks the windows of the app
``secret`` secure (their card is secure too), and logs what the menu does. Every line starts with ``menus:``, as the Rust shell of the same
example spells it.
"""

from gnoblin import gnoblin as session
from gnoblin.objects.model import Gnoblin
from gnoblin.objects.std import MenuEntry, MenuProvider, WindowMenus


def say(text):
    session.log(f"menus: {text}")


def entry_for(label):
    entry = MenuEntry(label)

    def run(client):
        say(f"ran {label} on window {client}")

    entry.run = run
    return entry


class Entries(MenuProvider):
    def items(self, client):
        return [entry_for(label) for label in ("Alpha", "Beta", "Gamma")]


menus = WindowMenus.get()
menus.provider = Entries()


@Gnoblin.get().clients.on("added")
def joined(client, collection):
    if client.app_id == "secret":
        client.secure = True


@menus.on("current_changed")
def opened(menu):
    say(f"a card opened for window {menu.client}, secure: {str(menu.panel.secure).lower()}")

    @menu.nav.on("selected_changed")
    def moved(row):
        say(f"the keyboard is on {row.label}")

    @menu.on("closed")
    def closed():
        say("the card closed")


menus.start()
say("running")
