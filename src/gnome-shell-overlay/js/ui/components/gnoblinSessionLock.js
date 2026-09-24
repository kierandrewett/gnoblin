// Session-lock authority for the Gnoblin Shell integration.
//
// GNOME Shell only relinquishes ScreenShield when a compositor-backed
// coordinator has already proved that it can enforce a lock.  This is an
// intentionally startup-only decision: changing D-Bus ownership halfway
// through a Shell process could leave two owners or no secure fallback.
import Gio from "gi://Gio";
import GLib from "gi://GLib";

const REQUIRED_CAPABILITY_VERSION = 1;
const BROKER_NAME = "org.gnoblin.Lock";
const BROKER_PATH = "/org/gnoblin/Lock";
const BROKER_INTERFACE = "org.gnoblin.Lock";
const DBUS_NAME = "org.freedesktop.DBus";
const DBUS_PATH = "/org/freedesktop/DBus";
const PROPERTIES_INTERFACE = "org.freedesktop.DBus.Properties";
const QUERY_TIMEOUT_MS = 1000;

let authoritative = false;

function nameOwner(name) {
    return Gio.DBus.session
        .call_sync(
            DBUS_NAME,
            DBUS_PATH,
            DBUS_NAME,
            "GetNameOwner",
            new GLib.Variant("(s)", [name]),
            new GLib.VariantType("(s)"),
            Gio.DBusCallFlags.NONE,
            QUERY_TIMEOUT_MS,
            null,
        )
        .deepUnpack()[0];
}

function brokerOwnsCompatibilityNames() {
    const brokerOwner = nameOwner(BROKER_NAME);
    const ready = Gio.DBus.session
        .call_sync(
            BROKER_NAME,
            BROKER_PATH,
            PROPERTIES_INTERFACE,
            "Get",
            new GLib.Variant("(ss)", [BROKER_INTERFACE, "CompatibilityReady"]),
            new GLib.VariantType("(v)"),
            Gio.DBusCallFlags.NONE,
            QUERY_TIMEOUT_MS,
            null,
        )
        .deepUnpack()[0]
        .deepUnpack();
    return (
        ready === true &&
        ["org.gnome.ScreenSaver", "org.gnome.Shell.ScreenShield"].every((name) => nameOwner(name) === brokerOwner)
    );
}

// Return true only for a specifically opted-in Gnoblin session with a native
// compositor capability and an external broker that has already acquired the
// compatibility API names. GNOME Shell and Mutter share a process, so this
// code never synchronously queries Mutter's own D-Bus endpoint.
export function shouldReplaceScreenShield() {
    if (global.session_mode !== "gnoblin" || GLib.getenv("GNOBLIN_SESSION_LOCK_CUTOVER") !== "1" || authoritative)
        return authoritative;

    try {
        const capability = global.backend.get_gnoblin_session_lock_capability?.() ?? 0;
        const launcherReady = global.backend.get_gnoblin_session_lock_launcher_ready?.() === true;
        authoritative = capability >= REQUIRED_CAPABILITY_VERSION && launcherReady && brokerOwnsCompatibilityNames();
        return authoritative;
    } catch (error) {
        console.debug(`gnoblin-session-lock: retaining ScreenShield (${error.message})`);
        return false;
    }
}

// The single lock predicate for Gnoblin-owned bridge and console operations.
// `stockLocked` keeps the normal GNOME path authoritative until cutover.
export function isLocked(stockLocked = false) {
    // The native active-state accessor will be added with the capability. Until
    // then `authoritative` is always false, so this is exactly GNOME's guard.
    return stockLocked || (authoritative && global.backend.get_gnoblin_session_lock_active?.() === true);
}

export function isAuthoritative() {
    return authoritative;
}

// The native request seam is deliberately unavailable until Mutter implements
// the complete state machine. A rejected request never falls through to a
// second ScreenShield owner after cutover.
export function requestLock(reason = "shell") {
    if (!authoritative) return false;
    try {
        return global.backend.request_gnoblin_session_lock?.(reason) === true;
    } catch (error) {
        console.warn(`gnoblin-session-lock: lock request failed: ${error.message}`);
        return false;
    }
}
