// `Viewport`, a workspace seen through a camera, with the checked state the manager reads through `viewportState`.

import type { ViewportInit } from "gnoblin-std/display/types";
import { shell } from "gnoblin-std/display/state";
import { Camera2D } from "gnoblin-std/display/camera2d";
import type { Display } from "gnoblin-std/display/display";
import { Camera3D } from "gnoblin-std/scene/camera";

// ---- Viewport -----------------------------------------------------------------------------------------------------

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

const viewports = new WeakMap<Viewport, ViewportState>();

/** The state of a viewport, for the manager. */
export function viewportState(viewport: Viewport): ViewportState {
    const state = viewports.get(viewport);
    if (state === undefined) {
        throw new TypeError("not a Viewport");
    }
    return state;
}

function isCamera(value: unknown): value is Camera2D | Camera3D {
    return value instanceof Camera2D || value instanceof Camera3D;
}

/**
 * A workspace shown through a camera. Assigning `content` on the viewport of a display shows that workspace there;
 * reading it gives the workspace shown now. A workspace can be shown on one display at a time. `Camera3D` values are
 * accepted and retained, but the shell reports that it draws flat workspaces only.
 */
export class Viewport {
    /**
     * @throws TypeError when `content` is not a workspace or `camera` is not a camera
     */
    constructor(options: ViewportInit = {}) {
        const { content, camera, clearColor, ...unknown } = options;
        const extra = Object.keys(unknown);
        if (extra.length > 0) {
            throw new TypeError(`Viewport.${extra[0]}: not a property of a viewport`);
        }
        viewports.set(this, { content: null, camera: null, clearColor: "#000000", display: null });
        if (content !== undefined) this.content = content;
        if (camera !== undefined) this.camera = camera;
        if (clearColor !== undefined) this.clearColor = clearColor;
    }

    /** The workspace this viewport shows, or null if it has none. */
    get content(): object | null {
        const state = viewportState(this);
        return state.display === null ? state.content : shell().shown(state.display);
    }
    /** Shows `value` on the assigned display and stores it as the viewport content. */
    set content(value: object) {
        if (!shell().isWorkspace(value)) {
            throw new TypeError("Viewport.content: must be a Workspace");
        }
        const state = viewportState(this);
        if (state.display !== null) {
            shell().show(state.display, value);
        }
        state.content = value;
    }

    /** The camera. A viewport made without one has a `Camera2D` with no zoom, pan or roll. */
    get camera(): Camera2D | Camera3D {
        const state = viewportState(this);
        if (state.camera === null) {
            state.camera = new Camera2D();
        }
        return state.camera;
    }
    /** Uses `value` to view this viewport's content. */
    set camera(value: Camera2D | Camera3D) {
        if (!isCamera(value)) {
            throw new TypeError("Viewport.camera: must be a Camera2D or a Camera3D");
        }
        viewportState(this).camera = value;
    }

    /** The colour behind the content: kept, but nothing draws it yet. */
    get clearColor(): string {
        return viewportState(this).clearColor;
    }
    /** Sets the background colour retained by the viewport. */
    set clearColor(value: string) {
        if (typeof value !== "string" || value === "") {
            throw new TypeError("Viewport.clearColor: must be a colour such as #101820");
        }
        viewportState(this).clearColor = value;
    }

    /** Returns the fixed name `Viewport`. */
    toString(): string {
        return "Viewport";
    }
}
