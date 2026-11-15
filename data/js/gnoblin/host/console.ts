// What the console shows of the host's own objects. A client is the short block of what a person asks about a
// window; nested inside another value (a collection's member) it is only its id, app and title.

import { Display } from "gnoblin/display/display";
import { OutputProfile } from "gnoblin/display/output_profile";
import { Client } from "gnoblin/host/client";
import { Collection } from "gnoblin/host/collections";
import { Output } from "gnoblin/host/output";
import { focusedAmong } from "gnoblin/host/seat";
import { Workspace } from "gnoblin/host/workspace";
import { inspect } from "gnoblin/inspect";

/** Registers how the console renders each kind of host object. */
export function registerInspectors(): void {
    // What the console shows of the host's own objects. A client is the short block of what a person asks about a window;
    // nested inside another value (a collection's member) it is only its id, app and title.
    // Concise client fields shown by the console inspector.
    inspect.register(Client, (client) => ({
        name: "Client",
        id: client.id,
        fields: {
            appId: client.appId,
            title: client.title,
            focused: focusedAmong([client]) === client,
            fullscreen: client.fullscreen,
            floating: client.floating,
            x: client.x,
            y: client.y,
            width: client.width,
            height: client.height,
            workspace: client.workspace?.id ?? null,
            ...(client.alive ? {} : { alive: false }),
        },
        brief: { appId: client.appId, title: client.title },
    }));

    inspect.register(Workspace, (workspace) => ({
        name: "Workspace",
        id: workspace.id,
        fields: {
            name: workspace.name,
            index: workspace.index,
            active: workspace.active,
            layout: workspace.layout.constructor.name,
            clients: workspace.clients.length,
        },
        brief: { name: workspace.name, active: workspace.active },
    }));

    inspect.register(Output, (output) => ({
        name: "Output",
        id: output.id,
        fields: {
            name: output.name,
            mode: output.mode,
            scale: output.scale,
            primary: output.primary,
            x: output.x,
            y: output.y,
            width: output.width,
            height: output.height,
            workspace: output.workspace?.id ?? null,
            ...(output.connected ? {} : { connected: false }),
        },
        brief: { name: output.name, mode: output.mode },
    }));

    inspect.register(Display, (display) => ({
        name: "Display",
        id: display.id,
        fields: {
            mode: display.mode,
            outputs: display.outputs.length,
            workspace: (display.viewport.content as Workspace | null)?.id ?? null,
        },
        brief: { mode: display.mode },
    }));

    inspect.register(OutputProfile, (profile) => ({
        name: "OutputProfile",
        id: profile.id,
        fields: { mode: profile.mode ?? null, scale: profile.scale ?? null, enabled: profile.enabled ?? null },
        brief: { id: profile.id },
    }));

    inspect.register(Collection, (collection) => ({
        name: "Collection",
        fields: {},
        items: [...collection],
    }));
}
