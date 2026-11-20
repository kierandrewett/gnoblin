import type { ParsedTrigger, TriggerButton } from "gnoblin-core/bindings/trigger_types";
import type { ErrorCode } from "gnoblin-core/errors";
/** A trigger spelling the grammar refuses. `token` is the part that is wrong. */
export declare class TriggerError extends TypeError {
    /** A spelling the grammar refuses is a bad argument. */
    readonly code: ErrorCode;
    /** The part of the trigger spelling that caused the error. */
    readonly token: string;
    /** The complete trigger spelling that was rejected. */
    readonly trigger: string;
    constructor(what: string, trigger: string, token: string, reason: string);
}
/** The kernel-side number of each button a trigger can name. */
export declare const BUTTON_NUMBER: Readonly<Record<TriggerButton, number>>;
/**
 * Parses a trigger into its parts and canonical spelling, or throws a `TriggerError` naming the token that
 * is wrong. `what` names the thing that holds the trigger, for the message.
 */
export declare function parseTrigger(trigger: unknown, what?: string): ParsedTrigger;
