// Console representations of the objects `gnoblin-std` adds, and the fields it adds to the clients and outputs of
// `gnoblin-core`.

import { Client } from "gnoblin-core/host/client";
import { Output } from "gnoblin-core/host/output";
import { focusedAmong } from "gnoblin-core/host/seat";
import { inspect } from "gnoblin-core/inspect";
import { Display } from "gnoblin-std/display/display";
import { OutputProfile } from "gnoblin-std/display/output_profile";
import { Workspace } from "gnoblin-std/host/workspace";

/** Registers how the console renders workspaces and displays, and renders clients and outputs with their workspace. */
export function registerInspectors(): void {
    // A later registration for a class replaces the one core made, so these views carry every field.
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
}
