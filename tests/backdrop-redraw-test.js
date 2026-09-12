import Clutter from "gi://Clutter";
import { BackdropRedraw } from "../src/gnome-shell-overlay/js/ui/components/gnoblinBackdropRedraw.js";
function assert(value, message) {
    if (!value) throw new Error(message);
}
class Actor {
    constructor() {
        this.mapped = true;
        this.signals = new Map();
        this.nextId = 1;
    }
    connect(name, callback) {
        const id = this.nextId++;
        this.signals.set(id, { name, callback });
        return id;
    }
    disconnect(id) {
        this.signals.delete(id);
    }
    emit(name) {
        for (const entry of [...this.signals.values()]) if (entry.name === name) entry.callback();
    }
}
const flag = Clutter.DrawDebugFlag.DISABLE_CLIPPED_REDRAWS;
const enabled = () => !!(Clutter.get_debug_flags()[1] & flag);
const initial = enabled();
Clutter.remove_debug_flags(0, flag, 0);
const policy = new BackdropRedraw();
try {
    const first = new Actor(),
        second = new Actor();
    policy.set(first, true);
    assert(enabled(), "mapped blurred surface enables full redraw");
    policy.set(second, true);
    first.mapped = false;
    first.emit("notify::mapped");
    assert(enabled(), "another mapped blurred surface retains full redraw");
    second.emit("destroy");
    assert(!enabled(), "last blurred surface closing restores partial redraw");
    first.mapped = true;
    first.emit("notify::mapped");
    assert(enabled(), "reopening enables full redraw again");
    policy.set(first, false);
    assert(!enabled(), "removing blur restores partial redraw");
    const native = new Actor();
    native.get_effect = () => ({ uses_damage_tracking: () => true });
    policy.set(native, true);
    assert(!enabled(), "native damage tracking keeps partial redraw enabled");
    assert(native.signals.size === 0, "native damage tracking needs no fallback signal handlers");
    policy.set(first, true);
    assert(enabled(), "mixed native and legacy effects retain the fallback");
    policy.set(first, false);
    assert(!enabled(), "removing the legacy effect restores native damage tracking");
    policy.set(native, false);
    Clutter.add_debug_flags(0, flag, 0);
    policy.set(first, true);
    policy.destroy();
    assert(enabled(), "does not remove a flag owned by another component");
    print("PASS: backdrop redraw lifetime, multiple surfaces, reopen, rule removal and existing flags");
} finally {
    policy.destroy();
    if (initial) Clutter.add_debug_flags(0, flag, 0);
    else Clutter.remove_debug_flags(0, flag, 0);
}
