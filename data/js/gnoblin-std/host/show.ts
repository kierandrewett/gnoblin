// Shows workspaces on outputs and records the slide for the next layout pass.

import { GnoblinError } from "gnoblin-core/errors";
import { ctx } from "gnoblin-core/host/context";
import { currentDesk, effectiveDesk, ensureDesk, setShown } from "gnoblin-std/host/desks";
import { isOutput } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import { motion } from "gnoblin-core/host/seat";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";

/** Default duration, in milliseconds, of a workspace slide. */
export const SLIDE_DURATION = 250;

/** Options for showing a workspace. */
export interface ShowOptions {
    /** Milliseconds the slide between the two workspaces takes. Zero or absent switches at once. */
    duration?: number;
    /** How much of the slide a touchpad gesture has made already, 0 to 1: the slide carries on from there. */
    progress?: number;
    /** The output to show it on. Left out, the one under the pointer, else the primary one. */
    output?: Output;
}

/** A slide between workspaces that the next layout pass starts. */
export interface Slide {
    /** Workspace being left. */
    from: Workspace;
    /** Workspace being shown. */
    to: Workspace;
    /** 1 when `to` comes after `from`, so the windows move left. */
    direction: 1 | -1;
    /** Slide duration in milliseconds. */
    duration: number;
    /** Fraction of the slide already completed, from 0 to 1. */
    progress: number;
}

/** Shows a workspace by object or id, optionally sliding from the current workspace. */
export function showWorkspace(workspace: Workspace | string, options: ShowOptions = {}): void {
    const target = typeof workspace === "string" ? ctx.workspaceRoot.list.find((w) => w.id === workspace) : workspace;
    if (target === undefined) {
        throw new GnoblinError("not_found", "gnoblin.workspaces.show", `there is no workspace "${workspace}"`);
    }
    if (!wsState(target).attached) {
        throw new GnoblinError("invalid_state", `${target}.show`, "add the workspace to gnoblin.workspaces first");
    }
    const asked = options.duration ?? 0;
    const progress = options.progress ?? 0;
    if (typeof asked !== "number" || !Number.isFinite(asked) || asked < 0) {
        throw new TypeError(`${target}.show: duration must be a finite number of milliseconds of at least 0`);
    }
    if (typeof progress !== "number" || !(progress >= 0 && progress <= 1)) {
        throw new TypeError(`${target}.show: progress must be a number from 0 to 1`);
    }
    if (options.output !== undefined && !isOutput(options.output)) {
        throw new TypeError(`${target}.show: output must be an Output`);
    }
    if (options.output !== undefined && !options.output.alive) {
        throw new GnoblinError("not_found", `${target}.show`, `${options.output} is gone`);
    }
    const duration = motion(asked);
    ensureDesk();
    const own = options.output === undefined ? currentDesk() : (ctx.deskByOutput.get(options.output) ?? null);
    const desk = own === null ? null : effectiveDesk(own);
    if (desk === null) {
        throw new GnoblinError("unavailable", `${target}.show`, "there is no output to show a workspace on");
    }
    const from = desk.shown;
    // A workspace can appear on only one desk, so swap the two desks' shown workspaces.
    const other = ctx.desks.find((candidate) => candidate !== desk && candidate.shown === target);
    setShown(desk, target as Workspace);
    if (other !== undefined) {
        setShown(other, from);
    }
    if (from !== null && from !== target && duration > 0) {
        ctx.slides.push({
            from,
            to: target as Workspace,
            direction: ctx.workspaceRoot.list.indexOf(target) > ctx.workspaceRoot.list.indexOf(from) ? 1 : -1,
            duration,
            progress,
        });
    }
}
