import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Shell from "gi://Shell";
import Clutter from "gi://Clutter";
import Cogl from "gi://Cogl";
import St from "gi://St";

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

// Read the installed GNOME cursor artwork, including its original frame delays
// and hotspot. No substitute spinner or input grab is used.
function loadStandardCursor(theme, size) {
    const roots = [
        GLib.build_filenamev([GLib.get_home_dir(), ".icons"]),
        GLib.build_filenamev([GLib.get_user_data_dir(), "icons"]),
        ...GLib.get_system_data_dirs().map((path) => GLib.build_filenamev([path, "icons"])),
    ];
    for (const name of [...new Set([theme, "Adwaita"])]) {
        for (const root of roots) {
            const file = Gio.File.new_for_path(GLib.build_filenamev([root, name, "cursors", "wait"]));
            if (!file.query_exists(null)) continue;
            const [, bytes] = file.load_contents(null);
            const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
            const word = (offset) => view.getUint32(offset, true);
            if (bytes.length < 16 || word(0) !== 0x72756358) continue;
            const entries = [];
            const count = Math.min(word(12), 4096);
            for (let i = 0; i < count; i++) {
                const offset = word(4) + i * 12;
                if (offset + 12 > bytes.length) break;
                if (word(offset) === 0xfffd0002) entries.push({ size: word(offset + 4), offset: word(offset + 8) });
            }
            if (!entries.length) continue;
            const closest = entries.reduce(
                (best, entry) => (Math.abs(entry.size - size) < Math.abs(best - size) ? entry.size : best),
                entries[0].size,
            );
            const frames = [];
            for (const entry of entries.filter((value) => value.size === closest)) {
                const offset = entry.offset;
                if (offset + 36 > bytes.length) continue;
                const width = word(offset + 16);
                const height = word(offset + 20);
                const pixels = offset + word(offset);
                if (!width || !height || width > 512 || height > 512 || pixels + width * height * 4 > bytes.length)
                    continue;
                const image = St.ImageContent.new_with_preferred_size(width, height);
                image.set_bytes(
                    global.stage.context.get_backend().get_cogl_context(),
                    new GLib.Bytes(bytes.slice(pixels, pixels + width * height * 4)),
                    Cogl.PixelFormat.BGRA_8888_PRE,
                    width,
                    height,
                    width * 4,
                );
                frames.push({
                    image,
                    width,
                    height,
                    hotX: word(offset + 24),
                    hotY: word(offset + 28),
                    delay: Math.max(16, word(offset + 32)),
                });
            }
            if (frames.length) return { frames, source: name + "/wait" };
        }
    }
    throw new Error("No standard GNOME wait cursor is installed");
}

class LaunchFeedback {
    constructor() {
        this.launches = new Map();
        this.tracker = global.backend.get_cursor_tracker();
        this.nativeCursor = typeof this.tracker.set_gnoblin_launch_cursor === "function";
        this.spinner = null;
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
        if (this.nativeCursor) return GLib.SOURCE_CONTINUE;
        const [x, y] = global.get_pointer();
        if (now >= this.nextFrame) {
            this.currentFrame = this.frames[this.frameIndex];
            this.frameIndex = (this.frameIndex + 1) % this.frames.length;
            this.spinner.set_content(this.currentFrame.image);
            this.spinner.set_size(this.currentFrame.width, this.currentFrame.height);
            this.nextFrame = now + this.currentFrame.delay * 1000;
        }
        this.spinner.set_position(x - this.currentFrame.hotX, y - this.currentFrame.hotY);
        global.stage.set_child_above_sibling(this.spinner, null);
        return GLib.SOURCE_CONTINUE;
    }

    show() {
        if (this.cursorInhibited) return;
        if (this.nativeCursor) {
            this.tracker.set_gnoblin_launch_cursor(true);
            this.cursorSource = "native-theme/wait";
        } else {
            const settings = new Gio.Settings({ schema_id: "org.gnome.desktop.interface" });
            const cursor = loadStandardCursor(settings.get_string("cursor-theme"), settings.get_int("cursor-size"));
            this.frames = cursor.frames;
            this.cursorSource = cursor.source;
            this.frameIndex = 0;
            this.nextFrame = 0;
            this.spinner = new Clutter.Actor({ reactive: false });
            global.stage.add_child(this.spinner);
            this.tracker.inhibit_cursor_visibility();
        }
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
        this.spinner?.destroy();
        this.spinner = null;
        if (this.cursorInhibited) {
            if (this.nativeCursor) this.tracker.set_gnoblin_launch_cursor(false);
            else this.tracker.uninhibit_cursor_visibility();
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
            spinnerVisible: this.nativeCursor ? this.tracker.get_gnoblin_launch_cursor() : !!this.spinner?.is_mapped(),
            cursorSource: this.cursorSource ?? null,
            position: this.spinner ? [this.spinner.x, this.spinner.y] : null,
        });
    }

    destroy() {
        this.clear();
        this.impl.unexport();
        Gio.bus_unown_name(this.nameId);
    }
}

export default function enable(api) {
    const feedback = new LaunchFeedback();
    // ScriptHost calls these disposers before reload and on session shutdown.
    api._disposers.push(() => feedback.destroy());
}
