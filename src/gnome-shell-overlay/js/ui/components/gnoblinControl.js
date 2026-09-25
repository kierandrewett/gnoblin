import * as Permissions from "./gnoblinPermissions.js";
// Gnoblin Core — the org.gnoblin.* control protocol.
//
// This is first-class gnoblin source, not an extension: it's copied verbatim
// into gnome-shell's js/ui/components/ as an overlay (see the manifest next to
// this file) and loaded as a session-mode *component* — the same mechanism
// gnome-shell uses for networkAgent/polkitAgent/keyring. It is enabled purely by
// listing "gnoblinControl" in the `gnoblin` session mode's components, so the
// only upstream edit is a single <file> line in the JS gresource. gnoblin stays
// "just GNOME + mutter"; this component is the one intentional addition, and it
// gets enable()/disable() lifecycle for free (which the Wayland soft-reload uses).
//
// The control component: the org.gnoblin.Shell bus name, health/version,
// runtime feature toggles (osd + per-type, screenshot, notifications), and
// the Wayland soft-reload all hang off this same object.

import {
    Autostart,
    ConfigFile,
    FEATURE_KEYS,
    Shortcuts,
    CommandShortcuts,
    ShortcutInput,
    applyWindowPreferences,
    applyCompositorPreferences,
    applyInputPreferences,
} from "./gnoblinConfig.js";
import { WindowRules } from "./gnoblinRules.js";
import * as Workspaces from "./gnoblinWorkspaces.js";
import Gio from "gi://Gio";
import Clutter from "gi://Clutter";
import GLib from "gi://GLib";
import Gtk from "gi://Gtk?version=4.0";
import Meta from "gi://Meta";
import Shell from "gi://Shell";
import St from "gi://St";
import * as Keyboard from "../status/keyboard.js";
import * as Location from "../status/location.js";
import * as Main from "../main.js";
import * as Volume from "../status/volume.js";
import * as Config from "../../misc/config.js";
import { CompositorBridge } from "./gnoblinBridge/compositor-bridge.js";
import { LaunchFeedback } from "./gnoblinLaunchFeedback.js";
import { MutterEventForwarder } from "./gnoblinMutterEvents.js";

const BUS_NAME = "org.gnoblin.Shell";
const OBJECT_PATH = "/org/gnoblin/Shell";
const CLUTTER_EVENT_NAMES = new Map(
    Object.entries(Clutter.EventType)
        .filter(([key]) => Number.isNaN(Number(key)))
        .map(([key, value]) => [value, key]),
);
const SCHEMA_ID = "org.gnoblin.shell";
const DISABLED_KEY = "disabled-features";
const PORTAL_GRANT_KINDS = ["screen-cast", "remote-desktop"];
const PORTAL_GRANT_FILE_PATTERN = /^[0-9a-f]{64}\.grant$/;
const PORTAL_GRANT_GROUP = "Grant";
const PORTAL_GRANT_VERSION = 1;
const SUPER_RELEASE_PROTOCOL_VERSION = 1;
const OSD_REQUEST_PROTOCOL_VERSION = 2;
const TRIM_INTERVAL_SECONDS = 300;
const BUILT_IN_SERVICE_SCRIPTS = new Set(["compositor-bridge.js", "input-sources.js", "launch-feedback.js"]);

// The live ScriptHost, so the module-level softReload() can re-run scripts.
let activeScriptHost = null;
let activeConfig = null;
const autostart = new Autostart();

// Identity of the stylesheet set the current St theme was built from. Used to
// skip Main.loadTheme() on soft reload when no stylesheet changed: every theme
// swap permanently leaks the old parsed theme (~4 MB, upstream St/GJS bug —
// the replaced StTheme wrapper survives GC with refcount 1), so reloads that
// only touch code must not pay that cost. null means "unknown, reload".
let lastStylesheetDigest = null;

function stylesheetDigest() {
    const theme = St.ThemeContext.get_for_stage(global.stage).get_theme();
    if (!theme) return null;
    const files = [theme.default_stylesheet, theme.application_stylesheet, ...theme.get_custom_stylesheets()].filter(
        (f) => f !== null,
    );
    const parts = [];
    for (const f of files) {
        let part = f.get_uri();
        try {
            const info = f.query_info(
                "standard::size,time::modified,time::modified-usec",
                Gio.FileQueryInfoFlags.NONE,
                null,
            );
            part += `:${info.get_size()}:${info.get_attribute_uint64("time::modified")}:${info.get_attribute_uint32("time::modified-usec")}`;
        } catch {
            part += ":unreadable";
        }
        parts.push(part);
    }
    return parts.sort().join("|");
}

// Process-wide monotonic counter for the script import cache-bust. Module-level
// (not per-host) so a disable→re-enable in the same process still re-imports fresh
// code instead of reusing the cached module.
let scriptImportSeq = 0;

// Return a GIcon's theme names without trusting the object supplied by the caller.
function osdIconNames(icon) {
    try {
        const names = icon?.get_names?.();
        if (Array.isArray(names)) return names.filter((name) => typeof name === "string");
        const name = icon?.to_string?.();
        return typeof name === "string" ? [name] : [];
    } catch {
        return [];
    }
}

// Convert arbitrary values to valid D-Bus strings. GJS strings can contain a
// NUL or an unpaired UTF-16 surrogate, neither of which is valid on the wire.
function serialiseOsdString(value) {
    let string;
    try {
        string = typeof value === "string" ? value : String(value ?? "");
    } catch {
        return "";
    }

    let result = "";
    for (let i = 0; i < string.length; i++) {
        const code = string.charCodeAt(i);
        if (code === 0) {
            result += "\ufffd";
        } else if (code >= 0xd800 && code <= 0xdbff) {
            const next = string.charCodeAt(i + 1);
            if (next >= 0xdc00 && next <= 0xdfff) result += string[i] + string[++i];
            else result += "\ufffd";
        } else if (code >= 0xdc00 && code <= 0xdfff) {
            result += "\ufffd";
        } else {
            result += string[i];
        }
    }
    return result;
}

function serialiseOsdIcon(icon) {
    if (typeof icon === "string") return serialiseOsdString(icon);

    return serialiseOsdString(osdIconNames(icon)[0] ?? "");
}

function serialiseOsdMonitorIndex(value) {
    let index;
    try {
        index = Number(value);
    } catch {
        return 0;
    }

    if (!Number.isFinite(index)) return 0;
    return Math.max(-0x80000000, Math.min(0x7fffffff, Math.trunc(index)));
}

function serialiseOsdLevel(value) {
    let level;
    try {
        level = Number(value);
    } catch {
        return 0;
    }
    return Number.isFinite(level) ? level : 0;
}

// Resolve the physical output names for the logical monitor that GNOME Shell
// uses as an OSD index. Mutter exposes the same connector names through
// Wayland, so an external layer-shell client can select the correct screen
// without relying on either side's monitor enumeration order.
function osdOutputNamesForMonitorIndex(monitorIndex) {
    const index = serialiseOsdMonitorIndex(monitorIndex);
    if (index < 0) return [];

    try {
        const logicalMonitors = global.backend.get_monitor_manager().get_logical_monitors();
        const logicalMonitor = logicalMonitors.find((monitor) => monitor.get_number() === index);
        if (!logicalMonitor) return [];

        const outputNames = [];
        for (const monitor of logicalMonitor.get_monitors()) {
            if (!monitor.is_active()) continue;

            const connector = serialiseOsdString(monitor.get_connector());
            if (connector.length > 0 && !outputNames.includes(connector)) outputNames.push(connector);
        }

        return outputNames;
    } catch {
        return [];
    }
}

// Removed UI features remain readable so existing external-shell configuration
// can keep setting them to false. They cannot recreate GNOME widgets.
const REMOVED_FEATURES = new Set([
    "osd",
    "osd-volume",
    "osd-microphone",
    "osd-brightness",
    "osd-keyboard-brightness",
    "osd-pad",
    "screenshot",
]);
const FEATURES = {
    // The separate notification daemon reads this setting before owning the bus.
    notifications: { summary: "Own org.freedesktop.Notifications", apply() {} },
    "input-source-switcher": { summary: "Native GNOME keyboard-layout switcher", apply() {} },
    wallpaper: {
        summary: "Show the GNOME desktop background",
        apply(enabled) {
            Main.layoutManager?.setGnoblinWallpaperEnabled(enabled);
        },
    },
};

// Console edits share the config owner and validation used by file reloads.
export function consoleConfig() {
    if (!activeConfig) throw new Error("Gnoblin configuration is not active");
    return activeConfig;
}

// Soft, in-process reload — the Wayland-safe answer to "reload the shell without
// logging out". mutter/Wayland is NEVER torn down, so windows and the external
// chrome survive. We reload only the mutable JS layer: the shell theme/CSS and
// script modules. Core services, including the compositor bridge,
// keep running across this reload.
// gnoblin keeps almost nothing else in-process — the chrome lives in a separate
// layer-shell client — so this covers the practical need. A true process re-exec
// on Wayland cannot preserve clients (no handoff protocol), which is exactly why
// this is a soft reload and not global.reexec_self().
export async function softReload(reason = "manual") {
    console.log(`gnoblin: soft-reload (${reason}) — reloading theme and scripts in-process`);
    const failures = [];
    try {
        activeConfig?.reload();
    } catch (e) {
        failures.push("config");
        console.warn(`gnoblin: config reload failed: ${e.message}`);
    }

    const digest = stylesheetDigest();
    if (digest !== null && digest === lastStylesheetDigest) {
        console.log("gnoblin: soft-reload: stylesheets unchanged, keeping current theme");
    } else {
        try {
            Main.loadTheme();
            lastStylesheetDigest = stylesheetDigest();
            // The dropped parsed CSS is freed but stays resident in glibc's
            // arenas (~4 MB per swap, measured); hand it back to the kernel.
            Shell.util_trim_memory();
        } catch (e) {
            failures.push("theme");
            logError(e, "gnoblin: soft-reload loadTheme failed");
        }
    }

    try {
        await activeScriptHost?.reload();
    } catch (e) {
        failures.push("scripts");
        logError(e, "gnoblin: soft-reload scripts failed");
    }

    if (failures.length > 0) throw new Error(`soft reload failed: ${failures.join(", ")}`);

    console.log(`gnoblin: soft-reload (${reason}) complete`);
}

// A tiny event bus scripts subscribe to via api.on(). Kept minimal on purpose —
// a few high-signal compositor events, wired to mutter/display signals.
class EventBus {
    constructor() {
        this._subs = new Map();
        this._handlers = [];
    }

    connectSources() {
        const display = global.display;
        this._handlers.push([display, display.connect("window-created", (_d, win) => this.emit("window-opened", win))]);
        const wm = global.workspace_manager;
        this._handlers.push([
            wm,
            wm.connect("active-workspace-changed", () =>
                this.emit("workspace-changed", wm.get_active_workspace_index()),
            ),
        ]);
        // GNOME Shell's overview is a shell-owned interaction, distinct from
        // Mutter's workspace and display signals.
        for (const signal of ["showing", "shown", "hiding", "hidden"])
            this._handlers.push([Main.overview, Main.overview.connect(signal, () => this.emit(`overview.${signal}`))]);
    }

    subscribe(event, cb) {
        if (!this._subs.has(event)) this._subs.set(event, new Set());
        this._subs.get(event).add(cb);
        return () => this._subs.get(event)?.delete(cb);
    }

    emit(event, ...args) {
        const payload = {};
        const first = args[0];
        if (typeof first === "number") payload.index = first;
        else if (first?.get_gtk_application_id) {
            payload.app_id = first.get_gtk_application_id() || "";
            payload.wm_class = first.get_wm_class() || "";
            payload.title = first.get_title() || "";
        }
        if (event.startsWith("overview.")) activeConfig?.dispatchEvent(`gnome.shell.${event}`, payload);
        for (const cb of this._subs.get(event) ?? []) {
            try {
                const result = cb(...args);
                if (result && typeof result.then === "function")
                    Promise.resolve(result).catch((e) =>
                        logError(e, `gnoblin-script: async handler for '${event}' threw`),
                    );
            } catch (e) {
                logError(e, `gnoblin-script: handler for '${event}' threw`);
            }
        }
    }

    destroy() {
        for (const [obj, id] of this._handlers) {
            try {
                obj.disconnect(id);
            } catch {
                /* already gone */
            }
        }
        this._handlers = [];
        this._subs.clear();
    }
}

// Loads GJS integrations and personal scripts from XDG data/config script
// directories. Each entry default-exports (api) => {...}; package integrations
// can register namespaced bridge operations without adding their policy to the
// built-in compositor service.
class ScriptHost {
    constructor(control, bus) {
        this._control = control;
        this._bus = bus;
        this._dir = GLib.build_filenamev([GLib.get_user_config_dir(), "gnoblin", "scripts"]);
        this._loaded = [];
        this._apiDisposers = new WeakMap();
        this._generation = 0; // bumped on every load/unload to drop stale in-flight imports
        this._destroyed = false;
    }

    _armRecovery() {
        if (this._recoveryChecked) return;
        this._recoveryChecked = true;
        const key = GLib.compute_checksum_for_string(GLib.ChecksumType.SHA256, this._dir, -1);
        const dir = GLib.build_filenamev([GLib.get_user_state_dir(), "gnoblin", "script-sessions", key]);
        GLib.mkdir_with_parents(dir, 0o700);
        const pid = GLib.file_read_link("/proc/self");
        const directory = Gio.File.new_for_path(dir);
        this._quarantine = directory.get_child("quarantined");
        this._safeMode = this._quarantine.query_exists(null);
        const entries = directory.enumerate_children("standard::name", Gio.FileQueryInfoFlags.NONE, null);
        let entry;
        while ((entry = entries.next_file(null)) !== null) {
            const name = entry.get_name();
            if (!/^\d+\.running$/.test(name)) continue;
            const previousPid = name.slice(0, -8);
            if (previousPid !== pid && !GLib.file_test(`/proc/${previousPid}`, GLib.FileTest.EXISTS)) {
                this._safeMode = true;
                this._quarantine.replace_contents(
                    "Unclean script session; explicit retry required",
                    null,
                    false,
                    Gio.FileCreateFlags.PRIVATE,
                    null,
                );
                directory.get_child(name).delete(null);
            }
        }
        entries.close(null);
        this._recoveryMarker = directory.get_child(`${pid}.running`);
        this._recoveryMarker.replace_contents("User scripts active", null, false, Gio.FileCreateFlags.PRIVATE, null);
        this._recoveryShutdown = global.connect("shutdown", () => this._clearRecovery());
    }

    _clearRecovery() {
        try {
            if (this._recoveryMarker?.query_exists(null)) this._recoveryMarker.delete(null);
        } catch (error) {
            console.warn(`gnoblin-script: cannot clear recovery marker: ${error.message}`);
        }
    }

    _api(name) {
        const disposers = [];
        const addCleanup = (callback) => {
            if (typeof callback !== "function") throw new TypeError("cleanup must be a function");
            let active = true;
            const dispose = () => {
                if (!active) return;
                active = false;
                const index = disposers.indexOf(dispose);
                if (index >= 0) disposers.splice(index, 1);
                callback();
            };
            disposers.push(dispose);
            return dispose;
        };
        const api = {
            log: (...a) => console.log(`gnoblin-script[${name}]:`, ...a),
            version: () => this._control.GetVersion(),
            getFeature: (id) => this._control.GetFeature(id),
            setFeature: (id, on) => this._control.SetFeature(id, on),
            reloadShell: () => softReload("script"),
            addCleanup,
            handleCompositorOperation: (operation, handler) => {
                const bridge = this._control._compositorBridge;
                if (!bridge) throw new Error("The compositor bridge is unavailable");
                return addCleanup(bridge.registerScriptOperation(operation, handler));
            },
            onCompositorClientClosed: (handler) => {
                const bridge = this._control._compositorBridge;
                if (!bridge) throw new Error("The compositor bridge is unavailable");
                return addCleanup(bridge.onClientClosed(handler));
            },
            on: (event, cb) => {
                const d = this._bus.subscribe(event, cb);
                return addCleanup(d);
            },
        };
        this._apiDisposers.set(api, disposers);
        return api;
    }

    _scriptPaths() {
        // Package integrations live in share/gnoblin/scripts; installed and
        // personal scripts live in the user's data and config directories.
        // Higher-precedence locations replace a same-named lower-precedence
        // script, so a user can disable or override a package integration.
        const directories = [
            ...GLib.get_system_data_dirs().slice().reverse(),
            GLib.get_user_data_dir(),
            GLib.get_user_config_dir(),
        ].map((base) => GLib.build_filenamev([base, "gnoblin", "scripts"]));
        const scripts = new Map();
        for (const directory of directories) {
            const dir = Gio.File.new_for_path(directory);
            if (!dir.query_exists(null)) continue;
            let entries;
            try {
                entries = dir.enumerate_children("standard::name", Gio.FileQueryInfoFlags.NONE, null);
            } catch {
                continue;
            }
            let entry;
            while ((entry = entries.next_file(null)) !== null) {
                const name = entry.get_name();
                if (BUILT_IN_SERVICE_SCRIPTS.has(name)) {
                    console.warn(`gnoblin-script: ignoring ${name}; its service is built in`);
                    continue;
                }
                if (name.endsWith(".js")) scripts.set(name, GLib.build_filenamev([directory, name]));
            }
            entries.close(null);
        }
        return [...scripts].sort(([a], [b]) => a.localeCompare(b));
    }

    _disposeApi(api) {
        // Undo stacked wrappers in reverse installation order. Drain first so
        // a failed/async load and a concurrent unload cannot dispose twice.
        const disposers = this._apiDisposers.get(api) ?? [];
        this._apiDisposers.delete(api);
        for (const d of disposers.splice(0).reverse()) {
            try {
                d();
            } catch {
                /* ignore */
            }
        }
    }

    async load() {
        if (this._destroyed) return;
        this._armRecovery();
        if (this._safeMode) {
            console.warn(
                "gnoblin-script: previous session ended uncleanly; scripts paused for recovery. Use gnoblinctl reload to retry explicitly.",
            );
            return;
        }

        const gen = ++this._generation;
        const failures = [];
        for (const [name, path] of this._scriptPaths()) {
            // First import in the process uses the plain URI; every later (re)load
            // cache-busts so code edits take effect. Module-level seq so a re-enable
            // in the same process is still fresh.
            scriptImportSeq++;
            const fileUri = Gio.File.new_for_path(path).get_uri();
            const uri = scriptImportSeq > 1 ? `${fileUri}?gnoblinScript=${scriptImportSeq}` : fileUri;

            let mod;
            try {
                mod = await import(uri);
            } catch (e) {
                failures.push(name);
                logError(e, `gnoblin-script: importing ${name} failed`);
                this._control._config?.dispatchEvent("gnoblin.scripts.load_failed", { script: name, error: e.message });
                continue;
            }

            // Drop stale in-flight imports: a newer load/unload happened, or the
            // host was destroyed, while this import was pending.
            if (this._destroyed || gen !== this._generation) return;
            if (typeof mod.default !== "function") {
                failures.push(name);
                console.warn(`gnoblin-script: ${name} has no default-exported function`);
                continue;
            }

            const api = this._api(name);
            try {
                await mod.default(api);
                if (this._destroyed || gen !== this._generation) {
                    this._disposeApi(api);
                    return;
                }
                this._loaded.push({ name, api });
                console.log(`gnoblin-script: loaded ${name}`);
            } catch (e) {
                // The script may have subscribed via api.on() before throwing —
                // dispose those so a failed load doesn't leak handlers.
                this._disposeApi(api);
                failures.push(name);
                logError(e, `gnoblin-script: ${name} threw on load`);
                this._control._config?.dispatchEvent("gnoblin.scripts.load_failed", { script: name, error: e.message });
            }
        }

        this._control._config?.dispatchEvent("gnoblin.scripts.loaded", { scripts: this.list().join(",") });
        if (failures.length > 0) throw new Error(`failed to load scripts: ${failures.join(", ")}`);
    }

    unload() {
        // Invalidate any in-flight imports from the current generation.
        this._generation++;
        for (const { api } of this._loaded.splice(0).reverse()) this._disposeApi(api);
        this._loaded = [];
    }

    async reload() {
        this.unload();
        if (this._quarantine?.query_exists(null)) this._quarantine.delete(null);
        this._safeMode = false;
        await this.load();
    }

    destroy() {
        this._destroyed = true;
        this.unload();
        this._clearRecovery();
        if (this._recoveryShutdown) global.disconnect(this._recoveryShutdown);
    }

    list() {
        return this._loaded.map((s) => s.name);
    }
}

// The wire contract. Deliberately small for now; grows with Phases 2.5/3.
const IFACE = `
<node>
  <interface name="org.gnoblin.Shell">
    <!-- Liveness check: returns "pong". -->
    <method name="Ping">
      <arg type="s" direction="out" name="pong"/>
    </method>
    <!-- Shell + protocol version string, e.g. "51.0-gnoblin". -->
    <method name="GetVersion">
      <arg type="s" direction="out" name="version"/>
    </method>
    <!-- Temporarily grab keyboard input and return one GTK accelerator. -->
    <method name="CaptureAccelerator">
      <arg type="u" direction="in" name="waitSeconds"/>
      <arg type="s" direction="out" name="accelerator"/>
    </method>
    <!-- Emitted after Super is released with no other input. The payload is
         [protocol version, monotonic timestamp in microseconds]. -->
    <signal name="SuperReleased">
      <arg type="u" name="protocolVersion"/>
      <arg type="t" name="monotonicUsec"/>
    </signal>
    <!-- OSD requests are always forwarded to external chrome. Payload: [protocol version,
         monitor index, icon name, label, level, maximum level, physical output
         connector names]. The connector names identify every physical output in
         the logical monitor that owns monitorIndex. -->
    <signal name="OsdRequested">
      <arg type="u" name="protocolVersion"/>
      <arg type="i" name="monitorIndex"/>
      <arg type="s" name="icon"/>
      <arg type="s" name="label"/>
      <arg type="d" name="level"/>
      <arg type="d" name="maxLevel"/>
      <arg type="as" name="outputNames"/>
    </signal>
    <!-- Soft in-process reload (theme + scripts). Wayland-safe: keeps windows. -->
    <method name="Reload"/>
    <method name="ReloadConfig"/>
    <!-- Keyboard source state comes from GNOME Shell's InputSourceManager. -->
    <!-- Sources are [type, id, short label, full display name]. -->
    <method name="ListInputSources">
      <arg type="a(ssss)" direction="out" name="sources"/>
    </method>
    <method name="GetCurrentInputSource">
      <arg type="s" direction="out" name="type"/>
      <arg type="s" direction="out" name="id"/>
      <arg type="s" direction="out" name="shortName"/>
      <arg type="s" direction="out" name="displayName"/>
    </method>
    <method name="SetInputSource">
      <arg type="s" direction="in" name="type"/>
      <arg type="s" direction="in" name="id"/>
    </method>
    <signal name="InputSourceChanged">
      <arg type="s" name="type"/>
      <arg type="s" name="id"/>
      <arg type="s" name="shortName"/>
      <arg type="s" name="displayName"/>
    </signal>
    <!-- True tells clients to refresh ListInputSources. -->
    <signal name="InputSourcesChanged">
      <arg type="b" name="changed"/>
    </signal>
    <!-- Current screen sharing, microphone recording, and location use state. -->
    <method name="GetPrivacyState">
      <arg type="b" direction="out" name="screenSharing"/>
      <arg type="b" direction="out" name="microphoneInUse"/>
      <arg type="b" direction="out" name="locationInUse"/>
    </method>
    <signal name="PrivacyStateChanged">
      <arg type="b" name="screenSharing"/>
      <arg type="b" name="microphoneInUse"/>
      <arg type="b" name="locationInUse"/>
    </signal>
    <!-- Loaded package integrations and personal scripts. -->
    <method name="ListScripts">
      <arg type="as" direction="out" name="scripts"/>
    </method>
    <!-- Typed ScreenCast/RemoteDesktop grants. Each tuple is:
         [opaque id, portal kind, namespaced requester identity,
          remote device mask, clipboard enabled, screen streams enabled]. -->
    <method name="GetPermissions">
      <arg type="s" direction="out" name="policy"/>
    </method>
    <method name="CheckPermission">
      <arg type="s" direction="in" name="capability"/>
      <arg type="s" direction="in" name="identity"/>
      <arg type="s" direction="out" name="level"/>
      <arg type="s" direction="out" name="rule"/>
      <arg type="as" direction="out" name="monitors"/>
      <arg type="u" direction="out" name="devices"/>
      <arg type="b" direction="out" name="clipboard"/>
    </method>
    <method name="ListPortalGrants">
      <arg type="a(sssubb)" direction="out" name="grants"/>
    </method>
    <method name="RevokePortalGrant">
      <arg type="s" direction="in" name="portal"/>
      <arg type="s" direction="in" name="id"/>
    </method>
    <!-- Feature toggles: gate gnome-shell subsystems on/off live. -->
    <!-- [id, human summary, enabled] for every gnoblin-gateable subsystem. -->
    <method name="ListFeatures">
      <arg type="a(ssb)" direction="out" name="features"/>
    </method>
    <!-- Whether a subsystem is currently enabled (unknown id -> false). -->
    <method name="GetFeature">
      <arg type="s" direction="in" name="id"/>
      <arg type="b" direction="out" name="enabled"/>
    </method>
    <!-- Turn a subsystem on/off live (persisted). Emits FeatureChanged. -->
    <method name="SetFeature">
      <arg type="s" direction="in" name="id"/>
      <arg type="b" direction="in" name="enabled"/>
    </method>
    <signal name="FeatureChanged">
      <arg type="s" name="id"/>
      <arg type="b" name="enabled"/>
    </signal>
    <!-- Whether the compositor is a Wayland session (soft-reload applies). -->
    <property name="IsWayland" type="b" access="read"/>
    <!-- The active gnome-shell session mode (expected: "gnoblin"). -->
    <property name="SessionMode" type="s" access="read"/>
  </interface>
</node>`;

export class Component {
    constructor() {
        this._impl = null;
        this._nameId = 0;
        this._overlayKeyId = 0;
        this._settings = null;
        this._settingsChangedId = 0;
        this._featureState = new Map();
        this._inputSourceManager = null;
        this._mixerControl = null;
        this._screenShareController = null;
        this._screenShareHandles = new Set();
        this._locationAgent = null;
        this._privacyState = null;
        this._compositorBridge = null;
        this._launchFeedback = null;
        this._acceleratorCapture = null;
        this._acceleratorCaptureSignal = 0;
    }

    enable() {
        this._settings = new Gio.Settings({ schema_id: SCHEMA_ID });
        this._settingsChangedId = this._settings.connect(`changed::${DISABLED_KEY}`, () => this._syncFeatureState());

        this._impl = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._impl.export(Gio.DBus.session, OBJECT_PATH);
        this._acceleratorCaptureSignal = global.stage.connect("captured-event", (_stage, event) =>
            this._captureAcceleratorEvent(event),
        );

        this._windowRules = new WindowRules();
        this._shortcutInput = new ShortcutInput(
            global.stage,
            () => Main.pushModal(global.stage, { actionMode: Shell.ActionMode.POPUP }),
            (grab) => Main.popModal(grab),
        );
        this._shortcuts = new Shortcuts(
            new CommandShortcuts(
                global.display,
                (action, enabled) => {
                    Main.wm.allowKeybinding(
                        typeof action === "string" ? action : Meta.external_binding_name_for_action(action),
                        enabled
                            ? Shell.ActionMode.NORMAL |
                                  Shell.ActionMode.OVERVIEW |
                                  Shell.ActionMode.POPUP |
                                  Shell.ActionMode.SYSTEM_MODAL |
                                  Shell.ActionMode.LOOKING_GLASS
                            : Shell.ActionMode.NONE,
                    );
                },
                undefined,
                (name) => this._shortcutInput.begin(name),
            ),
        );
        this._permissionPolicy = { default: "deny", rules: [] };
        this._config = new ConfigFile(
            undefined,
            (next) => this._applyConfig(next),
            () => {
                this._mutterEvents?.destroy();
                this._mutterEvents = new MutterEventForwarder(this._config);
            },
        );
        activeConfig = this._config;
        this._config.start();
        this._mutterEvents = new MutterEventForwarder(this._config);
        this._configFocusId = global.display.connect("notify::focus-window", () => {
            this._dispatchWindowEvent("focus_changed", global.display.focus_window);
            this._dispatchWindowEvent("gnome.shell.focus.changed", global.display.focus_window);
        });
        this._configEventId = global.display.connect("gnoblin-config-event", (_display, _event, document) => {
            this._config?.applyRuntimeDocument(document);
        });
        this._eventWindows = new Map();
        for (const window of global.display.list_all_windows()) this._watchEventWindow(window);
        this._configWindowId = global.display.connect("window-created", (_display, window) => {
            this._dispatchWindowEvent("window_created", window);
            this._dispatchWindowEvent("gnome.shell.window.created", window);
            this._watchEventWindow(window);
        });
        this._configInputId = global.stage.connect("captured-event", (_stage, event) => {
            const type = event.type();
            const name = CLUTTER_EVENT_NAMES.get(type) ?? `event_${type}`;
            const payload = { type: name, time: event.get_time() };
            if (
                [
                    Clutter.EventType.MOTION,
                    Clutter.EventType.BUTTON_PRESS,
                    Clutter.EventType.BUTTON_RELEASE,
                    Clutter.EventType.SCROLL,
                ].includes(type)
            ) {
                const [x, y] = event.get_coords();
                payload.x = x;
                payload.y = y;
            }
            if (type === Clutter.EventType.BUTTON_PRESS || type === Clutter.EventType.BUTTON_RELEASE)
                payload.button = event.get_button();
            if (type === Clutter.EventType.KEY_PRESS || type === Clutter.EventType.KEY_RELEASE)
                payload.key_symbol = event.get_key_symbol();
            if (type === Clutter.EventType.SCROLL) {
                const [dx, dy] = event.get_scroll_delta();
                payload.scroll_x = dx;
                payload.scroll_y = dy;
                payload.scroll_direction = String(event.get_scroll_direction());
            }
            const eventName = name.toLowerCase();
            this._config.dispatchEvent(`input.${eventName}`, payload);
            this._config.dispatchEvent(`gnome.shell.input.${eventName}`, payload);
            return Clutter.EVENT_PROPAGATE;
        });
        this._dispatchWindowEvent("focus_changed", global.display.focus_window);
        this._dispatchWindowEvent("gnome.shell.focus.changed", global.display.focus_window);

        // Apply the persisted feature state to the freshly-built subsystems.
        this._syncFeatureState();
        this._installOsdGate();

        this._setupDesktopState();

        // The socket is a core compositor service used by external shells and
        // gnoblinctl. It is not part of the optional, crash-quarantined user
        // script collection.
        try {
            this._compositorBridge = new CompositorBridge();
            global.__gnoblinCompositorBridge = this._compositorBridge;
        } catch (error) {
            logError(error, "gnoblin-control: compositor bridge startup failed");
        }
        try {
            this._launchFeedback = new LaunchFeedback();
        } catch (error) {
            logError(error, "gnoblin-control: launch feedback startup failed");
        }

        this._nameId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            () => console.log(`gnoblin-control: acquired ${BUS_NAME} at ${OBJECT_PATH}`),
            () => console.warn(`gnoblin-control: lost ${BUS_NAME} (another owner?)`),
        );

        // Mutter emits 'overlay-key' only when the configured Super key is
        // released without other input. This preserves Super-drag while
        // giving external chrome one precise edge to react to.
        this._overlayKeyId = global.display.connect("overlay-key", () => {
            if (this._acceleratorCapture) return;
            this._impl?.emit_signal(
                "SuperReleased",
                new GLib.Variant("(ut)", [SUPER_RELEASE_PROTOCOL_VERSION, GLib.get_monotonic_time()]),
            );
        });

        // User scripting: event bus + script host, loaded from the config dir.
        this._bus = new EventBus();
        this._bus.connectSources();
        this._scripts = new ScriptHost(this, this._bus);
        activeScriptHost = this._scripts;
        this._scripts.load().catch((e) => logError(e, "gnoblin-script: initial load failed"));

        // Seed the stylesheet identity so a first no-change Reload can skip the
        // theme swap (see stylesheetDigest above).
        lastStylesheetDigest = stylesheetDigest();

        // Periodically hand freed heap pages back to the kernel. Churn (theme
        // swaps, notification traffic, GC) otherwise ratchets RSS up for the
        // session lifetime; a full trim measures <10 ms on a ~230 MB heap.
        this._trimTimeoutId = GLib.timeout_add_seconds(GLib.PRIORITY_LOW, TRIM_INTERVAL_SECONDS, () => {
            Shell.util_trim_memory();
            return GLib.SOURCE_CONTINUE;
        });

        console.log(`gnoblin-control: enabled (mode=${this._mode()}, wayland=${Meta.is_wayland_compositor()})`);
    }

    disable() {
        this._finishAcceleratorCapture(null, "Shortcut capture cancelled because the shell is reloading");
        if (this._configFocusId) {
            global.display.disconnect(this._configFocusId);
            this._configFocusId = 0;
        }
        if (this._configWindowId) {
            global.display.disconnect(this._configWindowId);
            this._configWindowId = 0;
        }
        if (this._configEventId) {
            global.display.disconnect(this._configEventId);
            this._configEventId = 0;
        }
        if (this._configInputId) {
            global.stage.disconnect(this._configInputId);
            this._configInputId = 0;
        }
        for (const [window, id] of this._eventWindows ?? []) window.disconnect(id);
        this._eventWindows?.clear();
        this._mutterEvents?.destroy();
        this._mutterEvents = null;
        // Script disposers can still use window rules, config and the event bus.
        if (this._scripts) {
            this._scripts.destroy();
            this._scripts = null;
            activeScriptHost = null;
        }
        if (this._compositorBridge) {
            this._compositorBridge.destroy();
            if (global.__gnoblinCompositorBridge === this._compositorBridge) delete global.__gnoblinCompositorBridge;
            this._compositorBridge = null;
        }
        this._launchFeedback?.destroy();
        this._launchFeedback = null;
        this._shortcutInput?.destroy();
        this._shortcutInput = null;
        this._shortcuts?.destroy();
        this._shortcuts = null;
        this._windowRules?.destroy();
        this._windowRules = null;
        this._config?.destroy();
        this._config = null;
        activeConfig = null;
        if (this._acceleratorCaptureSignal) {
            global.stage.disconnect(this._acceleratorCaptureSignal);
            this._acceleratorCaptureSignal = 0;
        }
        if (this._overlayKeyId) {
            global.display.disconnect(this._overlayKeyId);
            this._overlayKeyId = 0;
        }
        if (this._trimTimeoutId) {
            GLib.source_remove(this._trimTimeoutId);
            this._trimTimeoutId = 0;
        }
        if (this._settings && this._settingsChangedId) {
            this._settings.disconnect(this._settingsChangedId);
            this._settingsChangedId = 0;
        }

        this._teardownDesktopState();
        // Restore every gated subsystem to stock before we go.
        this._removeOsdGate();
        for (const id of Object.keys(FEATURES)) FEATURES[id].apply(true);

        if (this._bus) {
            this._bus.destroy();
            this._bus = null;
        }

        if (this._nameId) {
            Gio.bus_unown_name(this._nameId);
            this._nameId = 0;
        }
        if (this._impl) {
            this._impl.unexport();
            this._impl = null;
        }
        this._settings = null;
        this._featureState.clear();
        console.log("gnoblin-control: disabled");
    }

    // --- desktop state ---
    _setupDesktopState() {
        this._inputSourceManager = Keyboard.getInputSourceManager();
        // The stock keyboard indicator normally initialises the keymap. Gnoblin
        // omits that indicator, so initialise it before IBus reports readiness.
        this._inputSourceManager.reload();
        this._inputSourceManager.connectObject(
            "current-source-changed",
            () => this._emitInputSourceChanged(),
            "sources-changed",
            () => this._emitInputSourcesChanged(),
            this,
        );

        try {
            this._mixerControl = Volume.getMixerControl();
            this._mixerControl.connectObject(
                "stream-added",
                () => this._emitPrivacyState(),
                "stream-removed",
                () => this._emitPrivacyState(),
                this,
            );
        } catch (e) {
            logError(e, "gnoblin-control: microphone state monitoring failed");
        }

        this._screenShareController = global.backend.get_remote_access_controller();
        this._screenShareController?.connectObject(
            "new-handle",
            (_controller, handle) => this._onRemoteAccessHandle(handle),
            this,
        );

        try {
            this._locationAgent = Location.getGeoclueAgent();
            this._locationAgent.connectObject("notify::in-use", () => this._emitPrivacyState(), this);
            global.__gnoblinLocationCaptures = () =>
                (this._locationAgent?.activeApps ?? []).map((appId) => ({ appId, app: appId, device: "Location" }));
        } catch (e) {
            logError(e, "gnoblin-control: location state monitoring failed");
        }

        this._privacyState = this._currentPrivacyState();
    }

    _teardownDesktopState() {
        this._inputSourceManager?.disconnectObject(this);
        this._mixerControl?.disconnectObject(this);
        this._screenShareController?.disconnectObject(this);
        this._locationAgent?.disconnectObject(this);
        for (const handle of this._screenShareHandles) handle.disconnectObject(this);

        this._inputSourceManager = null;
        this._mixerControl = null;
        this._screenShareController = null;
        this._screenShareHandles.clear();
        this._locationAgent = null;
        global.__gnoblinLocationCaptures = null;
        this._privacyState = null;
    }

    _inputSourceRecord(source) {
        if (!source) return ["", "", "", ""];

        return [source.type ?? "", source.id ?? "", source.shortName ?? "", source.displayName ?? ""];
    }

    _emitInputSourceChanged() {
        const source = this._inputSourceManager?.currentSource;
        this._impl?.emit_signal("InputSourceChanged", new GLib.Variant("(ssss)", this._inputSourceRecord(source)));
    }

    _emitInputSourcesChanged() {
        this._impl?.emit_signal("InputSourcesChanged", new GLib.Variant("(b)", [true]));
        this._emitInputSourceChanged();
    }

    _onRemoteAccessHandle(handle) {
        if (handle.isRecording ?? handle.is_recording ?? false) return;

        this._screenShareHandles.add(handle);
        handle.connectObject(
            "stopped",
            () => {
                this._screenShareHandles.delete(handle);
                this._emitPrivacyState();
            },
            this,
        );
        this._emitPrivacyState();
    }

    _currentPrivacyState() {
        let microphoneInUse = false;
        try {
            const ignoredApplications = new Set(["org.gnome.VolumeControl", "org.PulseAudio.pavucontrol"]);
            const sourceOutputs = this._mixerControl?.get_source_outputs() ?? [];
            microphoneInUse = sourceOutputs.some((output) => !ignoredApplications.has(output.get_application_id()));
        } catch {
            microphoneInUse = false;
        }

        return [this._screenShareHandles.size > 0, microphoneInUse, this._locationAgent?.inUse ?? false];
    }

    _emitPrivacyState() {
        const state = this._currentPrivacyState();
        if (this._privacyState?.every((value, index) => value === state[index])) return;

        this._privacyState = state;
        this._impl?.emit_signal("PrivacyStateChanged", new GLib.Variant("(bbb)", state));
    }

    _applyConfig(next) {
        Workspaces.configure(next["window-management"]["workspace-ids"] ?? [], (index) =>
            Meta.prefs_get_workspace_name(index),
        );
        applyWindowPreferences(next["window-management"]);
        applyCompositorPreferences(next.compositor);
        applyInputPreferences(next.input);
        Keyboard.configureGnoblinInputSources(
            next["input-sources"]?.sources ?? null,
            next["input-sources"]?.["per-window"] ?? false,
            next.input?.keyboard?.["xkb-options"] ?? null,
        );
        try {
            this._shortcuts.apply(next);
        } catch (error) {
            // A compositor-owned accelerator must not make an otherwise valid
            // config (especially its permission policy) disappear. The
            // shortcut manager has already rolled back its partial changes.
            if (!String(error?.message ?? error).startsWith("shortcut already claimed:")) throw error;
            console.warn(`gnoblin-config: skipping conflicting command shortcut: ${error.message}`);
        }
        const disabled = new Set(this._disabledList());
        for (const id of FEATURE_KEYS) {
            if (REMOVED_FEATURES.has(id)) disabled.add(id);
            else if (next[id] === true) disabled.delete(id);
            else if (next[id] === false) disabled.add(id);
        }
        const previous = this._disabledList();
        if (disabled.size !== previous.length || !previous.every((id) => disabled.has(id))) {
            if (!this._settings.set_strv(DISABLED_KEY, [...disabled]))
                throw new Error("could not save configured feature settings");
        }
        this._windowRules.refresh(next);
        autostart.apply(next.autostart);
        this._permissionPolicy = next.permissions;
        Meta.prefs_set_gnoblin_cursor_config(next.cursor.theme, next.cursor.size);
    }

    _dispatchWindowEvent(event, window) {
        if (!this._config) return;
        this._config.dispatchEvent(event, {
            app_id: window?.get_gtk_application_id() || "",
            wm_class: window?.get_wm_class() || "",
            title: window?.get_title() || "",
        });
    }

    _watchEventWindow(window) {
        if (this._eventWindows.has(window)) return;
        const id = window.connect("unmanaged", () => {
            this._dispatchWindowEvent("window_unmanaged", window);
            this._dispatchWindowEvent("gnome.shell.window.unmanaged", window);
            this._eventWindows.delete(window);
        });
        this._eventWindows.set(window, id);
    }

    // --- feature toggles ---
    _disabledList() {
        return this._settings ? this._settings.get_strv(DISABLED_KEY) : [];
    }

    _isEnabled(id) {
        return !REMOVED_FEATURES.has(id) && !this._disabledList().includes(id);
    }

    _syncFeatureState() {
        for (const id of Object.keys(FEATURES)) {
            const enabled = this._isEnabled(id);
            const previous = this._featureState.get(id);
            if (previous === enabled) continue;

            try {
                FEATURES[id].apply(enabled);
            } catch (e) {
                logError(e, `gnoblin: applying feature ${id} failed`);
            }

            this._featureState.set(id, enabled);
            if (previous === undefined) continue;

            this._impl?.emit_signal("FeatureChanged", new GLib.Variant("(sb)", [id, enabled]));
            this._config?.dispatchEvent("gnoblin.feature.changed", { feature: id, enabled });
            console.log(`gnoblin-control: feature '${id}' ${enabled ? "ENABLED" : "DISABLED"}`);
        }
    }

    _emitOsdRequested(monitorIndex, icon, label, level, maxLevel) {
        const outputNames = osdOutputNamesForMonitorIndex(monitorIndex);
        if (outputNames.length === 0) {
            console.warn(`gnoblin-control: no active output for OSD monitor ${serialiseOsdMonitorIndex(monitorIndex)}`);
            return false;
        }

        const normalizedLevel = serialiseOsdLevel(level);
        // Brightness uses OsdWindowManager.show(), whose omitted maxLevel is
        // represented as -1. External OSDs need the normalized 0..1 scale to
        // render the brightness percentage and progress bar.
        let suppliedMax;
        try {
            suppliedMax = Number(maxLevel);
        } catch {
            suppliedMax = Number.NaN;
        }
        const max = Number.isFinite(suppliedMax) ? suppliedMax : -1;
        const normalizedMax = normalizedLevel >= 0 && max < 0 ? 1 : max;
        const fields = [
            OSD_REQUEST_PROTOCOL_VERSION,
            serialiseOsdMonitorIndex(monitorIndex),
            serialiseOsdIcon(icon),
            serialiseOsdString(label),
            normalizedLevel,
            normalizedMax,
            outputNames,
        ];

        try {
            this._impl?.emit_signal("OsdRequested", new GLib.Variant("(uissddas)", fields));
            return true;
        } catch (e) {
            logError(e, "gnoblin-control: failed to emit OsdRequested");
            return false;
        }
    }

    // Keep the existing OSD transport contract without any native OSD widgets.
    _installOsdGate() {
        const mgr = Main.osdWindowManager;
        if (!mgr || this._osdGateInstalled) return;
        mgr._showOsdWindow = (...args) => this._emitOsdRequested(...args);
        this._osdGateInstalled = true;
    }

    _removeOsdGate() {
        const mgr = Main.osdWindowManager;
        if (mgr && this._osdGateInstalled) delete mgr._showOsdWindow; // restore the prototype method
        this._osdGateInstalled = false;
    }

    ListFeatures() {
        return Object.entries(FEATURES).map(([id, f]) => [id, f.summary, this._isEnabled(id)]);
    }

    GetFeature(id) {
        return Object.hasOwn(FEATURES, id) && this._isEnabled(id);
    }

    SetFeature(id, enabled) {
        if (REMOVED_FEATURES.has(id)) {
            if (enabled) throw new Error(`${id}: native UI has been removed; use the external shell`);
            return;
        }
        if (!Object.hasOwn(FEATURES, id)) throw new Error(`unknown feature: ${id}`);
        if (this._isEnabled(id) === enabled) return;

        const disabled = new Set(this._disabledList());
        if (enabled) disabled.delete(id);
        else disabled.add(id);

        if (!this._settings.set_strv(DISABLED_KEY, [...disabled])) throw new Error(`failed to persist feature: ${id}`);
    }

    // --- org.gnoblin.Shell ---
    Ping() {
        return "pong";
    }

    GetVersion() {
        return Config.PACKAGE_VERSION ?? "unknown";
    }

    CaptureAcceleratorAsync(params, invocation) {
        const [waitSeconds] = params.deep_unpack();
        if (!Number.isInteger(waitSeconds) || waitSeconds < 1 || waitSeconds > 60) {
            invocation.return_dbus_error(`${BUS_NAME}.Error.CaptureFailed`, "waitSeconds must be between 1 and 60");
            return;
        }
        if (this._acceleratorCapture) {
            invocation.return_dbus_error(`${BUS_NAME}.Error.CaptureFailed`, "another shortcut capture is active");
            return;
        }
        if (Main.sessionMode.isLocked) {
            invocation.return_dbus_error(
                `${BUS_NAME}.Error.CaptureFailed`,
                "cannot capture a shortcut while the screen is locked",
            );
            return;
        }

        let grab;
        try {
            grab = Main.pushModal(global.stage, { actionMode: Shell.ActionMode.SYSTEM_MODAL });
        } catch (error) {
            invocation.return_dbus_error(`${BUS_NAME}.Error.CaptureFailed`, error.message);
            return;
        }
        if (!grab || !(grab.get_seat_state() & Clutter.GrabState.KEYBOARD)) {
            if (grab) Main.popModal(grab);
            invocation.return_dbus_error(`${BUS_NAME}.Error.CaptureFailed`, "keyboard input is already grabbed");
            return;
        }

        const modifierKeys = new Set([
            Clutter.KEY_Shift_L,
            Clutter.KEY_Shift_R,
            Clutter.KEY_Control_L,
            Clutter.KEY_Control_R,
            Clutter.KEY_Alt_L,
            Clutter.KEY_Alt_R,
            Clutter.KEY_Meta_L,
            Clutter.KEY_Meta_R,
            Clutter.KEY_Super_L,
            Clutter.KEY_Super_R,
            Clutter.KEY_Hyper_L,
            Clutter.KEY_Hyper_R,
            Clutter.KEY_ISO_Level3_Shift,
            Clutter.KEY_ISO_Level5_Shift,
            Clutter.KEY_Caps_Lock,
            Clutter.KEY_Num_Lock,
        ]);
        const capture = {
            invocation,
            grab,
            modifierKeys,
            heldModifiers: new Set(),
            onlySuper: true,
            pending: null,
            sessionSignal: 0,
            timeout: 0,
        };
        this._acceleratorCapture = capture;
        capture.sessionSignal = Main.sessionMode.connect("updated", () => {
            if (Main.sessionMode.isLocked)
                this._finishAcceleratorCapture(null, "Shortcut capture cancelled because the screen was locked");
        });
        capture.timeout = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, waitSeconds, () => {
            capture.timeout = 0;
            this._finishAcceleratorCapture(null, `timed out waiting ${waitSeconds} seconds for a shortcut`);
            return GLib.SOURCE_REMOVE;
        });
    }

    _captureAcceleratorEvent(event) {
        const capture = this._acceleratorCapture;
        if (!capture) return Clutter.EVENT_PROPAGATE;
        const type = event.type();
        if (type !== Clutter.EventType.KEY_PRESS && type !== Clutter.EventType.KEY_RELEASE) return Clutter.EVENT_STOP;

        const key = event.get_key_symbol();
        if (type === Clutter.EventType.KEY_PRESS) {
            if (capture.modifierKeys.has(key)) {
                capture.heldModifiers.add(key);
                if (key !== Clutter.KEY_Super_L && key !== Clutter.KEY_Super_R) capture.onlySuper = false;
                return Clutter.EVENT_STOP;
            }
            if (capture.pending) return Clutter.EVENT_STOP;

            const modifiers = event.get_state() & Gtk.accelerator_get_default_mod_mask();
            if (key === Clutter.KEY_Escape && modifiers === 0) {
                capture.pending = { key, cancelled: true };
                return Clutter.EVENT_STOP;
            }
            const accelerator = Gtk.accelerator_name(key, modifiers);
            capture.pending = accelerator ? { key, accelerator } : { key, error: "key has no GTK accelerator name" };
            return Clutter.EVENT_STOP;
        }

        if (capture.modifierKeys.has(key)) {
            capture.heldModifiers.delete(key);
            if (!capture.pending && capture.onlySuper && capture.heldModifiers.size === 0) {
                this._finishAcceleratorCapture("Super");
                return Clutter.EVENT_STOP;
            }
        }
        if (capture.pending?.key === key) capture.pending.released = true;
        if (capture.pending?.released && capture.heldModifiers.size === 0) {
            const pending = capture.pending;
            if (pending.cancelled) this._finishAcceleratorCapture(null, "Shortcut capture cancelled");
            else if (pending.error) this._finishAcceleratorCapture(null, pending.error);
            else this._finishAcceleratorCapture(pending.accelerator);
        }
        return Clutter.EVENT_STOP;
    }

    _finishAcceleratorCapture(accelerator, error = null) {
        const capture = this._acceleratorCapture;
        if (!capture) return;
        this._acceleratorCapture = null;
        if (capture.timeout) GLib.source_remove(capture.timeout);
        if (capture.sessionSignal) Main.sessionMode.disconnect(capture.sessionSignal);
        try {
            Main.popModal(capture.grab);
        } catch (popError) {
            logError(popError, "gnoblin-control: failed to release shortcut capture keyboard grab");
        }
        if (error) capture.invocation.return_dbus_error(`${BUS_NAME}.Error.CaptureFailed`, error);
        else capture.invocation.return_value(new GLib.Variant("(s)", [accelerator]));
    }

    ListInputSources() {
        const manager = this._inputSourceManager ?? Keyboard.getInputSourceManager();
        return Object.values(manager.inputSources).map((source) => this._inputSourceRecord(source));
    }

    GetCurrentInputSource() {
        const manager = this._inputSourceManager ?? Keyboard.getInputSourceManager();
        return this._inputSourceRecord(manager.currentSource);
    }

    SetInputSource(type, id) {
        const manager = this._inputSourceManager ?? Keyboard.getInputSourceManager();
        const source = Object.values(manager.inputSources).find(
            (candidate) => candidate.type === type && candidate.id === id,
        );
        if (!source) throw new Error(`unknown input source: ${type}/${id}`);

        manager.activateInputSource(source, true);
    }

    GetPrivacyState() {
        return this._currentPrivacyState();
    }

    ReloadConfigAsync(_params, invocation) {
        try {
            this._config.reload();
            invocation.return_value(null);
        } catch (e) {
            console.warn(`gnoblin-config: keeping last valid settings: ${e.message}`);
            invocation.return_dbus_error(`${BUS_NAME}.Error.ReloadFailed`, e.message);
        }
    }

    ReloadAsync(_params, invocation) {
        return this._runReload(invocation, () => softReload("org.gnoblin.Shell.Reload"), "soft reload");
    }

    ListScripts() {
        return this._scripts?.list() ?? [];
    }

    async _runReload(invocation, operation, description) {
        try {
            await operation();
            invocation.return_value(null);
        } catch (e) {
            logError(e, `gnoblin-control: ${description} failed`);
            invocation.return_dbus_error(`${BUS_NAME}.Error.ReloadFailed`, `${description} failed: ${e.message}`);
        }
    }

    _grantsDir(portal) {
        return GLib.build_filenamev([GLib.get_user_data_dir(), "gnoblin", "portal-grants", portal]);
    }

    _readPortalGrant(portal, id) {
        const path = GLib.build_filenamev([this._grantsDir(portal), id]);
        const keyFile = new GLib.KeyFile();

        try {
            keyFile.load_from_file(path, 0);
            const version = keyFile.get_integer(PORTAL_GRANT_GROUP, "version");
            const storedPortal = keyFile.get_string(PORTAL_GRANT_GROUP, "portal");
            const identity = keyFile.get_string(PORTAL_GRANT_GROUP, "identity");
            const deviceTypes = keyFile.get_integer(PORTAL_GRANT_GROUP, "device-types");
            const clipboardEnabled = keyFile.get_boolean(PORTAL_GRANT_GROUP, "clipboard-enabled");
            let hasStreams = true;
            try {
                keyFile.get_string(PORTAL_GRANT_GROUP, "streams");
            } catch {
                hasStreams = false;
            }
            const validIdentity =
                (identity.startsWith("app-id:") && identity.length > 7) ||
                (identity.startsWith("host-exe:/") && identity.length > 10);
            const validCapabilities =
                portal === "screen-cast"
                    ? deviceTypes === 0 && !clipboardEnabled && hasStreams
                    : deviceTypes >= 0 &&
                      (deviceTypes & ~7) === 0 &&
                      (deviceTypes !== 0 || clipboardEnabled || hasStreams);

            if (version !== PORTAL_GRANT_VERSION || storedPortal !== portal || !validIdentity || !validCapabilities)
                throw new Error("grant metadata does not match its scope");

            return [id, portal, identity, deviceTypes, clipboardEnabled, hasStreams];
        } catch (e) {
            logError(e, `gnoblin: ignoring invalid portal grant ${portal}/${id}`);
            return null;
        }
    }

    GetPermissions() {
        return JSON.stringify({
            policy: this._permissionPolicy ?? Permissions.DEFAULT_POLICY,
            capabilities: Permissions.CAPABILITIES,
            levels: Permissions.LEVELS,
            path: this._config.path,
        });
    }

    CheckPermissionAsync([capability, identity], invocation) {
        try {
            const decision = Permissions.evaluate(
                this._permissionPolicy ?? Permissions.DEFAULT_POLICY,
                capability,
                identity,
            );
            invocation.return_value(
                new GLib.Variant("(ssasub)", [
                    decision.level,
                    decision.rule,
                    decision.monitors,
                    decision.devices,
                    decision.clipboard,
                ]),
            );
        } catch (error) {
            invocation.return_dbus_error(`${BUS_NAME}.Error.PermissionPolicy`, error.message);
        }
    }

    ListPortalGrants() {
        const grants = [];

        for (const portal of PORTAL_GRANT_KINDS) {
            const dir = Gio.File.new_for_path(this._grantsDir(portal));
            let enumerator;

            try {
                enumerator = dir.enumerate_children("standard::name,standard::type", Gio.FileQueryInfoFlags.NONE, null);
                let info;
                while ((info = enumerator.next_file(null)) !== null) {
                    const id = info.get_name();
                    if (info.get_file_type() !== Gio.FileType.REGULAR || !PORTAL_GRANT_FILE_PATTERN.test(id)) continue;
                    const grant = this._readPortalGrant(portal, id);
                    if (grant) grants.push(grant);
                }
            } catch (e) {
                if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND))
                    logError(e, `gnoblin: failed to list ${portal} grants`);
            } finally {
                enumerator?.close(null);
            }
        }

        return grants.sort((a, b) => `${a[1]}:${a[2]}:${a[0]}`.localeCompare(`${b[1]}:${b[2]}:${b[0]}`));
    }

    RevokePortalGrant(portal, id) {
        if (!PORTAL_GRANT_KINDS.includes(portal)) throw new Error(`invalid portal grant kind: ${portal}`);
        if (!PORTAL_GRANT_FILE_PATTERN.test(id)) throw new Error(`invalid portal grant id: ${id}`);

        const file = Gio.File.new_for_path(GLib.build_filenamev([this._grantsDir(portal), id]));
        if (!file.query_exists(null)) throw new Error(`no such portal grant: ${portal}/${id}`);
        file.delete(null);
        console.log(`gnoblin-control: revoked portal grant '${portal}/${id}'`);
    }

    get IsWayland() {
        return Meta.is_wayland_compositor();
    }

    get SessionMode() {
        return this._mode();
    }

    _mode() {
        return Main.sessionMode?.currentMode ?? GLib.getenv("GNOME_SHELL_SESSION_MODE") ?? "unknown";
    }
}
