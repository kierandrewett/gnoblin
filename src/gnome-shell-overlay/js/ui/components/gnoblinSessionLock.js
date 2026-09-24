// Session-lock authority for the Gnoblin Shell integration.
//
// GNOME Shell only relinquishes ScreenShield when Mutter has already proved it
// can enforce a standard session lock. This is an intentionally startup-only
// decision: changing lock ownership halfway through a Shell process could leave
// no secure fallback.
const REQUIRED_CAPABILITY_VERSION = 1;

let authoritative = false;

// The capability is zero until Mutter exposes a secure ext-session-lock-v1
// global. It deliberately has no launcher, policy, or D-Bus ownership meaning.
export function shouldReplaceScreenShield() {
    if (global.session_mode !== "gnoblin" || authoritative) return authoritative;

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
    return stockLocked || (authoritative && global.backend.get_gnoblin_session_lock_active?.() === true);
}

export function isAuthoritative() {
    return authoritative;
}
