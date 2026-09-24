import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Shell from "gi://Shell";

const NAME = "org.gnoblin.LaunchFeedback";
const PATH = "/org/gnoblin/LaunchFeedback";
const IFACE = `<node><interface name="${NAME}">
    <method name="Begin"><arg type="s" direction="in" name="token"/><arg type="s" direction="in" name="application"/><arg type="u" direction="in" name="timeout"/></method>
    <method name="End"><arg type="s" direction="in" name="token"/></method>
    <method name="GetState"><arg type="s" direction="out" name="state"/></method>
</interface></node>`;

function normalise(value) {
    return String(value ?? "")
        .toLowerCase()
        .replace(/\.desktop$/, "")
        .trim();
}

export class LaunchFeedback {
    constructor() {
        this.launches = new Map();
        this.tracker = global.backend.get_cursor_tracker();
        this.nativeCursor = typeof this.tracker.set_gnoblin_launch_cursor === "function";
        this.tick = 0;
        this.lastWindowCheck = 0;
        this.cursorInhibited = false;
        this.impl = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this.impl.export(Gio.DBus.session, PATH);
        this.nameId = Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, null, null, () =>
            this.clear(),
        );
    }

    Begin(token, application, timeout) {
        if (!token || token.length > 128 || application.length > 512)
            throw new Error("Invalid launch feedback request");
        if (this.launches.size >= 64 && !this.launches.has(token)) throw new Error("Too many pending launches");
        this.launches.set(token, {
            application: normalise(application),
            deadline: GLib.get_monotonic_time() + Math.min(10000, Math.max(100, timeout)) * 1000,
            windows: new Set(global.get_window_actors().map((actor) => actor.meta_window)),
            focus: global.display.focus_window,
        });
        try {
            this.show();
        } catch (error) {
            this.clear();
            throw error;
        }
    }

    End(token) {
        this.launches.delete(token);
        if (this.launches.size === 0) this.hide();
    }

    matches(window, hint) {
        if (!hint || window.skip_taskbar) return false;
        const app = Shell.WindowTracker.get_default().get_window_app(window);
        return [
            window.get_gtk_application_id(),
            window.get_wm_class(),
            window.get_wm_class_instance(),
            app?.get_id(),
            app?.get_name(),
        ].some((value) => normalise(value) === hint);
    }

    update() {
        const now = GLib.get_monotonic_time();
        const checkWindows = now - this.lastWindowCheck >= 100000;
        const actors = checkWindows ? global.get_window_actors() : [];
        if (checkWindows) this.lastWindowCheck = now;
        for (const [token, launch] of this.launches) {
            const appeared = actors.some(
                (actor) =>
                    actor.is_mapped() &&
                    (!launch.windows.has(actor.meta_window) ||
                        (global.display.focus_window === actor.meta_window && launch.focus !== actor.meta_window)) &&
                    this.matches(actor.meta_window, launch.application),
            );
            if (now >= launch.deadline || appeared) this.launches.delete(token);
        }
        if (this.launches.size === 0) {
            this.hide();
            return GLib.SOURCE_REMOVE;
        }
        return GLib.SOURCE_CONTINUE;
    }

    show() {
        if (this.cursorInhibited) return;
        if (!this.nativeCursor) throw new Error("Mutter was built without Gnoblin's Hyprcursor support");
        this.tracker.set_gnoblin_launch_cursor(true);
        this.cursorSource = "hyprcursor/wait";
        this.cursorInhibited = true;
        this.update();
        this.tick = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {
            try {
                return this.update();
            } catch (error) {
                this.clear();
                logError(error, "gnoblin launch feedback failed");
                return GLib.SOURCE_REMOVE;
            }
        });
    }

    hide() {
        if (this.tick) {
            GLib.Source.remove(this.tick);
            this.tick = 0;
        }
        if (this.cursorInhibited) {
            this.tracker.set_gnoblin_launch_cursor(false);
            this.cursorInhibited = false;
        }
    }

    clear() {
        this.launches.clear();
        this.hide();
    }

    GetState() {
        return JSON.stringify({
            busy: this.cursorInhibited,
            pending: this.launches.size,
            nativeCursor: this.nativeCursor,
            pointerVisible: this.tracker.get_pointer_visible(),
            spinnerVisible: this.nativeCursor && this.tracker.get_gnoblin_launch_cursor(),
            cursorSource: this.cursorSource ?? null,
            position: null,
        });
    }

    destroy() {
        this.clear();
        this.impl.unexport();
        Gio.bus_unown_name(this.nameId);
    }
}
