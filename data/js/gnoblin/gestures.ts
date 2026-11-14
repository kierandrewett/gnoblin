// The stock touchpad gesture: a three-finger swipe slides to the next or previous workspace and follows the
// fingers while they move. Fingers moving right bring the previous workspace in from the left, fingers moving
// left the next one from the right. Lifting after `commit` of `travel` finishes the slide from where the fingers
// left it; lifting sooner, or a cancel, slides back. A swipe that is more vertical than horizontal, or that has
// no workspace to reveal, moves nothing.
//
// A TypeScript workspace is not a node, so the slide moves its windows: each window's `x` follows the fingers
// and goes back to where it rests when the slide is over. The gesture reaches the shell through `SlideHost`
// and never imports the host module. Read `gnoblin/bindings` (`GestureBinding`) first. This module does not
// register itself: a config adds `new WorkspaceSlideBinding()` to the bindings it wants.

import { GestureBinding } from "gnoblin/bindings";
import type { BindingOptions, DragEvent } from "gnoblin/bindings";

// ---- What the slide needs of the shell ----

/** What the slide needs of a window. */
export interface SlideClient {
    x: number;
    visible: boolean;
    readonly alive: boolean;
    animate(values: { x: number }, options: { duration: number; easing: "out-cubic" }): void;
}

/** What it needs of a workspace. */
export interface SlideWorkspace {
    readonly active: boolean;
    /** The position among the workspaces. */
    readonly index: number;
    readonly clients: Iterable<SlideClient>;
}

/** What it needs of the shell: the `gnoblin` global. */
export interface SlideHost {
    readonly workspaces: {
        /** The area windows are laid out in; null while there is no screen. */
        readonly area: { readonly width: number } | null;
        at(index: number): SlideWorkspace | undefined;
        show(workspace: SlideWorkspace, options: { duration: number; progress: number }): void;
        [Symbol.iterator](): Iterator<SlideWorkspace>;
    };
    after(seconds: number, run: () => void): void;
    /** The seconds an animation takes: 0 while the seat asks for reduced motion. Absent, the seconds it was given. */
    motion?(seconds: number): number;
}

/**
 * The options of `WorkspaceSlideBinding`. Every field is optional; `id` is "workspace-slide" and `trigger` "Swipe3".
 */
export interface WorkspaceSlideOptions extends Partial<BindingOptions> {
    /** How far fingers move for a slide of one whole workspace, in touchpad units. Default 400. */
    travel?: number;
    /** The share of a slide at which lifting the fingers finishes it, from 0 to 1. Default 0.25. */
    commit?: number;
    /** Seconds a whole slide takes, which a slide finished part way scales by what is left. Default 0.25. */
    seconds?: number;
    /** The shell to slide in. Default the `gnoblin` global. */
    host?: SlideHost;
}

/** The seconds the slide back takes, whatever `seconds` is. It is a fixed 0.2 against the forward slide's 0.25. */
const SETTLE_BACK_SECONDS = 0.2;

/** What one swipe knows between the fingers' first touch and their lift. */
interface SlideState {
    readonly from: SlideWorkspace;
    /** Where each window the slide has moved rests. */
    readonly rest: Map<SlideClient, number>;
    sign: 0 | 1 | -1;
    to: SlideWorkspace | null;
    progress: number;
    /** Something else switched workspace under the fingers: the slide has nothing more to do. */
    over: boolean;
}

// ---- The binding ----

/**
 * A touchpad swipe that slides the workspace in the direction of the fingers.
 *
 * @example
 * bindings.add(new WorkspaceSlideBinding({ travel: 500, commit: 0.3 }));
 */
export class WorkspaceSlideBinding extends GestureBinding {
    /** Finger travel for a slide of one whole workspace, in touchpad units. */
    travel: number;
    /** The share of a slide at which lifting finishes it. */
    commit: number;
    /** Seconds a whole slide takes. */
    seconds: number;
    readonly #host: SlideHost | undefined;

    constructor(options: WorkspaceSlideOptions = {}) {
        super({ ...options, id: options.id ?? "workspace-slide", trigger: options.trigger ?? "Swipe3" });
        this.travel = options.travel ?? 400;
        this.commit = options.commit ?? 0.25;
        this.seconds = options.seconds ?? 0.25;
        if (!(this.travel > 0) || !Number.isFinite(this.travel)) {
            throw new RangeError("WorkspaceSlideBinding travel must be a finite number above 0");
        }
        if (!(this.commit >= 0 && this.commit <= 1)) {
            throw new RangeError("WorkspaceSlideBinding commit must be a number from 0 to 1");
        }
        if (!(this.seconds >= 0) || !Number.isFinite(this.seconds)) {
            throw new RangeError("WorkspaceSlideBinding seconds must be a finite number of at least 0");
        }
        this.#host = options.host;
    }

    /** The shell the binding slides in: the one it was given, else the `gnoblin` global (set when the shell runs). */
    #shell(): SlideHost {
        return this.#host ?? ((globalThis as { gnoblin?: unknown }).gnoblin as SlideHost);
    }

    /** The workspace the primary display shows, which a slide starts from. */
    #active(): SlideWorkspace | undefined {
        return [...this.#shell().workspaces].find((workspace) => workspace.active);
    }

    /** Refuses the gesture when there is no workspace to leave or no screen; else records where each window rests. */
    onStart(_event: DragEvent): SlideState | false {
        const from = this.#active();
        if (from === undefined || this.#shell().workspaces.area === null) {
            return false;
        }
        const rest = new Map<SlideClient, number>();
        for (const client of from.clients) {
            rest.set(client, client.x);
        }
        return { from, rest, sign: 0, to: null, progress: 0, over: false };
    }

    /** Follows the fingers: picks the target workspace on the first horizontal move and re-places every window. */
    onUpdate(event: DragEvent, state: object): void {
        const slide = state as SlideState;
        if (slide.over) {
            return;
        }
        const active = this.#active();
        if (active !== slide.from) {
            if (active !== slide.to) {
                this.#reset(slide);
            }
            slide.over = true;
            return;
        }
        const dx = event.dx ?? 0;
        const dy = event.dy ?? 0;
        if (dx === 0) {
            return; // no direction yet
        }
        const sign = dx > 0 ? 1 : -1;
        if (slide.to === null && Math.abs(dy) > Math.abs(dx)) {
            return; // vertical so far: not a slide
        }
        if (sign !== slide.sign) {
            this.#reset(slide);
            const index = slide.from.index - sign;
            const target = index >= 0 ? this.#shell().workspaces.at(index) : undefined;
            if (target !== undefined) {
                slide.sign = sign;
                slide.to = target;
                for (const client of target.clients) {
                    slide.rest.set(client, client.x);
                    client.visible = true;
                }
            }
        }
        if (slide.to !== null) {
            this.#place(slide, Math.min(Math.abs(dx) / this.travel, 1));
        }
    }

    /**
     * Finishes the slide through `workspaces.show` when it passed `commit` and went mostly sideways. Otherwise it
     * settles back.
     */
    onEnd(event: DragEvent, state: object): void {
        const slide = state as SlideState;
        const to = slide.to;
        if (slide.over || to === null || this.#active() !== slide.from) {
            return;
        }
        const committed = slide.progress >= this.commit && Math.abs(event.dx ?? 0) >= Math.abs(event.dy ?? 0);
        if (!committed) {
            this.#settleBack(slide, to);
            return;
        }
        // Back to rest at once, so the shell's slide starts from the positions it laid out and carries on from
        // `progress` for the rest of its time.
        for (const [client, rest] of slide.rest) {
            if (client.alive) {
                client.x = rest;
            }
        }
        this.#shell().workspaces.show(to, {
            duration: this.seconds * (1 - slide.progress),
            progress: slide.progress,
        });
    }

    /** A cancelled gesture slides back like a short one. */
    onCancel(_event: DragEvent, state: object): void {
        const slide = state as SlideState;
        if (slide.over || slide.to === null || this.#active() !== slide.from) {
            return;
        }
        this.#settleBack(slide, slide.to);
    }

    /** Puts the windows where a slide of `progress` (0 to 1) in the direction of `slide.sign` has them. */
    #place(slide: SlideState, progress: number): void {
        const width = this.#shell().workspaces.area?.width ?? 0;
        slide.progress = progress;
        for (const client of slide.from.clients) {
            client.x = (slide.rest.get(client) ?? client.x) + slide.sign * progress * width;
        }
        for (const client of slide.to?.clients ?? []) {
            client.x = (slide.rest.get(client) ?? client.x) + slide.sign * (progress - 1) * width;
        }
    }

    /** Hides the workspace a slide had brought in and puts every window it moved back. */
    #reset(slide: SlideState): void {
        if (slide.to === null) {
            return;
        }
        for (const client of slide.to.clients) {
            if (client.alive) {
                client.visible = false;
            }
        }
        for (const [client, rest] of slide.rest) {
            if (client.alive) {
                client.x = rest;
            }
        }
        slide.to = null;
        slide.sign = 0;
    }

    /**
     * Animates the windows back to rest. The incoming workspace's windows are hidden again when the animation
     * ends, unless the shell showed that workspace meanwhile.
     */
    #settleBack(slide: SlideState, to: SlideWorkspace): void {
        const width = this.#shell().workspaces.area?.width ?? 0;
        const seconds = this.#shell().motion?.(SETTLE_BACK_SECONDS) ?? SETTLE_BACK_SECONDS;
        const options = { duration: seconds, easing: "out-cubic" } as const;
        const incoming = [...to.clients];
        for (const client of slide.from.clients) {
            client.animate({ x: slide.rest.get(client) ?? client.x }, options);
        }
        for (const client of incoming) {
            client.animate({ x: (slide.rest.get(client) ?? client.x) - slide.sign * width }, options);
        }
        this.#shell().after(seconds, () => {
            if (this.#active() === to) {
                return; // the workspace came in for good meanwhile
            }
            for (const client of incoming) {
                if (client.alive) {
                    client.visible = false;
                    client.x = slide.rest.get(client) ?? client.x;
                }
            }
        });
    }
}
