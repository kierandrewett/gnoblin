// The shell-installed host bridge and the token used to construct native objects.

import type { Host } from "gnoblin/host/contract";
import type { NodeId } from "gnoblin/types";

/** Returns the shell-installed bridge or fails when it is unavailable. */
function requireHost(host: Host | undefined): Host {
    if (!host) {
        throw new Error("the gnoblin module needs the shell host and cannot run without it");
    }
    return host;
}

/** Shell-provided operations for reading and changing compositor objects. */
export const host = requireHost((globalThis as { __gnoblinHost?: Host }).__gnoblinHost);

/** Formats a host node for errors from forbidden staged operations. */
export function nodeName(node: NodeId): string {
    return `${host.kind(node) === "screen" ? "Output" : "Client"}#${host.label(node)}`;
}

/** Construction token for native objects; scripts cannot create compositor-managed objects. */
export const CREATE = Symbol("gnoblin.create");
