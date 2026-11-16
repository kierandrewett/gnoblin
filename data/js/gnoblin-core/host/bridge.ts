// The shell-installed host bridge and the token used to construct native objects.

import type { Host } from "gnoblin-core/host/contract";
import { noteHostRead, noteHostWrite } from "gnoblin-core/properties";
import type { NodeId } from "gnoblin-core/types";

/** Returns the shell-installed bridge or fails when it is unavailable. */
function requireHost(host: Host | undefined): Host {
    if (!host) {
        throw new Error("the gnoblin module needs the shell host and cannot run without it");
    }
    return host;
}

/** Shell-provided operations for reading and changing compositor objects. */
export const host = requireHost((globalThis as { __gnoblinHost?: Host }).__gnoblinHost);

/**
 * The host's reads, writes and calls as the shell handed them over, before anything wrapped them. A package that puts a
 * layer in front of them (the transition staging of `gnoblin-std`) wraps these, not whatever `host` holds by then.
 */
export const native = {
    get: host.get,
    set: host.set,
    animate: host.animate,
    check: host.check,
    call: host.call,
    raise: host.raise,
    lower: host.lower,
    api: { spawn: host.api.spawn, kill: host.api.kill, begin: host.api.begin },
};

/**
 * Tells property bindings of every read and write the host makes: a binding that read a property runs again when the
 * property changes. Called once, before anything uses `host`; a package that replaces `host.get` and `host.set` keeps
 * the same promise.
 */
export function installTracking(): void {
    host.get = (node, key) => {
        const value = native.get(node, key);
        noteHostRead(node, key, value);
        return value;
    };
    host.set = (node, key, value) => {
        const refused = native.set(node, key, value);
        if (refused == null) {
            noteHostWrite(node, key, value);
        }
        return refused;
    };
}

/** Formats a host node for errors from forbidden staged operations. */
export function nodeName(node: NodeId): string {
    return `${host.kind(node) === "screen" ? "Output" : "Client"}#${host.label(node)}`;
}

/** Construction token for native objects; scripts cannot create compositor-managed objects. */
export const CREATE = Symbol("gnoblin.create");
