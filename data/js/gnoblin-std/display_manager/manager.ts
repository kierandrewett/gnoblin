// `DisplayManager`: what displays and profiles do to the shell. The reconciling is the library's (`libgnoblin-std`, `displays.rs`):
// it applies a profile when its monitor is connected (and again when the monitor returns), gives every output one display, asks the
// compositor to mirror or span the outputs of a display, and shows a viewport's workspace on the display's desk. This class is the
// shell's side of it: it keeps `gnoblin.displays` and `gnoblin.outputs.profiles` equal to the library's lists, tells the library
// what the script adds and removes, and reports what the cameras ask of a renderer that has none.

import { Camera3D } from "gnoblin-std/scene/camera";
import { Camera2D, isIdentity } from "gnoblin-std/display/camera2d";
import {
    Display,
    displayOfHandle,
    displayState,
    forgetHandle,
    modeNameOf,
    wrapDisplay,
} from "gnoblin-std/display/display";
import { profileState } from "gnoblin-std/display/output_profile";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import type { DisplayOutputCollection, OutputView } from "gnoblin-std/display/types";
import { viewportState } from "gnoblin-std/display/viewport";
import type { DisplayBackend } from "gnoblin-std/display_manager/backend";
import { Display as NativeDisplay, Workspace as NativeWorkspace } from "gnoblin-std/generated/classes";
import { DisplayFlags } from "gnoblin-std/generated/constants";

const CAMERA3D_NOTICE = "not applied: camera3d (needs a projected workspace)";
const CAMERA2D_NOTICE =
    "not applied: camera2d zoom, position and rotation (windows sit under the tree root: there is no per-display root to transform)";

/** Coordinates the shell's profiles, displays and outputs. */
export class DisplayManager {
    readonly #backend: DisplayBackend;
    readonly #profiles: OutputProfile[];
    readonly #displays: Display[];

    /** Creates a manager for the host's profile and display collections. */
    constructor(backend: DisplayBackend, profiles: OutputProfile[], displays: Display[]) {
        this.#backend = backend;
        this.#profiles = profiles;
        this.#displays = displays;
    }

    // ---- Profiles ----

    /** @throws GnoblinError `already_exists` when a profile with the same id is in the collection */
    addProfile(profile: OutputProfile): void {
        if (this.#profiles.includes(profile)) {
            return;
        }
        this.#backend.through("gnoblin.outputs.profiles.add", { busy: "already_exists" }, () =>
            profileState(profile).native.add(),
        );
        this.#profiles.push(profile);
        this.#backend.listed(profile);
        this.sync();
    }

    /** Stops managing the profile. The compositor keeps the arrangement it applied last. */
    removeProfile(profile: OutputProfile): boolean {
        const at = this.#profiles.indexOf(profile);
        if (at < 0) {
            return false;
        }
        profileState(profile).native.remove();
        this.#profiles.splice(at, 1);
        this.#backend.unlisted(profile);
        return true;
    }

    // ---- Displays ----

    /**
     * Adds a display the script made.
     *
     * @throws GnoblinError `busy` naming both displays when one of its outputs is presented by another, or
     *   `already_exists` when a display has its id
     */
    addDisplay(display: Display): void {
        if (this.#displays.includes(display)) {
            return;
        }
        this.#backend.through("gnoblin.displays.add", { busy: "already_exists", refused: "busy" }, () =>
            displayState(display).native.add(),
        );
        this.sync();
    }

    /**
     * Removes a script display: its mirror and span end and each of its outputs gets a display of the shell's.
     * Returns false for a display of the shell's own, which stays, and for one that is not a member.
     */
    removeDisplay(display: Display): boolean {
        if (!this.#displays.includes(display)) {
            return false;
        }
        const removed = displayState(display).native.remove();
        this.sync();
        return removed;
    }

    /**
     * Replaces `old` with `next` as one validated change. Displays that share an output with `next` leave, and `next`
     * takes `old`'s place. Nothing changes if validation fails.
     *
     * @throws GnoblinError `invalid_argument` when `old` is not in the collection, or `already_exists` when `next` is
     *   present or has an id that remains in use
     */
    replaceDisplay(old: Display, next: Display): void {
        this.#backend.through(
            "gnoblin.displays.replace",
            { busy: "already_exists", refused: "busy", stale: "invalid_argument" },
            () => displayState(old).native.replace(displayState(next).native),
        );
        this.sync();
    }

    /** Removes every display of the script. */
    clearDisplays(): void {
        NativeDisplay.clear();
        this.sync();
    }

    /**
     * The display under `point` in the compositor's space, else the primary output's; null when no display has an
     * output.
     */
    displayAt(point: { x: number; y: number }): Display | null {
        const found = NativeDisplay.at(point.x, point.y);
        this.sync();
        return found === null ? null : (displayOfHandle(found.handle) ?? null);
    }

    /** What the display's viewport shows: the workspace on its output, else the one it last showed or was given. */
    shown(display: Display): object | null {
        try {
            const info = displayState(display).native.info();
            const handle = (info.flags & DisplayFlags.HAS_DESK) !== 0 ? info.shown : info.content;
            return handle === 0 ? null : this.#backend.workspaceOf(handle);
        } catch (error) {
            // A display the library dropped with its output shows nothing.
            if (this.#backend.isStale(error)) {
                return null;
            }
            throw error;
        }
    }

    /**
     * Shows `workspace` on the display, or keeps it for when the display has an output.
     *
     * @throws GnoblinError as `gnoblin.workspaces.show` does, when the workspace is not in `gnoblin.workspaces`
     */
    assignContent(display: Display, workspace: object): void {
        this.#backend.through(`${workspace}.show`, { refused: "invalid_state" }, () =>
            displayState(display).native.assign(new NativeWorkspace(this.#backend.workspaceHandle(workspace))),
        );
    }

    /** The `Collection<Output>` of the outputs the display presents. */
    outputsOf(display: Display): DisplayOutputCollection {
        const state = displayState(display);
        if (state.outputs === null) {
            state.outputs = this.#backend.outputSet();
            state.outputs.set(this.#outputsNow(display));
        }
        return state.outputs.collection;
    }

    /**
     * What a transition callback can change that is not a tree write, to put back when it fails: the displays' wanted
     * workspaces (the library keeps them), cameras and colours.
     */
    snapshot(): () => void {
        const token = NativeDisplay.snapshot();
        const saved = this.#displays.map((display) => {
            const view = viewportState(displayState(display).viewport);
            return { view, camera: view.camera, color: view.clearColor };
        });
        return () => {
            NativeDisplay.restore(token);
            for (const item of saved) {
                Object.assign(item.view, { camera: item.camera, clearColor: item.color });
            }
            this.sync();
        };
    }

    /** Called each layout pass: catches up with the library, then reports what the cameras ask of the renderer. */
    frame(): void {
        this.sync();
        for (const display of this.#displays) {
            const view = viewportState(displayState(display).viewport);
            const camera = view.camera;
            if (camera instanceof Camera3D) {
                this.#backend.say(CAMERA3D_NOTICE);
            } else if (camera instanceof Camera2D && !isIdentity(camera)) {
                this.#backend.say(CAMERA2D_NOTICE);
            }
            if (view.clearColor !== "#000000") {
                this.#backend.say(
                    "not applied: viewport.clearColor (the compositor has no background colour behind a workspace)",
                );
            }
        }
    }

    /** Has the library reconcile the profiles and displays now, and catches up with what it did. */
    reflow(): void {
        NativeDisplay.reflow();
        this.sync();
    }

    /** Makes `gnoblin.displays` and the outputs each display presents equal to the library's. */
    sync(): void {
        const next = NativeDisplay.list().map((handle) => this.#displayOf(handle));
        const before = [...this.#displays];
        const gone = before.filter((display) => !next.includes(display));
        const joined = next.filter((display) => !before.includes(display));
        if (gone.length > 0 || joined.length > 0 || next.some((display, at) => display !== before[at])) {
            this.#displays.splice(0, this.#displays.length, ...next);
        }
        for (const display of gone) {
            this.#backend.unlisted(display);
            // A display the library made for an output is dropped for good with its output.
            if (displayState(display).implicit) {
                forgetHandle(displayState(display).native.handle);
            }
        }
        for (const display of joined) {
            this.#backend.listed(display);
        }
        for (const display of next) {
            displayState(display).outputs?.set(this.#outputsNow(display));
        }
    }

    /** The object of a display of the library's handle: the script's own, or one made for a display the library keeps. */
    #displayOf(handle: number): Display {
        const known = displayOfHandle(handle);
        if (known !== undefined) {
            return known;
        }
        const native = new NativeDisplay(handle);
        const info = native.info();
        const output = native
            .outputs()
            .map((node) => this.#backend.outputOf(node))
            .find((one): one is OutputView => one !== null);
        if (output === undefined) {
            throw new TypeError(`display ${info.id} has no output to be presented on`);
        }
        return wrapDisplay(native, info.id, modeNameOf(info.mode), output);
    }

    /** The outputs the library says the display presents. */
    #outputsNow(display: Display): OutputView[] {
        return displayState(display)
            .native.outputs()
            .map((node) => this.#backend.outputOf(node))
            .filter((output): output is OutputView => output !== null);
    }
}
