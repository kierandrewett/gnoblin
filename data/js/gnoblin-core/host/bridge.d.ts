import type { Host } from "gnoblin-core/host/contract";
import type { NodeId } from "gnoblin-core/types";
/** Shell-provided operations for reading and changing compositor objects. */
export declare const host: Host;
/**
 * The host's reads, writes and calls as the shell handed them over, before anything wrapped them. A package that puts a
 * layer in front of them (the transition staging of `gnoblin-std`) wraps these, not whatever `host` holds by then.
 */
export declare const native: {
    get: (node: NodeId, key: string) => import("gnoblin-core/types").HostValue | undefined;
    set: (node: NodeId, key: string, value: import("gnoblin-core/types").HostValue) => string | undefined;
    animate: (node: NodeId, key: string, value: import("gnoblin-core/types").HostValue, duration: number, samples: number[], delay?: number, tween?: number) => string | undefined;
    check: (node: NodeId, key: string, value: import("gnoblin-core/types").HostValue, timed: boolean) => string | undefined;
    call: (node: NodeId, method: string) => string | undefined;
    raise: (node: NodeId) => string | undefined;
    lower: (node: NodeId) => string | undefined;
    api: {
        spawn: (argv: string, options: string) => string;
        kill: (id: number, signal: string) => string | undefined;
        begin: (verb: string) => string;
    };
};
/**
 * Tells property bindings of every read and write the host makes: a binding that read a property runs again when the
 * property changes. Called once, before anything uses `host`; a package that replaces `host.get` and `host.set` keeps
 * the same promise.
 */
export declare function installTracking(): void;
/** Formats a host node for errors from forbidden staged operations. */
export declare function nodeName(node: NodeId): string;
/** Construction token for native objects; scripts cannot create compositor-managed objects. */
export declare const CREATE: unique symbol;
