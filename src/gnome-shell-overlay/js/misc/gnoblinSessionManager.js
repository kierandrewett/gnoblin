import Gio from "gi://Gio";
import GLib from "gi://GLib";
import * as GnomeSession from "./gnomeSession.js";

// The standalone Gnoblin session uses logind for the actions that GNOME Shell
// normally delegates to gnome-session. Keep the same async method shapes as
// GnomeSession.SessionManager so systemActions can share its policy and UI.
const DESTINATION = "org.freedesktop.login1";
const OBJECT_PATH = "/org/freedesktop/login1";
const INTERFACE = "org.freedesktop.login1.Manager";

export async function startSessionServices() {
    // Mutter's idle monitor belongs to the standalone service, but Shell must
    // run its usual idle transition so user activity cancels the fade and the
    // existing lock-delay and lock-enabled settings retain their meaning.
    const { screenShield } = await import("../ui/main.js");
    if (!screenShield) throw new Error("Cannot start idle policy before the screen shield");
    Gio.DBus.session.signal_subscribe(
        "org.freedesktop.ScreenSaver",
        "org.gnoblin.SessionIdle",
        "Idle",
        "/org/gnoblin/SessionIdle",
        null,
        Gio.DBusSignalFlags.NONE,
        () => screenShield._onStatusChanged(GnomeSession.PresenceStatus.IDLE),
    );

    // Mutter discovers the Wayland socket after the login wrapper has copied
    // its environment. Give both D-Bus and systemd user services the display
    // address before XDG autostart begins.
    const display = GLib.getenv("WAYLAND_DISPLAY");
    if (!display) throw new Error("Cannot start session services without a Wayland display");

    const environment = {
        WAYLAND_DISPLAY: display,
        DISPLAY: "",
        XAUTHORITY: "",
        GNOME_SETUP_DISPLAY: "",
    };
    for (const name of ["DISPLAY", "XAUTHORITY"]) {
        const value = GLib.getenv(name);
        if (value) environment[name] = value;
    }
    await Gio.DBus.session.call(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "UpdateActivationEnvironment",
        new GLib.Variant("(a{ss})", [environment]),
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
    );
    await Gio.DBus.session.call(
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
        "UnsetEnvironment",
        new GLib.Variant("(as)", [Object.keys(environment).filter((name) => !environment[name])]),
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
    );
    await Gio.DBus.session.call(
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
        "SetEnvironment",
        new GLib.Variant("(as)", [
            Object.entries(environment)
                .filter(([, value]) => value)
                .map(([name, value]) => `${name}=${value}`),
        ]),
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
    );

    await Gio.DBus.session.call(
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
        "StartUnit",
        new GLib.Variant("(ss)", ["gnoblin-session.target", "replace"]),
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
    );
}

async function call(method, parameters = null) {
    const reply = await Gio.DBus.system.call(
        DESTINATION,
        OBJECT_PATH,
        INTERFACE,
        method,
        parameters,
        null,
        Gio.DBusCallFlags.NONE,
        -1,
        null,
    );
    return reply.deepUnpack();
}

async function availability(method) {
    const [answer] = await call(method);
    // GNOME SessionManager's action values are 0=unavailable, 2=challenge,
    // 3=available. logind answers with "na", "no", "challenge", or "yes".
    const value = answer === "yes" ? 3 : answer === "challenge" ? 2 : 0;
    return [value];
}

export class SessionManager {
    CanShutdownAsync() {
        return availability("CanPowerOff");
    }

    CanRebootAsync() {
        return availability("CanReboot");
    }

    CanSuspendAsync() {
        return availability("CanSuspend");
    }

    LogoutAsync() {
        const sessionId = GLib.getenv("XDG_SESSION_ID");
        if (!sessionId) return Promise.reject(new Error("Cannot log out without XDG_SESSION_ID"));
        // Let the login manager close the session when Shell exits. Killing the
        // logind scope first also kills gnoblin-session before it can stop the
        // graphical targets and clear the user manager's display environment.
        global.context.terminate();
        return Promise.resolve();
    }

    ShutdownAsync() {
        return call("PowerOff", new GLib.Variant("(b)", [true]));
    }

    RebootAsync() {
        return call("Reboot", new GLib.Variant("(b)", [true]));
    }

    SuspendAsync() {
        return call("Suspend", new GLib.Variant("(b)", [true]));
    }
}
