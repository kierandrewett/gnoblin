import Clutter from "gi://Clutter";
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import St from "gi://St";

import * as Main from "../main.js";
import { openTerminal } from "../backgroundMenu.js";

// This component has no dependency on the external shell or control service.
// Session-mode lifecycle removes it before the lock screen becomes active.
export class Component {
    enable() {
        this._missingSince = GLib.get_monotonic_time();
        this._dismissed = false;
        this._panel = null;
        this._timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
            const hasLayer = global
                .get_window_actors()
                .some(
                    (actor) =>
                        actor.is_mapped() &&
                        actor.opacity > 0 &&
                        actor.meta_window &&
                        Meta.gnoblin_layer_anchor(actor.meta_window) >= 0,
                );
            if (hasLayer) {
                this._missingSince = GLib.get_monotonic_time();
                this._dismissed = false;
                this._panel?.hide();
            } else if (Main.devConsole?.isOpen) {
                this._panel?.hide();
            } else if (
                !this._dismissed &&
                !Main.layoutManager._startingUp &&
                GLib.get_monotonic_time() - this._missingSince >= 8 * GLib.USEC_PER_SEC
            ) {
                this._show();
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    _show() {
        if (!this._panel) {
            this._panel = new St.BoxLayout({
                name: "gnoblin-recovery",
                orientation: Clutter.Orientation.VERTICAL,
                reactive: true,
                style: "background-color: #242424; color: #ffffff; border-radius: 10px; padding: 16px; spacing: 12px;",
            });
            this._panel.add_child(
                new St.Label({
                    text: _("Desktop unavailable"),
                    style: "font-size: 18px; font-weight: bold;",
                }),
            );
            this._message = new St.Label({
                text: "",
                visible: false,
            });
            this._message.clutter_text.set_line_wrap(true);
            this._panel.add_child(this._message);
            const actions = [
                ["terminal", _("Terminal"), openTerminal],
                ["console", _("Console"), () => Main.openDevConsole()],
                [
                    "settings",
                    _("Settings"),
                    () => {
                        const app = Gio.DesktopAppInfo.new("org.gnome.Settings.desktop");
                        if (!app) throw new Error(_("Settings is not installed."));
                        app.launch([], global.create_app_launch_context(0, -1));
                    },
                ],
                [
                    "config",
                    _("Config folder"),
                    () => {
                        const config = GLib.getenv("GNOBLIN_CONFIG");
                        const directory = config
                            ? Gio.File.new_for_path(GLib.path_get_dirname(config))
                            : Gio.File.new_for_path(GLib.build_filenamev([GLib.get_user_config_dir(), "gnoblin"]));
                        if (!directory.query_exists(null)) directory.make_directory_with_parents(null);
                        Gio.AppInfo.launch_default_for_uri(
                            directory.get_uri(),
                            global.create_app_launch_context(0, -1),
                        );
                    },
                ],
            ];
            let row;
            for (const [index, [id, label, action]] of actions.entries()) {
                if (index % 2 === 0) {
                    row = new St.BoxLayout({ style: "spacing: 8px;" });
                    row.layout_manager.homogeneous = true;
                    this._panel.add_child(row);
                }
                const button = new St.Button({
                    name: `gnoblin-recovery-${id}`,
                    label,
                    can_focus: true,
                    x_expand: true,
                    style_class: "button",
                });
                button.connect("clicked", () => {
                    try {
                        this._message.hide();
                        action();
                    } catch (error) {
                        this._message.text = error.message;
                        this._message.show();
                    }
                });
                row.add_child(button);
            }
            const close = new St.Button({
                name: "gnoblin-recovery-dismiss",
                label: _("Close"),
                can_focus: true,
                style_class: "button",
            });
            close.connect("clicked", () => {
                this._dismissed = true;
                this._panel.hide();
            });
            this._panel.add_child(close);
            Main.layoutManager.addTopChrome(this._panel);
        }
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor) return;
        this._panel.width = Math.min(400, monitor.width - 32);
        this._panel.set_position(monitor.x + (monitor.width - this._panel.width) / 2, monitor.y + 48);
        this._panel.show();
    }

    disable() {
        if (this._timer) GLib.source_remove(this._timer);
        this._timer = 0;
        this._panel?.destroy();
        this._panel = null;
    }
}
