// `Workspace`: a place windows are laid out in. The script makes it and adds it to `gnoblin.workspaces`; the
// compositor does not know it.

import { CREATE } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { collectionState, rootState } from "gnoblin/host/collection_state";
import { MutableCollection } from "gnoblin/host/collections";
import { ctx } from "gnoblin/host/context";
import { deskShowing, isShown } from "gnoblin/host/desks";
import { clearClients, detachClient, swapClients, transferClient } from "gnoblin/host/membership";
import type { Output } from "gnoblin/host/output";
import { hasFocus } from "gnoblin/host/seat";
import { showWorkspace } from "gnoblin/host/show";
import type { ShowOptions } from "gnoblin/host/show";
import { wsState } from "gnoblin/host/workspace_state";
import type { Gaps, Layout } from "gnoblin/layout";
import { NativeObject } from "gnoblin/native_object";
import { propertiesOf } from "gnoblin/properties";
import type { PropertyBinding, PropertyBindings } from "gnoblin/properties";
import { setProperties } from "gnoblin/transition/engine";
import type { Transition } from "gnoblin/transition/transition";
import type { TransitionOptions } from "gnoblin/transition/types";
import { checkGaps, checkWorkspaceOptions, resolveLayout } from "gnoblin/workspace";
import type { StockLayoutName, WorkspaceOptions } from "gnoblin/workspace";

/**
 * A place windows are laid out in. The script makes it and adds it to `gnoblin.workspaces`; `clients` holds
 * its windows, the same `Client` objects as `gnoblin.clients`. Removing a workspace, or a window from one,
 * never closes the window.
 */
export class Workspace extends NativeObject {
    constructor(options: WorkspaceOptions) {
        const checked = checkWorkspaceOptions(options);
        super();
        const clients = new MutableCollection<Client>(CREATE, {
            ...rootState("Client"),
            mutation: {
                add: (client) => transferClient(this, client as Client),
                remove: (client) => detachClient(this, client as Client),
                clear: () => clearClients(this),
                swapped: (a, b) => swapClients(this, a as Client, b as Client),
            },
        });
        ctx.wsStates.set(this, {
            id: checked.id,
            name: checked.name,
            layout: checked.layout,
            gaps: checked.gaps,
            attached: false,
            clients,
            clientsState: collectionState(clients),
            arrived: [],
            departed: [],
            dirty: true,
            focusSeen: null,
            coverSeen: "",
            fields: null,
            gapsSeen: [checked.gaps.inner, checked.gaps.outer],
            area: null,
        });
    }

    get id(): string {
        return wsState(this).id;
    }

    get name(): string {
        return wsState(this).name;
    }

    set name(value: string) {
        if (typeof value !== "string") {
            throw new TypeError(`${this}.name must be a string`);
        }
        wsState(this).name = value;
    }

    /** A workspace is the script's own and stays usable after it leaves `gnoblin.workspaces`. */
    get alive(): boolean {
        return true;
    }

    /** The position in `gnoblin.workspaces`, or -1 while it is not in it. */
    get index(): number {
        return ctx.workspaceRoot.list.indexOf(this);
    }

    /** Whether an output shows this workspace. With several outputs, several workspaces are active. */
    get active(): boolean {
        return isShown(this);
    }

    /** The output that shows this workspace, or null while none does. */
    get output(): Output | null {
        const desk = deskShowing(this);
        return desk !== null && !desk.parked ? desk.output : null;
    }

    get layout(): Layout {
        return wsState(this).layout;
    }

    /** A layout instance, or the name of a stock layout. The windows stay where they are until it computes. */
    set layout(value: Layout | StockLayoutName) {
        const state = wsState(this);
        state.layout = resolveLayout(value, `${this}`);
        state.fields = null;
        state.dirty = true;
    }

    get gaps(): Gaps {
        return wsState(this).gaps;
    }

    set gaps(value: { inner?: number; outer?: number }) {
        const state = wsState(this);
        state.gaps = checkGaps(value, `${this}.gaps`);
        state.dirty = true;
    }

    get clients(): MutableCollection<Client> {
        return wsState(this).clients;
    }

    /**
     * Assigns `props` (`layout`, `gaps`, `name`) as one `requestTransition`: the windows tween from where they are
     * drawn to where the new layout puts them. Without `options` the change is atomic and immediate.
     */
    set(props: Record<string, unknown>, options?: TransitionOptions): Transition {
        return setProperties(ctx.transitions, this, `${this}`, props, options);
    }

    /** The reactive properties of the workspace: `workspace.properties.bind("blurRadius", () => ...)`. */
    get properties(): PropertyBindings {
        return propertiesOf(this, () => `${this}`);
    }

    /** Shorthand for `workspace.properties.bind`. */
    bind(property: string, compute: () => unknown): PropertyBinding {
        return this.properties.bind(property, compute);
    }

    /** Shows this workspace on the output under the pointer (the primary output when no pointer says). */
    show(options?: ShowOptions): void {
        showWorkspace(this, options);
    }

    /**
     * Moves the keyboard focus `step` windows along the workspace's windows, in the order they are listed (the retired shell's
     * `Workspace:walk`): `1` to the next, `-1` to the previous. Nothing moves when no window of the workspace has the focus,
     * and a step past either end leaves the focus where it is.
     */
    walk(step: number): void {
        if (typeof step !== "number" || !Number.isInteger(step)) {
            throw new TypeError(`${this}.walk: step must be a whole number, got ${String(step)}`);
        }
        const list = wsState(this).clientsState.list as Client[];
        const at = list.findIndex((client) => hasFocus(client));
        const next = at < 0 ? undefined : list[at + step];
        if (next !== undefined && next.alive) {
            next.focus();
        }
    }

    toString(): string {
        return `Workspace#${wsState(this).id}`;
    }
}
