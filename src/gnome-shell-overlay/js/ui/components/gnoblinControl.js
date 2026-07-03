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

// Runtime feature toggles. Each feature gates a gnome-shell subsystem so an
// external userspace (Quickshell, waybar, custom) can own it instead — live, with
// no compositor restart. apply(false) turns the subsystem OFF; apply(true) restores
// stock behaviour. Gating is a method-shadow (own property) that a `delete` undoes,
// so it never mutates upstream prototypes permanently. A feature is ENABLED unless
// its id is in the org.gnoblin.shell 'disabled-features' list.
const FEATURES = {
    osd: {
        summary: 'On-screen display popups (volume / brightness)',
        apply(enabled) {
            const mgr = Main.osdWindowManager;
            if (!mgr)
                return;
            // Gate the common chokepoint: BOTH show() and showAll() funnel through
            // _showOsdWindow() (osdWindow.js). show() alone would miss showAll(),
            // which is what the volume OSD uses.
            if (enabled)
                delete mgr._showOsdWindow;   // restore prototype method
            else
                mgr._showOsdWindow = () => {}; // swallow every OSD request
        },
    },
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

        this._nameId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            () => console.log(`gnoblin-control: acquired ${BUS_NAME} at ${OBJECT_PATH}`),
            () => console.warn(`gnoblin-control: lost ${BUS_NAME} (another owner?)`));

        console.log(`gnoblin-control: enabled (mode=${this._mode()}, wayland=${Meta.is_wayland_compositor()})`);
    }

    disable() {
        // Restore every gated subsystem to stock before we go.
        for (const id of Object.keys(FEATURES))
            FEATURES[id].apply(true);

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
