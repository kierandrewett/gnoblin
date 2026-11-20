import { XRCamera3D, XRTrackedPose } from "gnoblin-std/extensions/xr/camera";
import { XRController, XRList } from "gnoblin-std/extensions/xr/controller";
import { SpatialLayout } from "gnoblin-std/extensions/xr/layout";
import type { SpatialLayoutOptions } from "gnoblin-std/extensions/xr/layout";
import { XRRoom } from "gnoblin-std/extensions/xr/room";
import type { NodeNumber, XRClient, XRWorld } from "gnoblin-std/extensions/xr/types";
import { XRViewport } from "gnoblin-std/extensions/xr/viewport";
import { ViewMoveBinding, ViewResizeBinding, ViewRotateBinding, ViewScaleBinding, XRControllerDragBinding } from "gnoblin-std/extensions/xr/bindings";
/** The hooks of a listener of the extension. Subclass it and override what you need. */
export declare class XRListener {
    /** A session began: the program made the `xr` seat. */
    onSessionStarted(_session: XRSession): void;
    /** The session ended: the seat is gone. `session.alive` is already false. */
    onSessionEnded(_session: XRSession): void;
}
/** One headset session, with its head pose, controllers and optional viewport. */
export declare class XRSession {
    #private;
    /** Controllers currently in the session, updated each frame. */
    readonly controllers: XRList<XRController>;
    /** The current head pose, or null until the head camera exists. */
    readonly head: XRTrackedPose;
    /**
     * How long, in milliseconds, a chord must have been down before a drag starts, and so before the drag claims its
     * buttons. Default 50, at most `MAX_CHORD_SETTLE_MS` (60): a larger value reads back as 60.
     *
     * The program holds button presses for 150 ms while it waits for a claim. The shell must claim within this settle
     * interval for a drag to own those inputs before a click reaches the window. Longer waits would outlast the hold,
     * so `chordSettleMs` is capped at 60 ms.
     */
    get chordSettleMs(): number;
    /** Sets the chord settle interval in milliseconds, clamped to 0-60. */
    set chordSettleMs(ms: number);
    /** @internal Made by the runtime when the seat appears. */
    constructor(world: XRWorld, seat: NodeNumber, onViewport: (viewport: XRViewport) => void);
    /** The XR seat node number as a string. */
    get id(): string;
    /** False once the program ended the session (the seat is gone). A dead session rejects writes. */
    get alive(): boolean;
    /** Whether the program is connected: the same as `alive`, because the seat is the session. */
    get connected(): boolean;
    /** `tracked` while a controller is tracked, else `untracked`. The tree does not say whether the head is. */
    get tracking(): "tracked" | "untracked";
    /** The seat the program made, as an input context: gaze and the pointer ray arrive here as pointer input. */
    get input(): object | null;
    /** Layer limit for each frame, as reported on the head camera; 0 until reported. */
    get layerBudget(): number;
    /** The viewport currently assigned to this session, or null. */
    get viewport(): XRViewport | null;
    /** Assigns the room and camera rendered by the headset. */
    set viewport(value: XRViewport);
    /** @internal */
    end(): void;
    /** @internal Brings `controllers` up to date with the controller nodes in the tree. */
    syncControllers(): void;
}
/** `gnoblin.extensions.xr`: present while the program has a session. */
export declare class XRExtension {
    /** True while this extension object is available. */
    readonly available = true;
    /** Sessions currently known to the extension. */
    readonly sessions: XRList<XRSession>;
    /** Camera class for creating a stereo headset camera. */
    readonly Camera3D: typeof XRCamera3D;
    /** Viewport class for rendering a room in the headset. */
    readonly Viewport: typeof XRViewport;
    /** Layout class for arranging room panels. */
    readonly SpatialLayout: typeof SpatialLayout;
    /** Default binding class for moving room panels. */
    readonly ViewMoveBinding: typeof ViewMoveBinding;
    /** Default binding class for resizing room panels. */
    readonly ViewResizeBinding: typeof ViewResizeBinding;
    /** Default binding class for rotating room panels. */
    readonly ViewRotateBinding: typeof ViewRotateBinding;
    /** Default binding class for scaling room panels. */
    readonly ViewScaleBinding: typeof ViewScaleBinding;
    /** Base binding class for controller-driven panel drags. */
    readonly ControllerDragBinding: typeof XRControllerDragBinding;
    /** Room class for showing clients as headset panels. */
    readonly Room: typeof XRRoom;
    /** The room the stock shell built, or the one a script assigned as the viewport's content. */
    room: XRRoom | null;
    /** @internal */
    constructor(sessions: XRList<XRSession>);
    /** The live session, or null. */
    get session(): XRSession | null;
}
/** What `new XRExtension` takes of the world and of the hooks the host gives it. */
export interface XROptions {
    /** Whether the stock shell builds a room when a session starts. Default true. */
    room?: boolean;
    /** The ring and the bindings of the stock room. */
    layout?: SpatialLayoutOptions;
    /** Selects clients to show in the default room; secure clients are excluded. */
    accept?: (client: XRClient) => boolean;
}
