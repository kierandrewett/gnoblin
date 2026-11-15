// The session wrappers a script holds: `XRSession` (the program's session, its views and its listeners) and
// `XRExtension`, which `gnoblin.extensions.xr` is while a session exists.

import { XRCamera3D, XRTrackedPose } from "gnoblin/xr/camera";
import { MAX_CHORD_SETTLE_MS, XRController, XRList } from "gnoblin/xr/controller";
import { SpatialLayout } from "gnoblin/xr/layout";
import type { SpatialLayoutOptions } from "gnoblin/xr/layout";
import { XRRoom } from "gnoblin/xr/room";
import type { NodeNumber, XRClient, XRWorld } from "gnoblin/xr/types";
import { XRViewport } from "gnoblin/xr/viewport";
import {
    ViewMoveBinding,
    ViewResizeBinding,
    ViewRotateBinding,
    ViewScaleBinding,
    XRControllerDragBinding,
} from "gnoblin/xr_bindings";

/** The hooks of a listener of the extension. Subclass it and override what you need. */
export class XRListener {
    /** A session began: the program made the `xr` seat. */
    onSessionStarted(_session: XRSession): void {}
    /** The session ended: the seat is gone. `session.alive` is already false. */
    onSessionEnded(_session: XRSession): void {}
}

/** One headset session, with its head pose, controllers and optional viewport. */
export class XRSession {
    readonly #world: XRWorld;
    readonly #seat: NodeNumber;
    #alive = true;
    #viewport: XRViewport | null = null;
    #onViewport: (viewport: XRViewport) => void;
    readonly controllers: XRList<XRController>;
    readonly head: XRTrackedPose;
    #chordSettleMs = 50;

    /**
     * How long, in milliseconds, a chord must have been down before a drag starts, and so before the drag claims its
     * buttons. Default 50, at most `MAX_CHORD_SETTLE_MS` (60): a larger value reads back as 60.
     *
     * The XR program holds button presses back from the window under the ray for 150 ms while it waits for a claim.
     * The shell must claim within this settle interval for a drag to own those inputs before a click reaches the window.
     * Longer waits would outlast the hold, so `chordSettleMs` is capped at 60 ms.
     */
    get chordSettleMs(): number {
        return this.#chordSettleMs;
    }

    set chordSettleMs(ms: number) {
        if (typeof ms !== "number" || Number.isNaN(ms)) {
            throw new TypeError(`session.chordSettleMs must be a number of milliseconds, got ${String(ms)}`);
        }
        this.#chordSettleMs = Math.min(Math.max(ms, 0), MAX_CHORD_SETTLE_MS);
    }

    /** @internal Made by the runtime when the seat appears. */
    constructor(world: XRWorld, seat: NodeNumber, onViewport: (viewport: XRViewport) => void) {
        this.#world = world;
        this.#seat = seat;
        this.#onViewport = onViewport;
        this.controllers = new XRList<XRController>(world.report);
        this.head = new XRTrackedPose(world, () => this.#headCamera());
    }

    #headCamera(): NodeNumber | undefined {
        return this.#world.nodes("Camera").find((node) => this.#world.scene.get(node, "name") === "head");
    }

    get id(): string {
        return String(this.#seat);
    }

    /** False once the program ended the session (the seat is gone). A dead session rejects writes. */
    get alive(): boolean {
        return this.#alive;
    }

    /** Whether the program is connected: the same as `alive`, because the seat is the session. */
    get connected(): boolean {
        return this.#alive;
    }

    /** `tracked` while a controller is tracked, else `untracked`. The tree does not say whether the head is. */
    get tracking(): "tracked" | "untracked" {
        for (const controller of this.controllers) {
            if (controller.tracked) return "tracked";
        }
        return "untracked";
    }

    /** The seat the program made, as an input context: gaze and the pointer ray arrive here as pointer input. */
    get input(): object | null {
        return this.#alive ? this.#world.inputContext(this.#seat) : null;
    }

    /** The number of layers the runtime allows in a frame, as the program reports it on the head camera; 0 if not yet. */
    get layerBudget(): number {
        const camera = this.#headCamera();
        return camera === undefined ? 0 : ((this.#world.scene.get(camera, "layer_budget") as number | undefined) ?? 0);
    }

    get viewport(): XRViewport | null {
        return this.#viewport;
    }

    set viewport(value: XRViewport) {
        if (!this.#alive) {
            throw new Error("XRSession.viewport: the session has ended");
        }
        if (!(value instanceof XRViewport)) {
            throw new TypeError("XRSession.viewport: expected an XRViewport");
        }
        this.#onViewport(value);
        this.#viewport = value;
    }

    /** @internal */
    end(): void {
        this.#alive = false;
    }

    /** @internal Brings `controllers` up to date with the controller nodes in the tree. */
    syncControllers(): void {
        const nodes = this.#world.nodes("Controller");
        for (const controller of [...this.controllers]) {
            if (!nodes.includes(Number(controller.id))) this.controllers.remove(controller);
        }
        for (const node of nodes) {
            if (this.controllers.get(String(node)) === undefined) {
                this.controllers.add(new XRController(this.#world, node));
            }
        }
        for (const controller of this.controllers) controller.update();
    }
}

/** `gnoblin.extensions.xr`: present while the program has a session. */
export class XRExtension {
    readonly available = true;
    readonly sessions: XRList<XRSession>;
    readonly Camera3D = XRCamera3D;
    readonly Viewport = XRViewport;
    readonly SpatialLayout = SpatialLayout;
    readonly ViewMoveBinding = ViewMoveBinding;
    readonly ViewResizeBinding = ViewResizeBinding;
    readonly ViewRotateBinding = ViewRotateBinding;
    readonly ViewScaleBinding = ViewScaleBinding;
    readonly ControllerDragBinding = XRControllerDragBinding;
    readonly Room = XRRoom;
    /** The room the stock shell built, or the one a script assigned as the viewport's content. */
    room: XRRoom | null = null;

    /** @internal */
    constructor(sessions: XRList<XRSession>) {
        this.sessions = sessions;
    }

    /** The live session, or null. */
    get session(): XRSession | null {
        for (const session of this.sessions) {
            if (session.alive) return session;
        }
        return null;
    }
}

/** What `new XRExtension` takes of the world and of the hooks the host gives it. */
export interface XROptions {
    /** Whether the stock shell builds a room when a session starts. Default true. */
    room?: boolean;
    /** The ring and the bindings of the stock room. */
    layout?: SpatialLayoutOptions;
    accept?: (client: XRClient) => boolean;
}
