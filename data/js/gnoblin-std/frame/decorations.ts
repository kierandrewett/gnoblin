// `Decorations`: creates and fills a `WindowFrame` for each window in the client collection.
// It updates frames when a window's decoration changes and removes them when windows leave.

import type { FrameClient, FrameHooks } from "gnoblin-std/frame/client";
import { WindowFrame } from "gnoblin-std/frame/frame";
import type { FrameKind } from "gnoblin-std/frame/frame";
import { NO_INSET } from "gnoblin-core/inset";
import type { FrameInset } from "gnoblin-core/inset";
import { defaultTitlebarStyle, titlebarFill } from "gnoblin-std/titlebar";
import type { TitlebarStyle } from "gnoblin-std/titlebar";

/** A client the decorations can ask who draws its frame. A `Client` of the host module is one. */
export interface DecoratedClient extends FrameClient {
    /**
     * Who draws the window's frame, as the compositor negotiated it (`Window.decoration`): `server` when the shell
     * does, `client` when the client draws its own title bar, `none` for a window nobody frames.
     */
    readonly decoration: string;
}

/**
 * Fills a frame. It runs once for each window when its frame is made, and again when the window's `decoration` changes,
 * with the window and its frame: read the outlets you want and put nodes in them. It returns `false` to give this
 * window no frame at all. A client-drawn frame (`frame.server` false) has only the underlay and overlay.
 */
export type FrameFill = (client: DecoratedClient, frame: WindowFrame) => false | void;

/** Applies the first matching rule to a window; `frame: false` leaves it without a frame. */
export interface FrameRule {
    /** The application id, or an expression it must match. */
    appId?: string | RegExp;
    /** The title the window has when its frame is made, or an expression it must match. */
    title?: string | RegExp;
    /** Fills the matching window, or returns `false` to leave it unframed. */
    frame: FrameFill | false;
}

/** Host operations `Decorations` uses to locate a window's node and run its buttons. */
export interface FrameHost {
    /** Returns the compositor node id for this window. */
    nodeOf(client: DecoratedClient): number;
    /** Shell operations used by each frame. */
    hooks: FrameHooks;
}

let frameHost: FrameHost | null = null;

/** Connects the decorations to the shell host. The shell calls it once per generation. */
export function attachFrameHost(host: FrameHost): void {
    frameHost = host;
}

/** Options of `Decorations`. Every one is optional; the shell's own supply `nodeOf` and `hooks`. */
export interface DecorationsOptions {
    /** Fills the frame of each window that no rule picked. Left out, the stock titlebar (`style`). */
    frame?: FrameFill;
    /** Fills the frames of the windows a rule matches, ahead of `frame`. */
    rules?: readonly FrameRule[];
    /** The look of the stock titlebar. Read live, so a change reaches the bars that are there. */
    style?: TitlebarStyle;
    /** Puts resize handles this many pixels outside the frame of every server window. Left out, there are none. */
    resizeHandles?: number;
    /** Returns the compositor node for a window. */
    nodeOf?: (client: DecoratedClient) => number;
    /** Shell hooks to use instead of the attached host's hooks. */
    hooks?: FrameHooks;
}

function matches(wanted: string | RegExp | undefined, actual: string): boolean {
    return wanted === undefined || (typeof wanted === "string" ? wanted === actual : wanted.test(actual));
}

/**
 * Gives every window a `WindowFrame` and fills it. A window the compositor told to be server-decorated gets a `server`
 * frame with edges and an inset; any other window gets a `layers` frame, which has the underlay and the overlay and no
 * inset: a client that draws its own title bar (a GTK or Qt window, foot) never gets a second bar over it.
 * The compositor tells a client that asks for server-side that the server draws only while this shell has declared
 * decorations (`Seat.decorations_available`), so a window requesting a frame the shell does not draw is told to
 * draw its own.
 *
 * `insetOf(client)` is how much room the client's frame takes round its content: 0 on every side for a client with no
 * frame. The layout driver leaves that much free around each window it places, keeping frame and content apart.
 */
export class Decorations {
    /** The stock titlebar style used by the default fill. */
    readonly style: TitlebarStyle;
    /** Shell hooks used by the frames. */
    readonly hooks: FrameHooks;
    readonly #nodeOf: (client: DecoratedClient) => number;
    readonly #fill: FrameFill;
    readonly #rules: readonly FrameRule[];
    readonly #resizeHandles: number;
    readonly #frames = new Map<DecoratedClient, WindowFrame>();

    constructor(options: DecorationsOptions = {}) {
        this.style = options.style ?? { ...defaultTitlebarStyle };
        this.hooks = options.hooks ?? frameHost?.hooks ?? {};
        this.#nodeOf = options.nodeOf ?? ((client) => this.#host().nodeOf(client));
        this.#fill = options.frame ?? titlebarFill(this.style);
        this.#rules = options.rules ?? [];
        this.#resizeHandles = options.resizeHandles ?? 0;
    }

    #host(): FrameHost {
        if (frameHost === null) {
            throw new Error("Decorations needs the shell host to find a window's node: pass `nodeOf`");
        }
        return frameHost;
    }

    /** The frame of `client`, if it has one. */
    frameOf(client: DecoratedClient): WindowFrame | undefined {
        return this.#frames.get(client);
    }

    /** The room `client`'s frame takes round its content: nothing on any side with no frame, or while fullscreen. */
    insetOf(client: DecoratedClient): FrameInset {
        return this.#frames.get(client)?.inset ?? NO_INSET;
    }

    /** The room `client`'s frame keeps for itself even while fullscreen hides it. */
    reservedOf(client: DecoratedClient): FrameInset {
        return this.#frames.get(client)?.reserved ?? NO_INSET;
    }

    /**
     * Takes the frame of `client` away and makes it again, calling the fill again. A rule that reads the title or a
     * fill that reads another property of the window is decided when the frame is made, so a script that wants it
     * decided again calls this.
     */
    reframe(client: DecoratedClient): void {
        this.#drop(client);
        this.#build(client);
    }

    /** Ensures the window has a frame when it enters the collection. */
    onAdded(client: DecoratedClient): void {
        this.#follow(client);
    }

    /** The compositor changed who draws the client's frame: the frame is made again for it. */
    onDecorationChanged(client: DecoratedClient): void {
        this.#follow(client);
    }

    /** Removes the frame and its outlets when the window leaves the collection. */
    onRemoved(client: DecoratedClient): void {
        this.#drop(client);
    }

    #drop(client: DecoratedClient): void {
        this.#frames.get(client)?.destroy();
        this.#frames.delete(client);
    }

    /** Makes the client's frame when it has none, and again when the kind of frame it takes has changed. */
    #follow(client: DecoratedClient): void {
        const have = this.#frames.get(client);
        if (have?.kind === this.#kindOf(client)) {
            return;
        }
        this.#drop(client);
        this.#build(client);
    }

    #kindOf(client: DecoratedClient): FrameKind {
        return client.decoration === "server" ? "server" : "layers";
    }

    #fillFor(client: DecoratedClient): FrameFill | false {
        const appId = client.appId ?? "";
        const rule = this.#rules.find(
            (candidate) => matches(candidate.appId, appId) && matches(candidate.title, client.title),
        );
        return rule === undefined ? this.#fill : rule.frame;
    }

    /** Fills a new frame. A thrown error or `false` result leaves no frame behind. */
    #build(client: DecoratedClient): void {
        const fill = this.#fillFor(client);
        if (fill === false) {
            return;
        }
        const frame = new WindowFrame(client, this.#nodeOf(client), { kind: this.#kindOf(client), hooks: this.hooks });
        try {
            if (fill(client, frame) === false) {
                frame.destroy();
                return;
            }
            if (frame.server && this.#resizeHandles > 0) {
                frame.resizeHandles(this.#resizeHandles);
            }
        } catch (error) {
            frame.destroy();
            throw error;
        }
        this.#frames.set(client, frame);
    }
}
