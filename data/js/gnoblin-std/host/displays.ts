// Connects logical displays and output profiles to the compositor's connected outputs.

import { Display } from "gnoblin-std/display/display";
import { attachDisplays } from "gnoblin-std/display/state";
import type { OutputProfile } from "gnoblin-std/display/output_profile";
import { DisplayManager } from "gnoblin-std/display_manager/manager";
import type { DisplayBackend, OutputSet } from "gnoblin-std/display_manager/backend";
import { GnoblinError } from "gnoblin-core/errors";
import type { ErrorCode } from "gnoblin-core/errors";
import { CREATE } from "gnoblin-core/host/bridge";
import { emitMember, holders, recompute } from "gnoblin-core/host/collection_events";
import { collectionState, rootState } from "gnoblin-core/host/collection_state";
import { Collection } from "gnoblin-core/host/collections";
import { ctx } from "gnoblin-core/host/context";
import { std } from "gnoblin-std/host/state";
import { failureCode, FAILED, nativeOutput, throughLibrary } from "gnoblin-std/host/library";
import { Collection as NativeCollection, Workspace as NativeWorkspace } from "gnoblin-std/generated/classes";
import { CollectionKind, EventKind } from "gnoblin-std/generated/constants";
import { queueFor, say } from "gnoblin-core/host/dispatch";
import { Output } from "gnoblin-core/host/output";
import { stateOf } from "gnoblin-core/host/node_state";
import { Workspace } from "gnoblin-std/host/workspace";
import { workspaceOf, wsState } from "gnoblin-std/host/workspace_state";

/** Whether two lists hold the same outputs in the same order. */
function sameOutputs(a: readonly Output[], b: readonly Output[]): boolean {
    return a.length === b.length && a.every((output, at) => output === b[at]);
}

/** A `Collection<Output>` for one display, kept equal to what the display presents. */
function makeOutputSet(): OutputSet {
    const collection = new Collection<Output>(CREATE, rootState("Output"));
    const state = collectionState(collection);
    return {
        collection,
        set(outputs) {
            const before = state.list as Output[];
            if (sameOutputs(before, outputs as Output[])) {
                return;
            }
            state.list = [...outputs] as Output[];
            for (const output of outputs as Output[]) {
                if (!before.includes(output)) {
                    queueFor(state, "onAdded", [output, collection]);
                }
            }
            for (const output of before) {
                if (!outputs.includes(output)) {
                    queueFor(state, "onRemoved", [output, collection]);
                }
            }
        },
    };
}

/** Tells the collection that holds `member` that it joined (`onAdded`) or left (`onRemoved`). */
function announceMember(member: Display | OutputProfile, joined: boolean): void {
    const root = member instanceof Display ? std.displayRoot : std.profileRoot;
    if (joined) {
        queueFor(root, "onAdded", [member, root.self]);
        recompute(root, member);
        return;
    }
    for (const view of holders(root, member)) {
        view.members?.delete(member);
        queueFor(view, "onRemoved", [member, view.self]);
    }
}

/** Makes the display manager and attaches `Display` to it. */
export function installDisplays(): void {
    const displayBackend: DisplayBackend = {
        outputOf: (node) => (ctx.outputsByNode.get(node) as Output | undefined) ?? null,
        workspaceOf: (handle) => workspaceOf(new NativeWorkspace(handle)),
        workspaceHandle: (workspace) => wsState(workspace as Workspace).native.handle,
        outputNode: (output) => nativeOutput(output as Output).node,
        through(target, known, call) {
            const codes: Record<number, ErrorCode> = {};
            if (known.busy !== undefined) codes[FAILED.busy] = known.busy;
            if (known.refused !== undefined) codes[FAILED.refused] = known.refused;
            if (known.stale !== undefined) codes[FAILED.stale] = known.stale;
            return throughLibrary(target, codes, () => {
                try {
                    return call();
                } catch (error) {
                    // The library's words start with the name of the code; the script's error is about the call.
                    if (error instanceof Error) {
                        error.message = error.message.replace(/^E_[A-Z_]+: /, "");
                    }
                    throw error;
                }
            });
        },
        isStale: (error) => failureCode(error) === FAILED.stale,
        say,
        outputSet: makeOutputSet,
        listed: (member) => announceMember(member, true),
        unlisted: (member) => announceMember(member, false),
    };

    // The library reconciles the script's profiles and displays with the connected outputs; the manager keeps the shell's
    // collections equal to what it did.
    std.displayManager = new DisplayManager(
        displayBackend,
        std.profileRoot.list as OutputProfile[],
        std.displayRoot.list as Display[],
    );

    // What the library says about displays reaches the shell as events of the root collection of outputs: the lists changed, an
    // answer to a request it sent for an output, and a sentence for the diagnostics.
    const connected = NativeCollection.root(CollectionKind.OUTPUTS);
    connected?.listen(EventKind.DISPLAYS, 0, () => std.displayManager.sync());
    connected?.listen(EventKind.NOTE, 0, (event) => say(event.text));
    connected?.listen(EventKind.APPLIED, 0, (event) => {
        const output = ctx.outputsByNode.get(event.member) as Output | undefined;
        if (output !== undefined && stateOf(output).alive) {
            emitMember(output, holders(ctx.outputRoot, output), "onApplied", [
                output,
                event.text === "" ? null : event.text,
            ]);
        }
    });

    attachDisplays({
        isWorkspace: (value) => value instanceof Workspace,
        isOutput: (value) => value instanceof Output,
        error: (code, target, message) => new GnoblinError(code, target, message),
        shown: (display) => std.displayManager.shown(display),
        show: (display, workspace) => std.displayManager.assignContent(display, workspace),
        outputs: (display) => std.displayManager.outputsOf(display),
        workspaceHandle: (workspace) => wsState(workspace as Workspace).native.handle,
        outputNode: (output) => nativeOutput(output as Output).node,
    });
}
