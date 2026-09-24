// Session-lock authority for the Gnoblin Shell integration.
//
// GNOME Shell only relinquishes ScreenShield when a compositor-backed
// coordinator has already proved that it can enforce a lock.  This is an
// intentionally startup-only decision: changing D-Bus ownership halfway
// through a Shell process could leave two owners or no secure fallback.
import GLib from "gi://GLib";

const REQUIRED_CAPABILITY_VERSION = 1;

let authoritative = false;

// Return true only for a specifically opted-in Gnoblin session with a native
// compositor capability. GNOME Shell and Mutter share a process, so a
// synchronous session-bus query here could deadlock startup. Mutter must not
// expose this capability until its server and coordinator contract are ready.
export function shouldReplaceScreenShield() {
    if (global.session_mode !== "gnoblin" || GLib.getenv("GNOBLIN_SESSION_LOCK_CUTOVER") !== "1" || authoritative)
        return authoritative;

    try {
        const capability = global.backend.get_gnoblin_session_lock_capability?.() ?? 0;
        authoritative = capability >= REQUIRED_CAPABILITY_VERSION;
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
