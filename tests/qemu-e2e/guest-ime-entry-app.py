"""A GTK4 window with one text entry. Writes the entry text to /tmp/ime-entry.txt on every change."""
import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk

Gtk.init()
window = Gtk.Window(title="ime-test")
window.set_default_size(500, 120)
entry = Gtk.Entry()
window.set_child(entry)
window.present()
entry.grab_focus()


def changed(widget):
    open("/tmp/ime-entry.txt", "w").write(widget.get_text())


entry.connect("changed", changed)
loop = GLib.MainLoop()
GLib.timeout_add_seconds(40, lambda: (window.destroy(), loop.quit()) and False)
loop.run()
