import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(
    new URL("../src/gnome-shell-overlay/js/ui/components/gnoblinSessionLock.js", import.meta.url),
    "utf8",
)
    .replace(/^import .*;\n/gm, "")
    .replace(/export function /g, "function ")
    .concat("\nthis.api = { shouldReplaceScreenShield, isLocked, isAuthoritative };\n");

function load({ cutover = "", capability = 0, coordinatorReady = false, active = false } = {}) {
    const context = vm.createContext({
        global: {
            session_mode: "gnoblin",
            backend: {
                get_gnoblin_session_lock_capability: () => capability,
                get_gnoblin_session_lock_coordinator_ready: () => coordinatorReady,
                get_gnoblin_session_lock_active: () => active,
            },
        },
        GLib: { getenv: () => cutover },
        console,
    });
    vm.runInContext(source, context);
    return context.api;
}

assert.equal(load().shouldReplaceScreenShield(), false, "cutover remains off without its explicit environment switch");
assert.equal(
    load({ cutover: "1" }).shouldReplaceScreenShield(),
    false,
    "an unavailable native capability cannot replace ScreenShield",
);

assert.equal(
    load({ cutover: "1", capability: 1 }).shouldReplaceScreenShield(),
    false,
    "the compositor capability cannot replace ScreenShield before the coordinator owns compatibility services",
);

const coordinator = load({ cutover: "1", capability: 1, coordinatorReady: true });
assert.equal(coordinator.shouldReplaceScreenShield(), true);
assert.equal(coordinator.isAuthoritative(), true);
assert.equal(coordinator.isLocked(true), true, "stock GNOME lock state remains a guard during fallback");
const covering = load({ cutover: "1", capability: 1, coordinatorReady: true, active: true });
covering.shouldReplaceScreenShield();
assert.equal(covering.isLocked(false), true, "the native active state blocks bridge work from covering onward");

const patch = readFileSync(
    new URL(
        "../patches/gnome-shell/90-session-lock/0001-gnoblin-gate-session-lock-cutover-on-compositor-authority.patch",
        import.meta.url,
    ),
    "utf8",
);
assert.match(patch, /!GnoblinSessionLock\.shouldReplaceScreenShield\(\)/);
assert.match(patch, /GnoblinSessionLock\.requestLock\('system-actions'\)/);
assert.match(patch, /screenShield !== null \|\| GnoblinSessionLock\.isAuthoritative\(\)/);
assert.match(patch, /shouldShowInMode && !GnoblinSessionLock\.isAuthoritative\(\)/);
assert.match(patch, /if \(GnoblinSessionLock\.isAuthoritative\(\)\)\n\+            return;/);
assert.doesNotMatch(
    source,
    /DBusProxy\.new_sync/,
    "Shell startup must not synchronously query a service in its own process",
);
console.log(
    "PASS: Shell lock cutover is opt-in, native-capability-gated, and routes System Actions once authoritative",
);
