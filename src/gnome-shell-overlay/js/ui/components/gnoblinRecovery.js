import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';

import * as Main from '../main.js';
import {openTerminal} from '../backgroundMenu.js';

// This component has no dependency on the external shell or control service.
// Session-mode lifecycle removes it before the lock screen becomes active.
export class Component {
    enable() {
        this._missingSince = GLib.get_monotonic_time();
        this._dismissed = false;
        this._panel = null;
        this._timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
            const hasLayer = global.get_window_actors().some(actor =>
                actor.is_mapped() && actor.opacity > 0 && actor.meta_window &&
                Meta.gnoblin_layer_anchor(actor.meta_window) >= 0);
            if (hasLayer) {
                this._missingSince = GLib.get_monotonic_time();
                this._dismissed = false;
                this._panel?.hide();
            } else if (!this._dismissed && !Main.layoutManager._startingUp &&
                GLib.get_monotonic_time() - this._missingSince >= 8 * GLib.USEC_PER_SEC) {
                this._show();
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    _show() {
        if (!this._panel) {
            this._panel = new St.BoxLayout({
                name: 'gnoblin-recovery',
                orientation: Clutter.Orientation.VERTICAL,
                reactive: true,
                style: 'background-color: #242424; color: #ffffff; border-radius: 16px; padding: 24px; spacing: 16px;',
            });
            this._panel.add_child(new St.Label({
                text: _('Desktop recovery'),
                style: 'font-size: 20px; font-weight: bold;',
            }));
            this._message = new St.Label({
                text: _('No desktop interface is visible. Open a terminal or Settings to repair it.'),
            });
            this._message.clutter_text.set_line_wrap(true);
            this._panel.add_child(this._message);
            const hint = new St.Label({
                text: _('These tools are also available when you right-click the desktop.'),
            });
            hint.clutter_text.set_line_wrap(true);
            this._panel.add_child(hint);
            const actions = new St.BoxLayout({style: 'spacing: 12px;'});
            for (const [label, action] of [
                [_('Open Terminal'), openTerminal],
                [_('Settings'), () => {
                    const app = Gio.DesktopAppInfo.new('org.gnome.Settings.desktop');
                    if (!app)
                        throw new Error(_('Settings is not installed.'));
                    app.launch([], global.create_app_launch_context(0, -1));
                }],
                [_('Dismiss'), () => {
                    this._dismissed = true;
                    this._panel.hide();
                }],
            ]) {
                const button = new St.Button({label, can_focus: true, style_class: 'button'});
                button.connect('clicked', () => {
                    try {
                        action();
                    } catch (error) {
                        this._message.text = error.message;
                    }
                });
                actions.add_child(button);
            }
            this._panel.add_child(actions);
            Main.layoutManager.addTopChrome(this._panel);
        }
        const monitor = Main.layoutManager.primaryMonitor;
        if (!monitor)
            return;
        this._panel.width = Math.min(540, monitor.width - 32);
        this._panel.set_position(monitor.x + (monitor.width - this._panel.width) / 2, monitor.y + 48);
        this._panel.show();
    }

    disable() {
        if (this._timer)
            GLib.source_remove(this._timer);
        this._timer = 0;
        this._panel?.destroy();
        this._panel = null;
    }
}
