// What the display manager needs of the shell: the `DisplayBackend` the host implements and the `OutputSet` it hands
// each display.

import type { Display } from "gnoblin/display/display";
import type { OutputSetting } from "gnoblin/display/output_config";
import type { OutputProfile } from "gnoblin/display/output_profile";
import type { DisplayOutputCollection, OutputView } from "gnoblin/display/types";
import type { ErrorCode } from "gnoblin/errors";

/** A `Collection<Output>` the manager keeps equal to the outputs a display presents. */
export interface OutputSet {
    readonly collection: DisplayOutputCollection;
    set(outputs: readonly OutputView[]): void;
}

/** What the manager needs of the shell: outputs, desks, the output requests and the events of the collections. */
export interface DisplayBackend {
    /** The connected outputs, in the order the compositor made them. */
    outputs(): readonly OutputView[];
    /** The output flagged primary, else the first. */
    primary(): OutputView | null;
    /** Whether the output has a desk of its own: it is on and not a mirror. */
    hasDesk(output: OutputView): boolean;
    /** The workspace the output shows (a spanned output shows the workspace its span does), or null. */
    shownOn(output: OutputView): object | null;
    /** Whether the workspace is in `gnoblin.workspaces`. */
    attached(workspace: object): boolean;
    /** Shows `workspace` on `output` as `gnoblin.workspaces.show` does; throws as it does. */
    show(workspace: object, output: OutputView): void;
    /** Makes `lead` show one workspace over the union of `members`; null ends the span. */
    span(lead: OutputView, members: readonly OutputView[] | null): void;
    /** The output `output` mirrors, or null. */
    mirrorOf(output: OutputView): OutputView | null;
    /** Mirrors `source` on `output`, or stops mirroring with null. */
    mirror(output: OutputView, source: OutputView | null, path: string): void;
    /** Sends one output request; a refusal is reported under `path`. */
    apply(output: OutputView, setting: OutputSetting, path: string): void;
    /** Whether a request for `output` has not been answered yet. */
    pending(output: OutputView): boolean;
    /** Reports `text` once per generation. */
    say(text: string): void;
    error(code: ErrorCode, target: string, message: string): Error;
    /** A new collection of outputs for one display. */
    outputSet(): OutputSet;
    /** The collections' events: the member joined or left. */
    listed(member: Display | OutputProfile): void;
    unlisted(member: Display | OutputProfile): void;
}
