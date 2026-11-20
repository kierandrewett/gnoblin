import { GestureBinding } from "gnoblin-core/bindings/binding";
import type { BindingOptions, DragEvent } from "gnoblin-core/bindings/types";
/** What the slide needs of a window. */
export interface SlideClient {
    /** The window's horizontal position, in logical pixels. */
    x: number;
    /** Whether the window is visible on screen. */
    visible: boolean;
    /** Whether the window still exists. */
    readonly alive: boolean;
}
/** What it needs of a workspace. */
export interface SlideWorkspace {
    /** Whether this is the active workspace. */
    readonly active: boolean;
    /** The position among the workspaces. */
    readonly index: number;
    /** Windows on this workspace. */
    readonly clients: Iterable<SlideClient>;
}
/** What it needs of the shell: the `gnoblin` global. */
export interface SlideHost {
    /** Workspaces in their current order. */
    readonly workspaces: {
        /** The area windows are laid out in; null while there is no screen. */
        readonly area: {
            readonly width: number;
        } | null;
        at(index: number): SlideWorkspace | undefined;
        show(workspace: SlideWorkspace, options: {
            duration: number;
            progress: number;
        }): void;
        [Symbol.iterator](): Iterator<SlideWorkspace>;
    };
    /** Creates a timer that fires after the delay, in milliseconds. */
    readonly timers: {
        /** Runs `run` once, `ms` milliseconds from now. */
        after(ms: number, run: () => void): unknown;
    };
    /** The milliseconds an animation takes: 0 while the seat asks for reduced motion. Absent, the time it was given. */
    motion?(duration: number): number;
    /** Runs `change` once and plays what it writes over `options.duration` milliseconds along the easing. */
    requestTransition(change: () => void, options: {
        duration: number;
        easing: "ease-out";
    }): unknown;
}
/**
 * The options of `WorkspaceSlideBinding`. Every field is optional; `id` is "workspace-slide" and `trigger` "Swipe3".
 */
export interface WorkspaceSlideOptions extends Partial<BindingOptions> {
    /** How far fingers move for a slide of one whole workspace, in touchpad units. Default 400. */
    travel?: number;
    /** The share of a slide at which lifting the fingers finishes it, from 0 to 1. Default 0.25. */
    commit?: number;
    /** Milliseconds a whole slide takes, which a slide finished part way scales by what is left. Default 250. */
    duration?: number;
    /** The shell to slide in. Default the `gnoblin` global. */
    host?: SlideHost;
}
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
/**
 * A touchpad swipe that slides the workspace in the direction of the fingers.
 *
 * @example
 * bindings.add(new WorkspaceSlideBinding({ travel: 500, commit: 0.3 }));
 */
export declare class WorkspaceSlideBinding extends GestureBinding {
    #private;
    /** Finger travel for a slide of one whole workspace, in touchpad units. */
    travel: number;
    /** The share of a slide at which lifting finishes it. */
    commit: number;
    /** Milliseconds a whole slide takes. */
    duration: number;
    constructor(options?: WorkspaceSlideOptions);
    /** Refuses the gesture when there is no workspace to leave or no screen; else records where each window rests. */
    onStart(_event: DragEvent): SlideState | false;
    /** Follows the fingers: picks the target workspace on the first horizontal move and re-places every window. */
    onUpdate(event: DragEvent, state: object): void;
    /**
     * Finishes the slide through `workspaces.show` when it passed `commit` and went mostly sideways. Otherwise it
     * settles back.
     */
    onEnd(event: DragEvent, state: object): void;
    /** A cancelled gesture slides back like a short one. */
    onCancel(_event: DragEvent, state: object): void;
}
export {};
