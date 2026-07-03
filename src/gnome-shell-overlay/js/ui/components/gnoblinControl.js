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
// Phase-2 skeleton: stands up the org.gnoblin.Shell bus name + a few read/health
// methods. Feature toggles (Phase 3) and the Wayland soft-reload (Phase 2.5) hang
// off this same object.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

import * as Main from '../main.js';
import * as Config from '../../misc/config.js';
import {ExtensionState} from '../../misc/extensionUtils.js';

const BUS_NAME = 'org.gnoblin.Shell';
const OBJECT_PATH = '/org/gnoblin/Shell';
const SCHEMA_ID = 'org.gnoblin.shell';
const DISABLED_KEY = 'disabled-features';

// The live ScriptHost, so the module-level softReload() can re-run user scripts.
let activeScriptHost = null;

// Process-wide monotonic counter for the script import cache-bust. Module-level
// (not per-host) so a disable→re-enable in the same process still re-imports fresh
// code instead of reusing the cached module.
let scriptImportSeq = 0;

// OSD subsystems gnoblin can gate individually, classified from the OSD icon.
// The single osd gate (installed in enable()) reads these live; `osd` is the master.
const OSD_TYPES = {
    'osd-volume': ['audio-volume', 'audio-speaker'],
    'osd-microphone': ['microphone', 'audio-input'],
    'osd-brightness': ['display-brightness'],
    'osd-keyboard-brightness': ['keyboard-brightness'],
};

// Classify an OSD by its icon → the per-type feature id, or null (unknown type,
// gated only by the master switch). Volume/brightness/etc. pass a Gio.ThemedIcon.
function classifyOsd(icon) {
    let names = [];
    try {
        names = icon?.get_names?.() ?? (icon?.to_string ? [icon.to_string()] : []);
    } catch {
        names = [];
    }
    const hay = names.join(' ');
    for (const [feature, prefixes] of Object.entries(OSD_TYPES)) {
        if (prefixes.some(p => hay.includes(p)))
            return feature;
    }
    return null;
}

// Runtime feature toggles. Each feature gates a gnome-shell subsystem so an
// external userspace (Quickshell, waybar, custom) can own it instead — live, with
// no compositor restart. A feature is ENABLED unless its id is in the
// org.gnoblin.shell 'disabled-features' list. Two kinds: 'screenshot' shadows a
// method in its apply(); the OSD family is enforced by a shared state-driven gate
// (installed in enable()), so their apply() is a no-op — the gate reads state live.
const FEATURES = {
    osd: {summary: 'On-screen display popups — master switch (all OSDs)', apply() {}},
    'osd-volume': {summary: 'Volume OSD popup', apply() {}},
    'osd-microphone': {summary: 'Microphone OSD popup', apply() {}},
    'osd-brightness': {summary: 'Screen-brightness OSD popup', apply() {}},
    'osd-keyboard-brightness': {summary: 'Keyboard-brightness OSD popup', apply() {}},
    screenshot: {
        summary: 'Built-in screenshot / screencast UI',
        apply(enabled) {
            const ui = Main.screenshotUI;
            if (!ui)
                return;
            if (enabled)
                delete ui.open;              // restore ScreenshotUI.prototype.open
            else
                ui.open = async () => {};     // no-op the built-in capture UI
        },
    },
};

// Soft, in-process reload — the Wayland-safe answer to "reload the shell without
// logging out". mutter/Wayland is NEVER torn down, so windows and the (external)
// chrome survive. We reload only the mutable JS layer: the shell theme/CSS and any
// enabled extensions (re-running their enable() so they pick up new settings/CSS).
// gnoblin keeps almost nothing else in-process — the chrome lives in a separate
// layer-shell client — so this covers the practical need. A true process re-exec
// on Wayland cannot preserve clients (no handoff protocol), which is exactly why
// this is a soft reload and not global.reexec_self().
export function softReload(reason = 'manual') {
    console.log(`gnoblin: soft-reload (${reason}) — reloading theme + extensions in-process`);

    try {
        Main.loadTheme();
    } catch (e) {
        logError(e, 'gnoblin: soft-reload loadTheme failed');
    }

    const em = Main.extensionManager;
    if (em) {
        // reloadExtension() re-imports the extension's code (cache-busted by the
        // 34-extension-hot-reload patch), so soft-reload picks up code edits live.
        // Serialize: reloadExtension() mutates _extensionOrder and disables/re-enables
        // dependent extensions, so running them in parallel would race.
        const active = em.getUuids().filter(
            uuid => em.lookup(uuid)?.state === ExtensionState.ACTIVE);
        (async () => {
            for (const uuid of active) {
                try {
                    await em.reloadExtension(em.lookup(uuid));
                } catch (e) {
                    logError(e, `gnoblin: soft-reload of ${uuid} failed`);
                }
            }
        })().catch(e => logError(e, 'gnoblin: soft-reload extension pass failed'));
    }

    // Re-run user scripts (hot-reloaded via cache-busted import).
    activeScriptHost?.reload();
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
        this._handlers.push([display,
            display.connect('window-created', (_d, win) => this.emit('window-opened', win))]);
        const wm = global.workspace_manager;
        this._handlers.push([wm,
            wm.connect('active-workspace-changed',
                () => this.emit('workspace-changed', wm.get_active_workspace_index()))]);
    }

    subscribe(event, cb) {
        if (!this._subs.has(event))
            this._subs.set(event, new Set());
        this._subs.get(event).add(cb);
        return () => this._subs.get(event)?.delete(cb);
    }

    emit(event, ...args) {
        for (const cb of this._subs.get(event) ?? []) {
            try {
                cb(...args);
            } catch (e) {
                logError(e, `gnoblin-script: handler for '${event}' threw`);
            }
        }
    }

    destroy() {
        for (const [obj, id] of this._handlers) {
            try {
                obj.disconnect(id);
            } catch { /* already gone */ }
        }
        this._handlers = [];
        this._subs.clear();
    }
}

// Loads lightweight GJS user scripts from $XDG_CONFIG_HOME/gnoblin/scripts/*.js.
// Each script default-exports (api) => {...}: reactive glue over org.gnoblin.*,
// lighter than an extension, hot-reloadable (cache-busted import). This is the
// gnoblin answer to "a scripting language like Hyprland's Lua" — native GJS.
class ScriptHost {
    constructor(control, bus) {
        this._control = control;
        this._bus = bus;
        this._dir = GLib.build_filenamev([GLib.get_user_config_dir(), 'gnoblin', 'scripts']);
        this._loaded = [];
        this._generation = 0;   // bumped on every load/unload to drop stale in-flight imports
        this._destroyed = false;
    }

    _api(name) {
        const disposers = [];
        return {
            _disposers: disposers,
            log: (...a) => console.log(`gnoblin-script[${name}]:`, ...a),
            version: () => this._control.GetVersion(),
            getFeature: id => this._control.GetFeature(id),
            setFeature: (id, on) => this._control.SetFeature(id, on),
            reloadShell: () => softReload('script'),
            on: (event, cb) => {
                const d = this._bus.subscribe(event, cb);
                disposers.push(d);
                return d;
            },
        };
    }

    _scriptNames() {
        const dir = Gio.File.new_for_path(this._dir);
        if (!dir.query_exists(null))
            return [];
        let e;
        try {
            e = dir.enumerate_children('standard::name', Gio.FileQueryInfoFlags.NONE, null);
        } catch {
            return [];
        }
        const names = [];
        let info;
        while ((info = e.next_file(null)) !== null) {
            const n = info.get_name();
            if (n.endsWith('.js'))
                names.push(n);
        }
        return names.sort();
    }

    _disposeApi(api) {
        for (const d of api._disposers ?? []) {
            try {
                d();
            } catch { /* ignore */ }
        }
    }

    load() {
        if (this._destroyed)
            return;
        const gen = ++this._generation;
        for (const name of this._scriptNames()) {
            const path = GLib.build_filenamev([this._dir, name]);
            // First import in the process uses the plain URI; every later (re)load
            // cache-busts so code edits take effect. Module-level seq so a re-enable
            // in the same process is still fresh.
            scriptImportSeq++;
            const uri = scriptImportSeq > 1
                ? `file://${path}?gnoblinScript=${scriptImportSeq}`
                : `file://${path}`;
            import(uri).then(mod => {
                // Drop stale in-flight imports: a newer load/unload happened, or the
                // host was destroyed, while this import was pending.
                if (this._destroyed || gen !== this._generation)
                    return;
                if (typeof mod.default !== 'function') {
                    console.warn(`gnoblin-script: ${name} has no default-exported function`);
                    return;
                }
                const api = this._api(name);
                try {
                    mod.default(api);
                    this._loaded.push({name, api});
                    console.log(`gnoblin-script: loaded ${name}`);
                } catch (e) {
                    // The script may have subscribed via api.on() before throwing —
                    // dispose those so a failed load doesn't leak handlers.
                    this._disposeApi(api);
                    logError(e, `gnoblin-script: ${name} threw on load`);
                }
            }).catch(e => logError(e, `gnoblin-script: importing ${name} failed`));
        }
    }

    unload() {
        // Invalidate any in-flight imports from the current generation.
        this._generation++;
        for (const {api} of this._loaded)
            this._disposeApi(api);
        this._loaded = [];
    }

    reload() {
        this.unload();
        this.load();
    }

    destroy() {
        this._destroyed = true;
        this.unload();
    }

    list() {
        return this._loaded.map(s => s.name);
    }
}

// Human-readable name for an ExtensionState value.
const STATE_NAMES = Object.fromEntries(
    Object.entries(ExtensionState).map(([k, v]) => [v, k.toLowerCase()]));

// The wire contract. Deliberately small for now; grows with Phases 2.5/3.
const IFACE = `
<node>
  <interface name="org.gnoblin.Shell">
    <!-- Liveness check: returns "pong". -->
    <method name="Ping">
      <arg type="s" direction="out" name="pong"/>
    </method>
    <!-- Shell + protocol version string, e.g. "49.6-gnoblin". -->
    <method name="GetVersion">
      <arg type="s" direction="out" name="version"/>
    </method>
    <!-- Soft in-process reload (theme + extensions). Wayland-safe: keeps windows. -->
    <method name="Reload"/>
    <!-- Extensions: [uuid, state] for every known extension. -->
    <method name="ListExtensions">
      <arg type="a(ss)" direction="out" name="extensions"/>
    </method>
    <!-- Hot-reload one extension's code in-place (re-imports fresh source). -->
    <method name="ReloadExtension">
      <arg type="s" direction="in" name="uuid"/>
    </method>
    <!-- User scripts: names of the loaded ~/.config/gnoblin/scripts/*.js. -->
    <method name="ListScripts">
      <arg type="as" direction="out" name="scripts"/>
    </method>
    <!-- Reload all user scripts in-place (re-imports fresh source). -->
    <method name="ReloadScripts"/>
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
        this._settings = null;
    }

    enable() {
        this._settings = new Gio.Settings({schema_id: SCHEMA_ID});

        this._impl = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._impl.export(Gio.DBus.session, OBJECT_PATH);

        // Apply the persisted feature state to the freshly-built subsystems.
        this._applyAllFeatures();
        this._installOsdGate();

        this._nameId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            () => console.log(`gnoblin-control: acquired ${BUS_NAME} at ${OBJECT_PATH}`),
            () => console.warn(`gnoblin-control: lost ${BUS_NAME} (another owner?)`));

        // User scripting: event bus + script host, loaded from the config dir.
        this._bus = new EventBus();
        this._bus.connectSources();
        this._scripts = new ScriptHost(this, this._bus);
        activeScriptHost = this._scripts;
        this._scripts.load();

        console.log(`gnoblin-control: enabled (mode=${this._mode()}, wayland=${Meta.is_wayland_compositor()})`);
    }

    disable() {
        // Restore every gated subsystem to stock before we go.
        this._removeOsdGate();
        for (const id of Object.keys(FEATURES))
            FEATURES[id].apply(true);

        if (this._scripts) {
            this._scripts.destroy();
            this._scripts = null;
            activeScriptHost = null;
        }
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
        console.log('gnoblin-control: disabled');
    }

    // --- feature toggles ---
    _disabledList() {
        return this._settings ? this._settings.get_strv(DISABLED_KEY) : [];
    }

    _isEnabled(id) {
        return !this._disabledList().includes(id);
    }

    _applyAllFeatures() {
        for (const id of Object.keys(FEATURES)) {
            try {
                FEATURES[id].apply(this._isEnabled(id));
            } catch (e) {
                logError(e, `gnoblin: applying feature ${id} failed`);
            }
        }
    }

    // Install a single state-driven wrapper on OsdWindowManager._showOsdWindow —
    // the chokepoint both show() and showAll() funnel through. It reads the feature
    // state live per call, so the master 'osd' switch and the per-type switches
    // (osd-volume, osd-brightness, ...) take effect immediately with no re-apply.
    _installOsdGate() {
        const mgr = Main.osdWindowManager;
        if (!mgr || this._osdGateInstalled)
            return;
        const control = this;
        const orig = mgr._showOsdWindow;   // the prototype method
        mgr._showOsdWindow = function (monitorIndex, icon, label, level, maxLevel) {
            if (!control._isEnabled('osd'))
                return;                                 // master off → all OSDs suppressed
            const feature = classifyOsd(icon);
            if (feature && !control._isEnabled(feature))
                return;                                 // this OSD type is turned off
            return orig.call(this, monitorIndex, icon, label, level, maxLevel);
        };
        this._osdGateInstalled = true;
    }

    _removeOsdGate() {
        const mgr = Main.osdWindowManager;
        if (mgr && this._osdGateInstalled)
            delete mgr._showOsdWindow;   // restore the prototype method
        this._osdGateInstalled = false;
    }

    ListFeatures() {
        return Object.entries(FEATURES).map(
            ([id, f]) => [id, f.summary, this._isEnabled(id)]);
    }

    GetFeature(id) {
        return Object.hasOwn(FEATURES, id) && this._isEnabled(id);
    }

    SetFeature(id, enabled) {
        if (!Object.hasOwn(FEATURES, id))
            throw new Error(`unknown feature: ${id}`);
        if (this._isEnabled(id) === enabled)
            return;

        const disabled = new Set(this._disabledList());
        if (enabled)
            disabled.delete(id);
        else
            disabled.add(id);
        this._settings.set_strv(DISABLED_KEY, [...disabled]);

        FEATURES[id].apply(enabled);
        this._impl?.emit_signal('FeatureChanged', new GLib.Variant('(sb)', [id, enabled]));
        console.log(`gnoblin-control: feature '${id}' ${enabled ? 'ENABLED' : 'DISABLED'}`);
    }

    // --- org.gnoblin.Shell ---
    Ping() {
        return 'pong';
    }

    GetVersion() {
        return Config.PACKAGE_VERSION ?? 'unknown';
    }

    Reload() {
        softReload('org.gnoblin.Shell.Reload');
    }

    ListExtensions() {
        const em = Main.extensionManager;
        if (!em)
            return [];
        return em.getUuids().map(uuid => {
            const ext = em.lookup(uuid);
            return [uuid, STATE_NAMES[ext?.state] ?? 'unknown'];
        });
    }

    ReloadExtension(uuid) {
        const em = Main.extensionManager;
        const ext = em?.lookup(uuid);
        if (!ext)
            throw new Error(`unknown extension: ${uuid}`);
        // Fire-and-forget; reloadExtension re-imports fresh code (cache-busted).
        em.reloadExtension(ext).catch(
            e => logError(e, `gnoblin: hot-reload of ${uuid} failed`));
        console.log(`gnoblin-control: hot-reloading extension '${uuid}'`);
    }

    ListScripts() {
        return this._scripts?.list() ?? [];
    }

    ReloadScripts() {
        this._scripts?.reload();
        console.log('gnoblin-control: reloading user scripts');
    }

    get IsWayland() {
        return Meta.is_wayland_compositor();
    }

    get SessionMode() {
        return this._mode();
    }

    _mode() {
        return Main.sessionMode?.currentMode ?? GLib.getenv('GNOME_SHELL_SESSION_MODE') ?? 'unknown';
    }
}
