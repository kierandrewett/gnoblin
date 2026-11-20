import type { Client } from "gnoblin-core/host/client";
import type { WindowPointerEvent } from "gnoblin-core/host/contract";
import type { NodeId } from "gnoblin-core/types";
/** Raises the events one client's changed properties imply, to the listeners that held it before the change. */
export declare function changed(client: Client): void;
/** Processes one batch of host tree changes and returns the queued callback count. */
export declare function plan(created: NodeId[], touched: NodeId[], destroyed: NodeId[]): number;
/** Queues the clients' pointer listeners for the edges the compositor reported on windows this frame. */
export declare function pointer(events: WindowPointerEvent[]): number;
/** Queues `onReady` for the clients that were there at boot, parents first, once the config has loaded. */
export declare function ready(nodes: NodeId[]): number;
