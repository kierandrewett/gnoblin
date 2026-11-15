// `Viewport`, a workspace seen through a camera, with the checked state the manager reads through `viewportState`.

import type { ViewportInit } from "gnoblin/display/types";
import { shell } from "gnoblin/display/state";
import { Camera2D } from "gnoblin/display/camera2d";
import type { Display } from "gnoblin/display/display";
import { Camera3D } from "gnoblin/scene/camera";

// ---- Viewport -----------------------------------------------------------------------------------------------------

/** The state of one viewport, read by the manager. */
export interface ViewportState {
    /** The workspace last assigned or last seen shown. */
    content: object | null;
    camera: Camera2D | Camera3D | null;
    clearColor: string;
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
 * What a display shows and how it looks at it: a workspace (`content`) seen through a camera. Assigning `content` to
 * the viewport of a display shows that workspace on the display, as `gnoblin.workspaces.show` does, and reading it
 * gives what the display shows now (a workspace is on one display at a time). A `Camera3D` is accepted and kept, and
 * the shell reports once per reload that it draws a flat workspace only.
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

    /** The workspace the viewport shows. Null for a viewport that was made without one and has no display to ask. */
    get content(): object | null {
        const state = viewportState(this);
        if (state.display === null) {
            return state.content;
        }
        const shown = shell().shown(state.display);
        return shown === undefined ? state.content : shown;
    }
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
    set clearColor(value: string) {
        if (typeof value !== "string" || value === "") {
            throw new TypeError("Viewport.clearColor: must be a colour such as #101820");
        }
        viewportState(this).clearColor = value;
        shell().changed();
    }

    toString(): string {
        return "Viewport";
    }
}
