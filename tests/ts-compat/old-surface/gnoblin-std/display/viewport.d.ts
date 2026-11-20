import type { ViewportInit } from "gnoblin-std/display/types";
import { Camera2D } from "gnoblin-std/display/camera2d";
import type { Display } from "gnoblin-std/display/display";
import { Camera3D } from "gnoblin-std/scene/camera";
/** The state of one viewport, read by the manager. */
export interface ViewportState {
    /** The workspace last assigned or last seen shown. */
    content: object | null;
    /** Camera applied to the content, or null until one is requested. */
    camera: Camera2D | Camera3D | null;
    /** Background colour retained by the viewport. */
    clearColor: string;
    /** Display that presents this viewport, or null while unassigned. */
    display: Display | null;
}
/** The state of a viewport, for the manager. */
export declare function viewportState(viewport: Viewport): ViewportState;
/**
 * A workspace shown through a camera. Assigning `content` on the viewport of a display shows that workspace there;
 * reading it gives the workspace shown now. A workspace can be shown on one display at a time. `Camera3D` values are
 * accepted and retained, but the shell reports that it draws flat workspaces only.
 */
export declare class Viewport {
    /**
     * @throws TypeError when `content` is not a workspace or `camera` is not a camera
     */
    constructor(options?: ViewportInit);
    /** The workspace this viewport shows, or null if it has none. */
    get content(): object | null;
    /** Shows `value` on the assigned display and stores it as the viewport content. */
    set content(value: object);
    /** The camera. A viewport made without one has a `Camera2D` with no zoom, pan or roll. */
    get camera(): Camera2D | Camera3D;
    /** Uses `value` to view this viewport's content. */
    set camera(value: Camera2D | Camera3D);
    /** The colour behind the content: kept, but nothing draws it yet. */
    get clearColor(): string;
    /** Sets the background colour retained by the viewport. */
    set clearColor(value: string);
    /** Returns the fixed name `Viewport`. */
    toString(): string;
}
