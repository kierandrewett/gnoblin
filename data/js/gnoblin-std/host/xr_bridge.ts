// Connects the host's scene nodes, clients and input contexts to the scene and XR modules.

import { host } from "gnoblin-core/host/bridge";
import { Client } from "gnoblin-core/host/client";
import { ctx } from "gnoblin-core/host/context";
import { hiddenByLayout } from "gnoblin-std/host/client_state";
import { std } from "gnoblin-std/host/state";
import { stateOf } from "gnoblin-core/host/node_state";
import { mainSeat } from "gnoblin-core/host/seat";
import { Workspace } from "gnoblin-std/host/workspace";
import { attachScene } from "gnoblin-std/scene/backend";
import type { SceneBackend } from "gnoblin-std/scene/backend";
import { XRRuntime } from "gnoblin-std/extensions/xr/runtime";
import type { XRWorld } from "gnoblin-std/extensions/xr/types";

/** Attaches the scene and makes the XR runtime. */
export function installScene(): void {
    const sceneBackend: SceneBackend = {
        create: (cls) => host.xr.create(cls),
        insert: (node, parent, index) => host.xr.insert(node, parent, index),
        destroy: (node) => host.xr.destroy(node),
        children: (node) => host.xr.children(node),
        parent: (node) => host.xr.parent(node),
        root: () => host.xr.root(),
        get: (node, key) => host.get(node, key),
        set: (node, key, value) => host.set(node, key, value),
        nodeOf: (content) => (content instanceof Client ? stateOf(content).node : undefined),
        contentOf: (node) => ctx.clientsByNode.get(node),
        workspace: (content) =>
            content instanceof Workspace
                ? {
                      clients: () =>
                          [...content.clients].map((client) => {
                              // Reading `visible` is what makes a view follow the layout, which writes it.
                              void client.visible;
                              return {
                                  node: stateOf(client).node,
                                  x: client.x,
                                  y: client.y,
                                  width: client.width,
                                  height: client.height,
                                  visible: !client.minimised && !hiddenByLayout(client),
                              };
                          }),
                      area: () => {
                          const output = content.output;
                          return output === null
                              ? null
                              : { x: output.x, y: output.y, width: output.width, height: output.height };
                      },
                  }
                : undefined,
    };

    attachScene(sceneBackend);

    // XR view into host nodes, seats, clients, and input contexts.
    const xrWorld: XRWorld = {
        scene: sceneBackend,
        nodes: (cls) => host.xr.nodes(cls),
        kind: (node) => host.kind(node),
        seats: () => host.input.seats().map((node) => ({ node, name: host.input.seatName(node) })),
        focusedClient() {
            const seat = mainSeat();
            return seat === undefined ? null : (ctx.clientsByNode.get(host.input.focus(seat)) ?? null);
        },
        clients: () => [...ctx.clientRoot.list] as Client[],
        parentKind(client) {
            const parent = host.xr.parent(stateOf(client).node);
            return parent === undefined ? undefined : host.kind(parent);
        },
        inputContext(seat) {
            const name = host.input.seatName(seat);
            return ctx.input.contexts.find((context) => context.name === name) ?? null;
        },
        declared: () => host.xr.declared(),
        now: () => host.input.now(),
        report: (site, error) => {
            ctx.reports.push({ site, error });
        },
    };

    // Runtime for the optional XR extension, attached to the scene and host adapters.
    std.xrRuntime = new XRRuntime(xrWorld);
}
