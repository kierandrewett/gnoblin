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
        for (const uuid of em.getUuids()) {
            const ext = em.lookup(uuid);
            if (!ext || ext.state !== ExtensionState.ACTIVE)
                continue;
            try {
                em.disableExtension(uuid);
                em.enableExtension(uuid);
            } catch (e) {
                logError(e, `gnoblin: soft-reload of ${uuid} failed`);
            }
        }
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
    <!-- Shell + protocol version string, e.g. "49.6-gnoblin". -->
    <method name="GetVersion">
      <arg type="s" direction="out" name="version"/>
    </method>
    <!-- Soft in-process reload (theme + extensions). Wayland-safe: keeps windows. -->
    <method name="Reload"/>
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
    }

    enable() {
        this._impl = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._impl.export(Gio.DBus.session, OBJECT_PATH);

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
        if (this._nameId) {
            Gio.bus_unown_name(this._nameId);
            this._nameId = 0;
        }
        if (this._impl) {
            this._impl.unexport();
            this._impl = null;
        }
        console.log('gnoblin-control: disabled');
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
