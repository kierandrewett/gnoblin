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

function load({ capability = 0, active = false } = {}) {
    const context = vm.createContext({
        global: {
            session_mode: "gnoblin",
            backend: {
                get_gnoblin_session_lock_capability: () => capability,
                get_gnoblin_session_lock_active: () => active,
            },
        },
        console,
    });
    vm.runInContext(source, context);
    return context.api;
}

assert.equal(load().shouldReplaceScreenShield(), false, "an insecure or unavailable protocol retains ScreenShield");
const protocol = load({ capability: 1 });
assert.equal(
    protocol.shouldReplaceScreenShield(),
    true,
    "a secure protocol replaces ScreenShield only in the Gnoblin session",
);
assert.equal(protocol.isAuthoritative(), true);
assert.equal(protocol.isLocked(true), true, "stock GNOME lock state remains a guard during fallback");
const covering = load({ capability: 1, active: true });
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
assert.match(patch, /screenShield !== null \|\| GnoblinSessionLock\.isAuthoritative\(\)/);
assert.match(
    patch,
    /showLock && allowLockScreen && LoginManager\.canLock\(\)[\s\S]*!GnoblinSessionLock\.isAuthoritative\(\)/,
);
assert.match(patch, /if \(GnoblinSessionLock\.isAuthoritative\(\)\)\n\+            return;/);
assert.match(patch, /js\/ui\/screenshot\.js/);
assert.match(patch, /GnoblinSessionLock\.isLocked\(Main\.sessionMode\.isLocked\)/);
assert.match(patch, /Screenshot unavailable while the session is locked/);
assert.doesNotMatch(
    source,
    /DBus\.session|call_sync/,
    "Shell startup must not synchronously query a service in its own process",
);
console.log(
    "PASS: Shell cutover follows only secure session-lock capability and denies Shell screenshots while active",
);
