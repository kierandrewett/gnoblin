"""Open a light parent window, then a dark modal dialog on top of it after a short delay."""
import sys

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk  # noqa: E402

app = Gtk.Application(application_id="org.gnoblin.DialogTest")


def activate(application):
    provider = Gtk.CssProvider()
    provider.load_from_string("window.dlg { background: #202020; }")
    Gtk.StyleContext.add_provider_for_display(
        parent_display(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
    )
    parent = Gtk.ApplicationWindow(application=application, title="dialog-parent")
    parent.set_default_size(500, 360)
    parent.present()

    def show_dialog():
        dialog = Gtk.Window(title="dialog-child", transient_for=parent, modal=True)
        dialog.add_css_class("dlg")
        dialog.set_default_size(300, 200)
        dialog.set_child(Gtk.Label(label="dialog"))
        dialog.present()
        print("dialog presented", file=sys.stderr, flush=True)
        return False

    GLib.timeout_add(4000, show_dialog)
    GLib.timeout_add(14000, application.quit)


def parent_display():
    from gi.repository import Gdk

    return Gdk.Display.get_default()


app.connect("activate", activate)
app.run([])
