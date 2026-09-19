import Gio from "gi://Gio";
import GLib from "gi://GLib";

import * as Main from "../main.js";
import { FdoNotificationDaemonSource } from "../notificationDaemon.js";

// Gnoblin's notification surface owns org.freedesktop.Notifications. GNOME
// Shell's own notifications normally stop at MessageTray, so forward those
// notifications across the standard API boundary instead.
const NOTIFICATIONS_BUS = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH = "/org/freedesktop/Notifications";
const NOTIFICATIONS_IFACE = "org.freedesktop.Notifications";
const APP_NAME = "GNOME Shell";

Gio._promisify(Gio.DBusConnection.prototype, "call");

function isExternalNotificationSource(source) {
    // These sources are already backed by a public notification API. Forwarding
    // them again would create a GNOME Shell -> Bingux -> Shell feedback loop.
    return source instanceof FdoNotificationDaemonSource;
}

function iconName(icon) {
    if (!(icon instanceof Gio.ThemedIcon)) return "";

    return icon.iconName ?? "";
}

export class NotificationBridge {
    constructor() {
        this._entries = new Map();
        this._signalId = 0;
    }

    enable() {
        this._signalId = Gio.DBus.session.signal_subscribe(
            NOTIFICATIONS_BUS,
            NOTIFICATIONS_IFACE,
            null,
            NOTIFICATIONS_PATH,
            null,
            Gio.DBusSignalFlags.NONE,
            this._onNotificationSignal.bind(this),
        );

        Main.messageTray.connectObject(
            "source-added",
            (_, source) => this._watchSource(source),
            "source-removed",
            (_, source) => this._unwatchSource(source),
            this,
        );

        for (const source of Main.messageTray.getSources()) this._watchSource(source);
    }

    disable() {
        Main.messageTray.disconnectObject(this);

        if (this._signalId) {
            Gio.DBus.session.signal_unsubscribe(this._signalId);
            this._signalId = 0;
        }

        for (const source of Main.messageTray.getSources()) source.disconnectObject(this);

        for (const entry of this._entries.values()) {
            entry.notification.disconnectObject(this);
            this._closeRemote(entry);
        }
        this._entries.clear();
    }

    _watchSource(source) {
        if (isExternalNotificationSource(source)) return;

        source.connectObject(
            "notification-added",
            (_, notification) => this._forward(source, notification),
            "destroy",
            () => source.disconnectObject(this),
            this,
        );

        // A component can be enabled after MessageTray has already received a
        // notification, so do not rely only on future notification-added emits.
        for (const notification of source.notifications) this._forward(source, notification);
    }

    _unwatchSource(source) {
        source.disconnectObject(this);
    }

    _forward(source, notification) {
        if (this._entries.has(notification)) return;

        const entry = { source, notification, id: 0, remoteClosed: false };
        this._entries.set(notification, entry);
        notification.connectObject("destroy", () => this._onNotificationDestroyed(entry), this);

        const actions = ["default", ""];
        notification.actions.forEach((action, index) => {
            actions.push(`action-${index}`, action.label ?? "");
        });

        const hints = {
            urgency: new GLib.Variant("y", notification.urgency >= 2 ? 2 : notification.urgency),
            "desktop-entry": new GLib.Variant("s", "gnome-shell"),
        };
        const timeout = notification.resident ? 0 : -1;
        const appIcon = iconName(source.icon) || iconName(notification.gicon);

        Gio.DBus.session
            .call(
                NOTIFICATIONS_BUS,
                NOTIFICATIONS_PATH,
                NOTIFICATIONS_IFACE,
                "Notify",
                new GLib.Variant("(susssasa{sv}i)", [
                    APP_NAME,
                    0,
                    appIcon,
                    notification.title ?? "",
                    notification.body ?? "",
                    actions,
                    hints,
                    timeout,
                ]),
                null,
                Gio.DBusCallFlags.NONE,
                -1,
                null,
            )
            .then((reply) => {
                const [id] = reply.deepUnpack();
                entry.id = id;
                if (entry.remoteClosed || !this._entries.has(notification)) this._closeRemote(entry);
            })
            .catch((error) => {
                this._entries.delete(notification);
                notification.disconnectObject(this);
                log(`gnoblin-notification-bridge: forwarding failed: ${error.message}`);
            });
    }

    _onNotificationDestroyed(entry) {
        this._entries.delete(entry.notification);
        entry.notification.disconnectObject(this);
        if (!entry.remoteClosed) this._closeRemote(entry);
    }

    _closeRemote(entry) {
        if (!entry.id) return;

        entry.remoteClosed = true;
        Gio.DBus.session
            .call(
                NOTIFICATIONS_BUS,
                NOTIFICATIONS_PATH,
                NOTIFICATIONS_IFACE,
                "CloseNotification",
                new GLib.Variant("(u)", [entry.id]),
                null,
                Gio.DBusCallFlags.NONE,
                -1,
                null,
            )
            .catch(() => {});
        entry.id = 0;
    }

    _onNotificationSignal(_connection, _sender, _path, _iface, signal, parameters) {
        const [id, action] = parameters.deepUnpack();
        const entry = [...this._entries.values()].find((candidate) => candidate.id === id);
        if (!entry) return;

        if (signal === "ActionInvoked") {
            if (action === "default") {
                entry.notification.activate();
            } else if (action.startsWith("action-")) {
                const index = Number(action.slice("action-".length));
                entry.notification.actions[index]?.activate();
            }
        } else if (signal === "NotificationClosed") {
            entry.remoteClosed = true;
            entry.notification.destroy();
        }
    }
}

export default class GnoblinNotificationBridge {
    enable() {
        this._bridge = new NotificationBridge();
        this._bridge.enable();
    }

    disable() {
        this._bridge?.disable();
        this._bridge = null;
    }
}
