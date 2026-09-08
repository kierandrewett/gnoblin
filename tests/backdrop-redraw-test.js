import Clutter from 'gi://Clutter';
import {BackdropRedraw} from '../src/gnome-shell-overlay/js/ui/components/gnoblinBackdropRedraw.js';
function assert(value, message) { if (!value) throw new Error(message); }
class Actor {
    constructor() { this.mapped = true; this.signals = new Map(); this.nextId = 1; }
    connect(name, callback) { const id = this.nextId++; this.signals.set(id, {name, callback}); return id; }
    disconnect(id) { this.signals.delete(id); }
    emit(name) { for (const entry of [...this.signals.values()]) if (entry.name === name) entry.callback(); }
}
const flag = Clutter.DrawDebugFlag.DISABLE_CLIPPED_REDRAWS;
const enabled = () => !!(Clutter.get_debug_flags()[1] & flag);
const initial = enabled();
Clutter.remove_debug_flags(0, flag, 0);
const policy = new BackdropRedraw();
try {
    const first = new Actor(), second = new Actor();
    policy.set(first, true);
    assert(enabled(), 'mapped menu enables full redraw');
    policy.set(second, true);
    first.mapped = false; first.emit('notify::mapped');
    assert(enabled(), 'another mapped menu retains full redraw');
    second.emit('destroy');
    assert(!enabled(), 'last menu closing restores partial redraw');
    first.mapped = true; first.emit('notify::mapped');
    assert(enabled(), 'reopening enables full redraw again');
    policy.set(first, false);
    assert(!enabled(), 'removing blur restores partial redraw');
    Clutter.add_debug_flags(0, flag, 0);
    policy.set(first, true); policy.destroy();
    assert(enabled(), 'does not remove a flag owned by another component');
    print('PASS: menu redraw lifetime, multiple menus, reopen, rule removal and existing flags');
} finally {
    policy.destroy();
    if (initial) Clutter.add_debug_flags(0, flag, 0);
    else Clutter.remove_debug_flags(0, flag, 0);
}
