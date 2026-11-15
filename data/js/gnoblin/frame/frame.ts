// `WindowFrame`: the surface round one window's content, with outlets a script fills with its own nodes.
// `Decorations` makes one per window and hands it to the script's `frame` callback. The frame makes an outlet's node when
// the script first reads the outlet, places every outlet from the window's size and the frame's inset, and reports the
// inset to the layout driver. design/window-frame.md.

import { WindowActions } from "gnoblin/frame/actions";
import type { FrameClient, FrameContext, FrameHooks } from "gnoblin/frame/client";
import { CornerOutlet } from "gnoblin/frame/corner_outlet";
import type { Corner } from "gnoblin/frame/corner_outlet";
import { EdgeOutlet } from "gnoblin/frame/edge_outlet";
import type { Side } from "gnoblin/frame/edge_outlet";
import { NO_INSET } from "gnoblin/frame/inset";
import type { FrameInset } from "gnoblin/frame/inset";
import { LayerOutlet } from "gnoblin/frame/layer_outlet";
import type { LayerKind } from "gnoblin/frame/layer_outlet";
import type { FrameGeometry, Outlet, OutletOwner } from "gnoblin/frame/outlet";
import { ResizeHandles } from "gnoblin/frame/parts/resize_handles";
import type { ResizeHandlesOptions } from "gnoblin/frame/parts/resize_handles";
import type { Rect } from "gnoblin/types";
import { onUiFrame } from "gnoblin/ui";

/**
 * What a frame is for: `server` for a window the shell frames (it has edges and an inset), `layers` for a window that
 * draws its own frame or has none, which can still have an underlay and an overlay but no edge and no inset.
 */
export type FrameKind = "server" | "layers";

/** The content outlet: where the client's surface sits. It is not filled, and says where it is. */
export interface ContentOutlet {
    /** The content's rectangle in the window's coordinates: its origin is the window node's. */
    readonly rect: Rect;
    /** The room the frame leaves round it. */
    readonly inset: FrameInset;
}

/** The order the outlets are in the window's children, back to front: edges, then corners, then the overlay. */
const STACKING = [
    "left",
    "right",
    "top",
    "bottom",
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
    "overlay",
] as const;
type StackName = (typeof STACKING)[number];

/**
 * The frame of one window. Its outlets are `top`, `bottom`, `left` and `right` (edges that take room), `topLeft`,
 * `topRight`, `bottomLeft` and `bottomRight` (corners that take none), `underlay` and `overlay` (layers that take none),
 * `start`, `center` and `end` (the zones of the top edge), and `content` (not fillable). Reading an outlet of a frame of
 * a window that draws its own frame (`kind` `layers`) throws, except the layers: a second title bar over the client's own
 * is the double title bar.
 *
 * `inset` is what the layout driver leaves round the window's content, and it is all zero while the window is fullscreen;
 * `reserved` is the same without that, the room the window keeps while fullscreen hides the frame.
 */
export class WindowFrame implements FrameContext, OutletOwner {
    readonly client: FrameClient;
    readonly hooks: FrameHooks;
    readonly actions: WindowActions;
    readonly kind: FrameKind;
    readonly #node: number;
    readonly #outlets = new Map<StackName | "underlay", Outlet>();
    #stop: (() => void) | null = null;
    #alive = true;

    /**
     * `node` is the window's node in the tree: the outlets are made under it. `actions` is the window's shared actions;
     * left out, the frame makes them from `hooks`.
     */
    constructor(
        client: FrameClient,
        node: number,
        options: { kind?: FrameKind; hooks?: FrameHooks; actions?: WindowActions } = {},
    ) {
        this.client = client;
        this.#node = node;
        this.kind = options.kind ?? "server";
        this.hooks = options.hooks ?? {};
        this.actions = options.actions ?? new WindowActions(client, this.hooks);
    }

    /** Whether the frame is a server frame: the window has edges and an inset. */
    get server(): boolean {
        return this.kind === "server";
    }

    /** True until `destroy`, and until the window's node is gone. */
    get alive(): boolean {
        return this.#alive && this.client.alive !== false;
    }

    /** The content's size and the inset now. */
    geometry(): FrameGeometry {
        return { width: this.client.width, height: this.client.height ?? 0, inset: this.inset };
    }

    /** The room the frame takes round the content now: nothing while the window is fullscreen or has no server frame. */
    get inset(): FrameInset {
        return this.#insetOf((edge) => edge.thickness);
    }

    /** The room the frame keeps for itself even while fullscreen hides it. */
    get reserved(): FrameInset {
        return this.#insetOf((edge) => edge.reserved);
    }

    #insetOf(thickness: (edge: EdgeOutlet) => number): FrameInset {
        if (this.kind !== "server") {
            return NO_INSET;
        }
        const of = (side: Side): number => {
            const edge = this.#outlets.get(side);
            return edge instanceof EdgeOutlet ? thickness(edge) : 0;
        };
        return { top: of("top"), right: of("right"), bottom: of("bottom"), left: of("left") };
    }

    /** The content outlet: not fillable, it says where the client's surface is and how much room the edges left it. */
    get content(): ContentOutlet {
        const { width, height, inset } = this.geometry();
        return { rect: { x: 0, y: 0, width, height }, inset };
    }

    get top(): EdgeOutlet {
        return this.#edge("top");
    }

    get bottom(): EdgeOutlet {
        return this.#edge("bottom");
    }

    get left(): EdgeOutlet {
        return this.#edge("left");
    }

    get right(): EdgeOutlet {
        return this.#edge("right");
    }

    get topLeft(): CornerOutlet {
        return this.#corner("top-left");
    }

    get topRight(): CornerOutlet {
        return this.#corner("top-right");
    }

    get bottomLeft(): CornerOutlet {
        return this.#corner("bottom-left");
    }

    get bottomRight(): CornerOutlet {
        return this.#corner("bottom-right");
    }

    /** The first zone of the top edge: a titlebar's left. */
    get start(): Outlet {
        return this.top.start;
    }

    /** The middle zone of the top edge: a titlebar's title, which takes the room the other two leave. */
    get center(): Outlet {
        return this.top.center;
    }

    /** The last zone of the top edge: a titlebar's right. */
    get end(): Outlet {
        return this.top.end;
    }

    /** The layer behind the window's content: a backdrop, a glow. It takes no room and no pointer. */
    get underlay(): LayerOutlet {
        return this.#layer("under");
    }

    /** The layer in front of the window: a focus ring, a badge, resize handles. It takes no room and, by default, no pointer. */
    get overlay(): LayerOutlet {
        return this.#layer("over");
    }

    /**
     * Puts the eight resize handles round the frame, `thickness` pixels reaching outside it, and returns them. The stock
     * titlebar has none: a window is resized with `Super+MouseRight+Drag` unless its frame has handles.
     */
    resizeHandles(thickness = 6, options: Omit<ResizeHandlesOptions, "thickness"> = {}): ResizeHandles {
        const handles = new ResizeHandles(this, { thickness, ...options });
        this.overlay.add(handles);
        return handles;
    }

    #edge(side: Side): EdgeOutlet {
        this.#need(`the ${side} outlet`);
        return this.#make(side, () => new EdgeOutlet(this, side));
    }

    #corner(corner: Corner): CornerOutlet {
        this.#need(`the ${corner} corner outlet`);
        return this.#make(corner, () => new CornerOutlet(this, corner));
    }

    #layer(kind: LayerKind): LayerOutlet {
        return this.#make(kind === "under" ? "underlay" : "overlay", () => new LayerOutlet(this, kind));
    }

    #need(what: string): void {
        if (this.kind !== "server") {
            throw new RangeError(
                `${what} belongs to a window the shell frames: this window draws its own frame, so it has the underlay and the overlay only`,
            );
        }
    }

    /**
     * Makes an outlet's node the first time it is read, and keeps the window's children in stacking order. The map holds
     * each name's outlet of the one class its getter asks for, so `found` is a `T`.
     */
    #make<T extends Outlet>(name: StackName | "underlay", build: () => T): T {
        const found = this.#outlets.get(name) as T | undefined;
        if (found !== undefined && found.alive) {
            return found;
        }
        if (!this.#alive) {
            throw new RangeError("the frame was destroyed");
        }
        const made = build();
        this.#outlets.set(name, made);
        made.mount(this.#node);
        // Outlets made earlier that stack above this one are mounted again, so they stay above it. The underlay is
        // painted behind the window, whatever its place among the children.
        if (name !== "underlay") {
            for (const above of STACKING.slice(STACKING.indexOf(name) + 1)) {
                this.#outlets.get(above)?.mount(this.#node);
            }
        }
        this.#stop ??= onUiFrame(() => this.actions.advance());
        return made;
    }

    /** Takes the frame away: its outlets and everything in them are destroyed, and its hook stops. */
    destroy(): void {
        if (!this.#alive) {
            return;
        }
        this.#alive = false;
        this.#stop?.();
        this.#stop = null;
        for (const outlet of this.#outlets.values()) {
            outlet.destroy();
        }
        this.#outlets.clear();
    }
}
