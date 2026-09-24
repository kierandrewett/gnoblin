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

function load({
    cutover = "",
    capability = 0,
    launcherReady = false,
    compatibilityReady = false,
    matchingOwners = true,
    active = false,
} = {}) {
    const brokerOwner = ":1.42";
    const context = vm.createContext({
        global: {
            session_mode: "gnoblin",
            backend: {
                get_gnoblin_session_lock_capability: () => capability,
                get_gnoblin_session_lock_launcher_ready: () => launcherReady,
                get_gnoblin_session_lock_active: () => active,
            },
        },
        GLib: {
            getenv: () => cutover,
            Variant: class {
                constructor(_signature, values) {
                    this.values = values;
                }

                deepUnpack() {
                    return this.values;
                }
            },
            VariantType: class {},
        },
        Gio: {
            DBus: {
                session: {
                    call_sync(_name, _path, _interface, method, parameters) {
                        if (method === "GetNameOwner") {
                            const [name] = parameters.deepUnpack();
                            const owner = matchingOwners || name === "org.gnoblin.Lock" ? brokerOwner : ":1.99";
                            return { deepUnpack: () => [owner] };
                        }
                        return { deepUnpack: () => [{ deepUnpack: () => compatibilityReady }] };
                    },
                },
            },
            DBusCallFlags: { NONE: 0 },
        },
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
assert.equal(
    load({
        cutover: "1",
        capability: 1,
        launcherReady: true,
        compatibilityReady: true,
        matchingOwners: false,
    }).shouldReplaceScreenShield(),
    false,
    "the broker must own both compatibility names before Shell relinquishes them",
);

assert.equal(
    load({ cutover: "1", capability: 1, compatibilityReady: true }).shouldReplaceScreenShield(),
    false,
    "the compositor must prove the trusted lock launcher is ready",
);

const coordinator = load({ cutover: "1", capability: 1, launcherReady: true, compatibilityReady: true });
assert.equal(coordinator.shouldReplaceScreenShield(), true);
assert.equal(coordinator.isAuthoritative(), true);
assert.equal(coordinator.isLocked(true), true, "stock GNOME lock state remains a guard during fallback");
const covering = load({ cutover: "1", capability: 1, launcherReady: true, compatibilityReady: true, active: true });
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
assert.match(patch, /js\/ui\/screenshot\.js/);
assert.match(patch, /GnoblinSessionLock\.isLocked\(Main\.sessionMode\.isLocked\)/);
assert.match(patch, /Screenshot unavailable while the session is locked/);
assert.doesNotMatch(
    source,
    /get_gnoblin_session_lock_coordinator_ready/,
    "Shell must not trust a hypothetical native broker-ready claim",
);
console.log(
    "PASS: Shell lock cutover is opt-in, native-capability-gated, and routes System Actions once authoritative",
);
