// `Output.apply`: one request to the compositor's output engine and the answer that comes back a frame later.

import { outputRequest, supersampleNotice } from "gnoblin-core/output_config";
import type { OutputConfig } from "gnoblin-core/output_config";
import { errorCodeOfRefusal, GnoblinError } from "gnoblin-core/errors";
import type { ErrorCode } from "gnoblin-core/errors";
import { host } from "gnoblin-core/host/bridge";
import { emitMember, holders } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import { say } from "gnoblin-core/host/dispatch";
import { hooks } from "gnoblin-core/host/hooks";
import { isOutput, stateOf } from "gnoblin-core/host/node_state";
import type { Output } from "gnoblin-core/host/output";
import type { Operation } from "gnoblin-core/operation";

/** An output request awaiting its compositor reply and optional config notice path. */
export interface PendingApplication {
    /** Output receiving the request. */
    output: Output;
    /** The operation the caller holds; the compositor's answer settles it. */
    operation: Operation<Output>;
    /** The config path (`outputs.DP-1`) to name in a notice when the compositor refuses; null for a script's own
     * call. */
    path: string | null;
}

/**
 * The code of an `apply` the compositor refused, from the words it gave. The message of the failure stays those words,
 * which name the key (`transform: this compositor draws only normal`), so only the code is worked out here.
 */
export function applyFailureCode(refusal: string): ErrorCode {
    // A call the shell gave up on: the transaction that carried it was refused for a reason named in the text.
    const given = /^failed: the transaction was refused (\S+) /.exec(refusal);
    if (given !== null) {
        return errorCodeOfRefusal(given[1]);
    }
    if (refusal.startsWith("transform: ")) {
        return "unsupported";
    }
    if (refusal === "the output is not connected") {
        return "not_found";
    }
    return "denied";
}

/**
 * Sends one request for `output` to the compositor and returns the operation its answer settles: it succeeds with the
 * output, or fails with the compositor's own words, and the code `applyFailureCode` gives them. A request that cannot
 * be sent (a bad value, an output that is gone) throws instead.
 */
export function applyOutput(
    output: Output,
    options: Omit<OutputConfig, "mirrorOf"> & { mirrorOf?: Output | string | null },
    target: string,
    path: string | null = null,
): Operation<Output> {
    if (!stateOf(output).alive) {
        throw new GnoblinError("not_found", target, "the output is gone");
    }
    const { mirrorOf, ...rest } = options;
    const source = isOutput(mirrorOf) ? mirrorOf.connector : mirrorOf;
    const request = outputRequest(mirrorOf === undefined ? rest : { ...rest, mirrorOf: source }, target);
    // Unsupported supersample factors are accepted with a diagnostic. Repeating a profile after hotplug does not
    // repeat it.
    const notice = rest.supersample === undefined ? null : supersampleNotice(rest.supersample);
    if (notice !== null) {
        say(notice);
    }
    const token = host.outputs.apply(stateOf(output).node, request);
    if (token === undefined) {
        throw new GnoblinError("not_found", target, "the output is gone");
    }
    const operation = ctx.hostApi.operation<Output>(target);
    ctx.applications.set(token, { output, operation, path });
    return operation;
}

/** Settles each answer's operation, tells the output's listeners, and reports a refused config entry. */
export function settleApplications(): void {
    for (const answer of host.outputs.takeAnswers()) {
        const pending = ctx.applications.get(answer.token);
        if (pending === undefined) {
            continue;
        }
        ctx.applications.delete(answer.token);
        hooks.outputChange(pending.output, null);
        const error = answer.error === "" ? null : answer.error;
        if (error === null) {
            pending.operation.succeed(pending.output);
        } else {
            pending.operation.fail(applyFailureCode(error), error);
            if (pending.path !== null) {
                // `mode: ...` is a key's own refusal, so it extends the path; anything else is about the entry as a whole.
                const keyed = /^[a-z_]+: /.test(error);
                say(`${pending.path}${keyed ? "." : ": "}${error}`);
            }
        }
        if (stateOf(pending.output).alive) {
            emitMember(pending.output, holders(ctx.outputRoot, pending.output), "onApplied", [pending.output, error]);
        }
    }
}
