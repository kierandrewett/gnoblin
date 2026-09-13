import Adw from "gi://Adw?version=1";
import Gtk from "gi://Gtk?version=4.0";
import Gdk from "gi://Gdk?version=4.0";
import Gio from "gi://Gio";
import GLib from "gi://GLib";

const app = new Adw.Application({ application_id: "org.gnoblin.CornerFixture" });
app.connect("activate", () => {
    const css = new Gtk.CssProvider();
    css.load_from_string("window.background { background: white; color: black; } headerbar { background: white; }");
    const colourFile = GLib.getenv("GNOBLIN_CSD_COLOUR_FILE");
    if (colourFile) {
        let previous = "";
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {
            const [, bytes] = Gio.File.new_for_path(colourFile).load_contents(null);
            const colour = new TextDecoder().decode(bytes).trim();
            if (colour !== previous) {
                css.load_from_string(`window.background, headerbar { background: ${colour}; color: white; }`);
                previous = colour;
            }
            return GLib.SOURCE_CONTINUE;
        });
    }
    Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION);
    const window = new Adw.ApplicationWindow({
        application: app,
        title: "Corner fixture",
        default_width: 320,
        default_height: 240,
    });
    const view = new Adw.ToolbarView();
    view.add_top_bar(new Adw.HeaderBar());
    view.set_content(new Gtk.Box());
    window.set_content(view);
    window.present();
});
app.run([]);
