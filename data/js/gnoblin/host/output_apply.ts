// `Output.apply`: one request to the compositor's output engine and the answer that comes back a frame later.

import { outputRequest, supersampleNotice } from "gnoblin/display/output_config";
import type { OutputConfig } from "gnoblin/display/output_config";
import { GnoblinError } from "gnoblin/errors";
import { host } from "gnoblin/host/bridge";
import { emitMember, holders } from "gnoblin/host/collection_events";
import { ctx } from "gnoblin/host/context";
import { say } from "gnoblin/host/dispatch";
import { isOutput, stateOf } from "gnoblin/host/node_state";
import type { Output } from "gnoblin/host/output";

/** The compositor's answer to an `Output.apply`, filled in a frame after the call. */
export interface OutputApplication {
    /** Whether the compositor has answered. */
    readonly done: boolean;
    /** Null while waiting and on success; else why the arrangement was left as it was, naming the key. */
    readonly error: string | null;
}

/** An output request awaiting its compositor reply and optional config notice path. */
export interface PendingApplication {
    output: Output;
    result: { done: boolean; error: string | null };
    /** The config path (`outputs.DP-1`) to name in a notice when the compositor refuses; null for a script's own call. */
    path: string | null;
}

/** Sends one request for `output` to the compositor and tracks its answer. */
export function applyOutput(
    output: Output,
    options: Omit<OutputConfig, "mirrorOf"> & { mirrorOf?: Output | string | null },
    target: string,
    path: string | null = null,
): OutputApplication {
    if (!stateOf(output).alive) {
        throw new GnoblinError("not_found", target, "the output is gone");
    }
    const { mirrorOf, ...rest } = options;
    const source = isOutput(mirrorOf) ? mirrorOf.connector : mirrorOf;
    const request = outputRequest(mirrorOf === undefined ? rest : { ...rest, mirrorOf: source }, target);
    // A factor that is not 1, 2 or 4 is accepted, with one diagnostic the owner can see: `say` repeats nothing, so the
    // same factor asked for again (a profile re-applied after a hotplug, say) is not reported twice.
    const notice = rest.supersample === undefined ? null : supersampleNotice(rest.supersample);
    if (notice !== null) {
        say(notice);
    }
    const token = host.outputs.apply(stateOf(output).node, request);
    if (token === undefined) {
        throw new GnoblinError("not_found", target, "the output is gone");
    }
    const result = { done: false, error: null as string | null };
    ctx.applications.set(token, { output, result, path });
    return {
        get done() {
            return result.done;
        },
        get error() {
            return result.error;
        },
    };
}

/** Hands each answer to its application, tells the output's listeners, and reports a refused config entry. */
export function settleApplications(): void {
    for (const answer of host.outputs.takeAnswers()) {
        const pending = ctx.applications.get(answer.token);
        if (pending === undefined) {
            continue;
        }
        ctx.applications.delete(answer.token);
        ctx.displayManager.changed();
        pending.result.done = true;
        pending.result.error = answer.error === "" ? null : answer.error;
        if (pending.result.error !== null && pending.path !== null) {
            // `mode: ...` is a key's own refusal, so it extends the path; anything else is about the entry as a whole.
            const keyed = /^[a-z_]+: /.test(pending.result.error);
            say(`${pending.path}${keyed ? "." : ": "}${pending.result.error}`);
        }
        if (stateOf(pending.output).alive) {
            emitMember(pending.output, holders(ctx.outputRoot, pending.output), "onApplied", [
                pending.output,
                pending.result.error,
            ]);
        }
    }
}
