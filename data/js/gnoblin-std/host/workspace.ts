// A workspace is a place for laying out windows; the compositor does not know it, `libgnoblin-std` does.
//
// A `Workspace` is the script's handle on the library's workspace: its id and name, its gaps, the windows on it and the output
// that shows it are the library's, read when asked. What the object adds is what only a script has: the `Layout` instance the
// library asks to place the windows (`host/layout_bridge`), the live `gaps` object, `clients` as a `MutableCollection` of the
// engine's `Client` objects (`host/membership` keeps its list equal to the library's), and the reactive `properties`.

import { CREATE } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { collectionState, rootState } from "gnoblin-core/host/collection_state";
import { MutableCollection } from "gnoblin-core/host/collections";
import { ctx } from "gnoblin-core/host/context";
import type { Output } from "gnoblin-core/host/output";
import { NativeObject } from "gnoblin-core/native_object";
import { propertiesOf } from "gnoblin-core/properties";
import type { PropertyBinding, PropertyBindings } from "gnoblin-core/properties";
import { EventKind } from "gnoblin-std/generated/constants";
import { Workspace as NativeWorkspace } from "gnoblin-std/generated/classes";
import { bridgeLayout } from "gnoblin-std/host/layout_bridge";
import { throughLibrary } from "gnoblin-std/host/library";
import { clearClients, detachClient, swapClients, syncMembership, transferClient } from "gnoblin-std/host/membership";
import { showWorkspace } from "gnoblin-std/host/show";
import type { ShowOptions } from "gnoblin-std/host/show";
import { std } from "gnoblin-std/host/state";
import { wsState } from "gnoblin-std/host/workspace_state";
import type { WorkspaceState } from "gnoblin-std/host/workspace_state";
import type { Gaps, Layout } from "gnoblin-std/layout";
import { FloatingLayout } from "gnoblin-std/layouts/floating";
import { setProperties } from "gnoblin-std/transition/engine";
import type { Transition } from "gnoblin-std/transition/transition";
import type { TransitionOptions } from "gnoblin-std/transition/types";
import { checkGaps, checkWorkspaceOptions, resolveLayout } from "gnoblin-std/workspace";
import type { StockLayoutName, WorkspaceOptions } from "gnoblin-std/workspace";

/** The workspace the library made that `new Workspace` is to wrap, set only for the moment `wrapWorkspace` constructs it. */
let adopting: NativeWorkspace | null = null;

/**
 * A place windows are laid out in. The script makes it and adds it to `gnoblin.workspaces`; `clients` holds
 * its windows, the same `Client` objects as `gnoblin.clients`. Removing a workspace, or a window from one,
 * never closes the window.
 */
export class Workspace extends NativeObject {
    constructor(options: WorkspaceOptions) {
        const wrapped = adopting;
        adopting = null;
        super();
        if (wrapped !== null) {
            const info = wrapped.info();
            this.#state(wrapped, info.id, new FloatingLayout(), { inner: info.gapInner, outer: info.gapOuter });
            return;
        }
        const checked = checkWorkspaceOptions(options);
        const bridge = bridgeLayout(checked.layout, () => this);
        // A workspace of this id that an earlier generation left in the library is this one: it keeps its windows and its place.
        const left = [...std.inherited].find((workspace) => workspace.id === checked.id);
        if (left !== undefined) {
            const held = wsState(left).native;
            std.inherited.delete(left);
            held.setLayout(bridge.spec);
            held.setGaps(checked.gaps.inner, checked.gaps.outer);
            held.setName(checked.name);
            this.#state(held, checked.id, checked.layout, checked.gaps, bridge);
            return;
        }
        const native = throughLibrary("Workspace", {}, () =>
            NativeWorkspace.new({
                id: checked.id,
                name: checked.name,
                gapInner: checked.gaps.inner,
                gapOuter: checked.gaps.outer,
                layout: bridge.spec,
            }),
        );
        if (native === null) {
            throw new TypeError("Workspace: the library made no workspace");
        }
        this.#state(native, checked.id, checked.layout, checked.gaps, bridge);
    }

    /** Gives the object its state and the registrations that keep its clients equal to the library's. */
    #state(
        native: NativeWorkspace,
        id: string,
        layout: Layout,
        gaps: Gaps,
        bridge = bridgeLayout(layout, () => this),
    ): void {
        const nativeClients = native.clients();
        if (nativeClients === null) {
            throw new TypeError("Workspace: the library gave the workspace no clients");
        }
        const clients = new MutableCollection<Client>(CREATE, {
            ...rootState("Client"),
            mutation: {
                add: (client) => transferClient(this, client as Client),
                remove: (client) => detachClient(this, client as Client),
                clear: () => clearClients(this),
                swapped: (a, b) => swapClients(this, a as Client, b as Client),
            },
        });
        const state: WorkspaceState = {
            native,
            id,
            layout,
            bridge,
            gaps,
            gapsSeen: [gaps.inner, gaps.outer],
            attached: false,
            nativeClients,
            clients,
            clientsState: collectionState(clients),
        };
        std.wsStates.set(this, state);
        std.byHandle.set(native.handle, this);
        // What the library does to the windows (a new window, a closed one, a move to another workspace) reaches the list.
        nativeClients.listen(EventKind.ADDED, 0, syncMembership);
        nativeClients.listen(EventKind.REMOVED, 0, syncMembership);
    }

    /** Wraps a workspace the library made itself (the default `desktop`, a per-output policy's): the one object for its handle. */
    static wrap(native: NativeWorkspace): Workspace {
        adopting = native;
        return new Workspace(undefined as never);
    }

    /** Stable workspace id used to find it across reloads. */
    get id(): string {
        return wsState(this).id;
    }

    /** Display name shown by workspace controls. */
    get name(): string {
        return wsState(this).native.info().name;
    }

    /** Sets the workspace display name. */
    set name(value: string) {
        if (typeof value !== "string") {
            throw new TypeError(`${this}.name must be a string`);
        }
        wsState(this).native.setName(value);
    }

    /** A workspace is the script's own and stays usable after it leaves `gnoblin.workspaces`. */
    get alive(): boolean {
        return true;
    }

    /** The position in `gnoblin.workspaces`, or -1 while it is not in it. */
    get index(): number {
        return wsState(this).native.info().index;
    }

    /** Whether an output shows this workspace. With several outputs, several workspaces are active. */
    get active(): boolean {
        return wsState(this).native.info().active !== 0;
    }

    /** The output that shows this workspace, or null while none does. */
    get output(): Output | null {
        const node = wsState(this).native.info().output;
        return node === 0 ? null : (ctx.outputsByNode.get(node) ?? null);
    }

    /** The layout instance used to place this workspace's windows. */
    get layout(): Layout {
        return wsState(this).layout;
    }

    /** A layout instance, or the name of a stock layout. The windows stay where they are until it computes. */
    set layout(value: Layout | StockLayoutName) {
        const state = wsState(this);
        const layout = resolveLayout(value, `${this}`);
        const bridge = bridgeLayout(layout, () => this);
        state.native.setLayout(bridge.spec);
        state.layout = layout;
        state.bridge = bridge;
    }

    /** Inner and outer gaps, in logical pixels. */
    get gaps(): Gaps {
        return wsState(this).gaps;
    }

    /** Sets inner and outer gaps, in logical pixels. */
    set gaps(value: { inner?: number; outer?: number }) {
        const state = wsState(this);
        state.gaps = checkGaps(value, `${this}.gaps`);
        state.native.setGaps(state.gaps.inner, state.gaps.outer);
        state.gapsSeen = [state.gaps.inner, state.gaps.outer];
    }

    /** Mutable collection of this workspace's windows. */
    get clients(): MutableCollection<Client> {
        return wsState(this).clients;
    }

    /**
     * Assigns `props` (`layout`, `gaps`, `name`) as one `requestTransition`: the windows tween from where they are
     * drawn to where the new layout puts them. Without `options` the change is atomic and immediate.
     */
    set(props: Record<string, unknown>, options?: TransitionOptions): Transition {
        return setProperties(std.transitions, this, `${this}`, props, options);
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
     * Moves the keyboard focus `step` windows along the workspace's windows, in the order they are listed: `1` to
     * the next, `-1` to the previous. Nothing moves when no window of the workspace has the focus, and a step past
     * either end leaves the focus where it is.
     */
    walk(step: number): void {
        if (typeof step !== "number" || !Number.isInteger(step)) {
            throw new TypeError(`${this}.walk: step must be a whole number, got ${String(step)}`);
        }
        wsState(this).native.walk(step);
    }

    /** Returns a label containing the workspace id. */
    toString(): string {
        return `Workspace#${wsState(this).id}`;
    }
}
