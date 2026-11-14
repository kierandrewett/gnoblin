// What a `View` of a workspace is made of. A workspace is a script object, not a tree node, so the compositor has no
// node to name as the View's `source`. The mirror builds the picture from what the tree can already draw: under the
// View's own node it keeps one holder node, and under the holder one child View for each client on the workspace,
// each sourcing that client's node and placed where the client sits in the workspace's area. The holder's scale and
// offset fit that area into the View's size. The clients are not moved or reparented.
//
// The mirror keeps itself current with a `watch`: it runs again when a client joins or leaves, moves or resizes, or
// when the View's size or fit changes. It owns the nodes it makes and removes them with `stop`. It does not own the
// View: `gnoblin/scene` makes the node and calls it.

import { noteSignal, untracked, watch } from "gnoblin/properties";
import type { PropertyBinding } from "gnoblin/properties";

type NodeId = number;

/** How a picture is fitted to the View's rectangle. */
export type ContentFit = "contain" | "cover" | "stretch";

/** One client of the mirrored workspace: its tree node and its rectangle in the compositor's logical pixels. */
export interface MirroredClient {
    node: NodeId;
    x: number;
    y: number;
    width: number;
    height: number;
    visible: boolean;
}

/** What the mirror reads of a workspace. Every read goes through the tracked getters, so it is followed. */
export interface WorkspaceSource {
    /** The clients on the workspace now, in stacking order. */
    clients(): MirroredClient[];
    /** The rectangle the workspace fills (its output), or null while no output shows it. */
    area(): { x: number; y: number; width: number; height: number } | null;
}

/** The tree operations the mirror needs. The scene's backend provides them. */
export interface MirrorTree {
    create(cls: string): NodeId | string;
    insert(node: NodeId, parent: NodeId, index: number): string | undefined;
    destroy(node: NodeId): string | undefined;
    get(node: NodeId, key: string): unknown;
    set(node: NodeId, key: string, value: number | boolean | number[]): string | undefined;
}

function must<T>(result: T | string | undefined, what: string): T {
    if (typeof result === "string") {
        throw new Error(`${what}: the shell refused (${result})`);
    }
    return result as T;
}

/** The scale and the offset that fit `area` into `size` under `fit`. A size with no area or no extent is left as is. */
export function fitScale(
    fit: ContentFit,
    area: { width: number; height: number },
    size: { width: number; height: number },
): { scale: [number, number, number]; x: number; y: number } {
    if (!(area.width > 0 && area.height > 0 && size.width > 0 && size.height > 0)) {
        return { scale: [1, 1, 1], x: 0, y: 0 };
    }
    const across = size.width / area.width;
    const down = size.height / area.height;
    const [sx, sy] =
        fit === "stretch"
            ? [across, down]
            : fit === "contain"
              ? [Math.min(across, down), Math.min(across, down)]
              : [Math.max(across, down), Math.max(across, down)];
    return {
        scale: [sx, sy, 1],
        x: (size.width - area.width * sx) / 2,
        y: (size.height - area.height * sy) / 2,
    };
}

/** Keeps the tree under one View node in step with one workspace. */
export class WorkspaceMirror {
    readonly #tree: MirrorTree;
    readonly #source: WorkspaceSource;
    readonly #view: NodeId;
    readonly #fit: () => ContentFit;
    readonly #children = new Map<NodeId, NodeId>();
    readonly #watch: PropertyBinding;
    #holder: NodeId | null = null;

    /**
     * Starts mirroring `source` under `view`. `owner` is the script's View, whose death ends the mirror; `site` names
     * it in a report; `fit` is read on every run, and the caller raises the signal named `fit` on `owner` when it
     * changes.
     */
    constructor(
        tree: MirrorTree,
        source: WorkspaceSource,
        view: NodeId,
        owner: object,
        site: string,
        fit: () => ContentFit,
    ) {
        this.#tree = tree;
        this.#source = source;
        this.#view = view;
        this.#fit = fit;
        this.#watch = watch(owner, site, () => this.#sync(owner));
    }

    /** Ends the mirror and removes the nodes it made. */
    stop(): void {
        this.#watch.dispose();
        if (this.#holder !== null) {
            this.#tree.destroy(this.#holder);
            this.#holder = null;
            this.#children.clear();
        }
    }

    /** Writes `key` unless the node already has `value`, so a steady picture costs no operations. */
    #write(node: NodeId, key: string, value: number | boolean | number[]): void {
        const have = untracked(() => this.#tree.get(node, key));
        const same = Array.isArray(value)
            ? Array.isArray(have) && have.length === value.length && have.every((part, at) => part === value[at])
            : have === value;
        if (!same) {
            must(this.#tree.set(node, key, value), `the workspace picture's ${key}`);
        }
    }

    #sync(owner: object): void {
        noteSignal(owner, "fit");
        const items = this.#source.clients();
        const area = this.#source.area() ?? boundingBox(items);
        const size = {
            width: Number(this.#tree.get(this.#view, "width")),
            height: Number(this.#tree.get(this.#view, "height")),
        };
        if (this.#holder === null) {
            const made = must(this.#tree.create("Node"), "the workspace picture");
            must(this.#tree.insert(made, this.#view, 0), "the workspace picture");
            this.#holder = made;
        }
        const holder = this.#holder;
        const fitted = fitScale(this.#fit(), area, size);
        this.#write(holder, "x", fitted.x);
        this.#write(holder, "y", fitted.y);
        this.#write(holder, "scale", fitted.scale);
        const seen = new Set<NodeId>();
        for (const item of items) {
            seen.add(item.node);
            let child = this.#children.get(item.node);
            if (child === undefined) {
                child = must(this.#tree.create("View"), "the workspace picture");
                this.#children.set(item.node, child);
                must(this.#tree.insert(child, holder, -1), "the workspace picture");
                this.#write(child, "source", item.node);
            }
            this.#write(child, "x", item.x - area.x);
            this.#write(child, "y", item.y - area.y);
            this.#write(child, "width", item.width);
            this.#write(child, "height", item.height);
            this.#write(child, "visible", item.visible);
        }
        for (const [client, child] of [...this.#children]) {
            if (!seen.has(client)) {
                this.#tree.destroy(child);
                this.#children.delete(client);
            }
        }
    }
}

/** The smallest rectangle that holds every client: the area of a workspace no output shows. */
function boundingBox(items: readonly MirroredClient[]): { x: number; y: number; width: number; height: number } {
    if (items.length === 0) {
        return { x: 0, y: 0, width: 0, height: 0 };
    }
    const left = Math.min(...items.map((item) => item.x));
    const top = Math.min(...items.map((item) => item.y));
    const right = Math.max(...items.map((item) => item.x + item.width));
    const bottom = Math.max(...items.map((item) => item.y + item.height));
    return { x: left, y: top, width: right - left, height: bottom - top };
}
