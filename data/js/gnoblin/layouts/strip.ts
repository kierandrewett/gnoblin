// Two layouts whose nodes are not tiled, from data/lua/gnoblin/shell/strip.lua.
// StripLayout: a row of columns, one node per column, scrolled so the focused column stays in view.
// Carousel3dLayout: nodes on a ring seen in perspective, the focused one at the front.
//
// Neither floors its rectangles and neither marks its nodes tiled, as the Lua classes did not.

import { Layout } from "gnoblin/layout";
import type { LayoutContext, LayoutNode, Placement, Rect } from "gnoblin/layout";

const TAU = 2 * Math.PI;
const MIN_COLUMN = 0.1;
const MAX_COLUMN = 1;

/** A column's width as a fraction of the strip's width, held to 0.1 to 1. Anything but a number is an error. */
export function clampColumn(value: unknown): number {
    if (typeof value !== "number" || Number.isNaN(value)) {
        throw new TypeError(`column must be a number, got ${String(value)}`);
    }
    return Math.min(Math.max(value, MIN_COLUMN), MAX_COLUMN);
}

/** What the focus helpers need of a node: whether it has focus and how to give it focus. */
export interface FocusableNode {
    readonly focused: boolean;
    focus(): void;
}

/** What they need of a workspace: the nodes it holds. */
export interface NodeSource {
    readonly clients: Iterable<LayoutNode>;
}

function focusable(node: LayoutNode): FocusableNode {
    return node as FocusableNode;
}

/** Options of `StripLayout`. */
export interface StripOptions {
    /** A column's width as a fraction of the strip's, held to 0.1 to 1. Default 0.5. */
    column?: number;
}

/**
 * Columns sit edge to edge with no gap or margin. The view follows focus unless `scrollTo` pinned it; a pin ends
 * when the focused node changes.
 *
 * The layout keeps its own column order, because a new node opens right after the focused column rather than at the
 * end. One instance must serve one workspace.
 */
export class StripLayout extends Layout {
    column: number;
    /** The view offset in pixels `scrollTo` pinned, or `null` while the view follows the focused column. */
    pin: number | null = null;
    #order: LayoutNode[] = [];
    #view = 0;
    /** The focus the previous compute saw. */
    #focus: LayoutNode | null = null;

    constructor(options: StripOptions = {}) {
        super();
        this.column = options.column ?? 0.5;
        clampColumn(this.column);
    }

    compute(context: LayoutContext): Placement[] {
        const { x, y, width: viewWidth, height } = context.area;
        const columnWidth = clampColumn(this.column) * viewWidth;
        const focused = context.focused ?? null;
        const live = new Set<LayoutNode>(context.children);
        this.#order = this.#order.filter((node) => live.has(node));
        const known = new Set(this.#order);
        // A node opens after the column that had focus when it arrived, which the compositor has since moved to it.
        let at = this.#focus === null ? -1 : this.#order.indexOf(this.#focus);
        for (const node of context.children) {
            if (known.has(node)) {
                continue;
            }
            if (at >= 0) {
                at += 1;
                this.#order.splice(at, 0, node);
            } else {
                this.#order.push(node);
            }
        }
        if (focused !== this.#focus) {
            this.#focus = focused !== null && live.has(focused) ? focused : null;
            this.pin = null;
        }
        const order = this.#order;
        if (this.pin !== null) {
            this.#view = this.pin;
        } else {
            const index = focused === null ? -1 : order.indexOf(focused);
            if (index >= 0) {
                const left = index * columnWidth;
                if (left < this.#view || left + columnWidth > this.#view + viewWidth) {
                    this.#view = left + columnWidth / 2 - viewWidth / 2; // centre the focused column
                }
            }
        }
        this.#view = Math.min(Math.max(this.#view, 0), Math.max(0, order.length * columnWidth - viewWidth));
        const view = this.#view;
        return order.map((node, i) => ({
            node,
            bounds: { x: x + i * columnWidth - view, y, width: columnWidth, height },
        }));
    }

    /** Scrolls the view to `offset` pixels and keeps it there until the focused node changes. */
    scrollTo(offset: number): void {
        if (typeof offset !== "number" || !Number.isFinite(offset)) {
            throw new TypeError(`scrollTo needs a finite number, got ${String(offset)}`);
        }
        this.pin = offset;
    }

    /** Moves focus one column left; at the first column nothing happens. */
    focusLeft(workspace: NodeSource): void {
        this.#step(workspace, -1);
    }

    /** Moves focus one column right; at the last column nothing happens. */
    focusRight(workspace: NodeSource): void {
        this.#step(workspace, 1);
    }

    #step(workspace: NodeSource, delta: -1 | 1): void {
        const held = new Set(workspace.clients);
        const columns = this.#order.filter((node) => held.has(node));
        const at = columns.findIndex((node) => focusable(node).focused);
        const next = at >= 0 ? columns[at + delta] : undefined;
        if (next !== undefined) {
            focusable(next).focus();
        }
    }

    /** Focus moves to a neighbour of a focused column that went, so the strip keeps a focus. */
    onClientRemoved(client: LayoutNode, context: LayoutContext): void {
        const at = this.#order.indexOf(client);
        if (at < 0 || client !== this.#focus) {
            return;
        }
        const live = new Set<LayoutNode>(context.children);
        const next = [this.#order[at + 1], this.#order[at - 1]].find(
            (node): node is LayoutNode => node !== undefined && live.has(node),
        );
        if (next !== undefined) {
            focusable(next).focus();
        }
    }
}

/** Options of `ring`, which `Carousel3dLayout` lays its cards out with. */
export interface RingOptions {
    /** Vertical field of view in degrees. Default 40. */
    fov?: number;
    /** A card's width as a fraction of the area's. Default 0.36. */
    cardWidth?: number;
    /** A card's height as a fraction of the area's. Default 0.6. */
    cardHeight?: number;
}

/** One card of the ring: the projected rectangle, the depth fade, the angle round the ring and the depth. */
export interface RingSlot extends Rect {
    opacity: number;
    /** Degrees: 0 at the front, 180 at the back, positive to the right of the front card. */
    rotation: number;
    /** Negative in front of the camera, more so for the back of the ring. */
    z: number;
}

/**
 * The pure ring layout of `count` cards with the 1-based `focus` at the front, inside `area`. One slot per card.
 */
export function ring(count: number, focus: number, area: Rect, options: RingOptions = {}): RingSlot[] {
    if (count === 0) {
        return [];
    }
    if (!Number.isInteger(focus) || focus < 1 || focus > count) {
        throw new RangeError(`focus must be an index in 1..${count}, got ${String(focus)}`);
    }
    const cardWidth = area.width * (options.cardWidth ?? 0.36);
    const cardHeight = area.height * (options.cardHeight ?? 0.6);
    const radius = Math.max(cardWidth * 1.25, (count * cardWidth * 1.25) / TAU);
    const focal = Math.max(1, cardHeight / 2 / Math.tan(((options.fov ?? 40) * Math.PI) / 180 / 2));
    const centreZ = -(focal + radius);
    const cx = area.x + area.width / 2;
    const cy = area.y + area.height / 2;
    const slots: RingSlot[] = [];
    for (let i = 1; i <= count; i++) {
        let k = (((i - focus) % count) + count) % count;
        if (k > count / 2) {
            k -= count; // signed offset from the front, in (-count/2, count/2]
        }
        const angle = (k * TAU) / count;
        const sideways = radius * Math.sin(angle);
        const z = centreZ + radius * Math.cos(angle);
        const scale = focal / -z;
        const width = cardWidth * scale;
        const height = cardHeight * scale;
        slots.push({
            x: cx + sideways * scale - width / 2,
            y: cy - height / 2,
            width,
            height,
            opacity: 0.4 + (0.6 * (1 + Math.cos(angle))) / 2,
            rotation: (angle * 180) / Math.PI,
            z,
        });
    }
    return slots;
}

/** Options of `Carousel3dLayout`. It takes the same fields as `RingOptions`. */
export interface Carousel3dOptions extends RingOptions {}

/**
 * Cards on a ring, the focused node at the front; with no focus among the nodes the first is. Each placement is the
 * ring's projected rectangle together with the ring's depth (`z`) and its angle as a rotation about the vertical
 * axis, so a renderer that draws depth can use them. The depth fade of the ring is not applied: a node's opacity
 * belongs to the rules.
 */
export class Carousel3dLayout extends Layout {
    fov: number;
    cardWidth: number;
    cardHeight: number;

    constructor(options: Carousel3dOptions = {}) {
        super();
        this.fov = options.fov ?? 40;
        this.cardWidth = options.cardWidth ?? 0.36;
        this.cardHeight = options.cardHeight ?? 0.6;
    }

    compute(context: LayoutContext): Placement[] {
        const nodes = context.children;
        const focused = context.focused ?? null;
        const index = focused === null ? -1 : nodes.lastIndexOf(focused);
        const slots = ring(nodes.length, index >= 0 ? index + 1 : 1, context.area, {
            fov: this.fov,
            cardWidth: this.cardWidth,
            cardHeight: this.cardHeight,
        });
        return nodes.map((node, i) => {
            const slot = slots[i];
            return {
                node,
                bounds: { x: slot.x, y: slot.y, width: slot.width, height: slot.height },
                z: slot.z,
                rotation: [0, (slot.rotation * Math.PI) / 180, 0] as const,
            };
        });
    }

    /** Moves focus `steps` cards round the ring, wrapping at both ends. */
    rotate(workspace: NodeSource, steps: number): void {
        if (typeof steps !== "number" || !Number.isFinite(steps)) {
            throw new TypeError(`rotate needs a number of steps, got ${String(steps)}`);
        }
        const cards = [...workspace.clients];
        const at = cards.findIndex((node) => focusable(node).focused);
        if (at < 0) {
            return;
        }
        const count = cards.length;
        focusable(cards[(((at + steps) % count) + count) % count]).focus();
    }
}
