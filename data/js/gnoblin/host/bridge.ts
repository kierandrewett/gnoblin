// The shell's host bridge, captured once, and the token only this library holds.
//
// `host` is the object the Rust shell installed before it loaded this library; every module reads and writes nodes
// through it (after `installStaging` has put the transition layer in front of its functions). `CREATE` is what a
// native object's constructor demands, so a script cannot make a `Client` or an `Output`: the compositor owns their
// lifetime.

import type { Host } from "gnoblin/host/contract";
import type { NodeId } from "gnoblin/types";

/** The shell-installed bridge; missing host functions make this module unusable. */
function requireHost(host: Host | undefined): Host {
    if (!host) {
        throw new Error("the gnoblin module needs the shell host and cannot run without it");
    }
    return host;
}

export const host = requireHost((globalThis as { __gnoblinHost?: Host }).__gnoblinHost);

/** Names a host node for errors raised when a staged operation is forbidden. */
export function nodeName(node: NodeId): string {
    return `${host.kind(node) === "screen" ? "Output" : "Client"}#${host.label(node)}`;
}

/** Only this module can make a Client, a Workspace or a Collection: the compositor owns their lifetime. */
export const CREATE = Symbol("gnoblin.create");
