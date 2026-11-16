// Data shapes shared by the script host APIs and compositor event handling.

import type { ErrorCode } from "gnoblin-core/errors";
import type { ClockHost } from "gnoblin-core/operation";
import type { ExitEvent } from "gnoblin-core/process";
import type { TrackEndedEvent } from "gnoblin-std/transition/types";

/** Lock states reported by the compositor. */
export type LockState = "unlocked" | "covering" | "locked" | "failsafe";

/**
 * The latest published session state. `unavailable` means there is no compositor connection, not that the session is
 * unlocked. `lockState` is present only when `lockAvailable` is true.
 */
export interface SessionStatus {
    /** Whether the session connection is available. */
    readonly state: "running" | "unavailable";
    /** Whether the shell can report lock state. */
    readonly lockAvailable: boolean;
    /** How many lock state changes the session has published. */
    readonly revision: number;
    /** Current lock state, present when `lockAvailable` is true. */
    readonly lockState?: LockState;
}

/** The compositor accepted a logout, power, reboot, suspend or hibernate request. */
export interface Accepted {
    /** Always true when the compositor accepts the request. */
    readonly accepted: true;
}

/** A shell client received the lock request. Read `status()` to check the session lock. */
export interface LockRequest {
    /** Whether the shell client received the lock request. */
    readonly dispatched: boolean;
}

/** The result of a successful configuration reload. */
export interface ReloadResult {
    /** Always true for a successful reload result. */
    readonly ok: true;
    /** The action completed by the reload. */
    readonly action: "config reload";
    /** The revision of the settings the compositor now holds. */
    readonly settingsRevision: number;
    /** The sections the commit dropped, one sentence each. */
    readonly warnings: readonly string[];
    /** The settings accepted but not applied (read only at a process's start, or by nothing), one sentence each. */
    readonly notApplied: readonly string[];
}

/** Build identity fields from `gnoblin.version`. Unknown fields are `"unknown"`; remote credentials are removed. */
export interface Version {
    /** Gnoblin version. */
    readonly gnoblin: string;
    /** GNOME version. */
    readonly gnome: string;
    /** Mutter version. */
    readonly mutter: string;
    /** The script engine. */
    readonly js: string;
    /** The control API version, `major.minor.patch`. */
    readonly api: string;
    /** Sanitised Git remote URL. */
    readonly gitRemote: string;
    /** Git commit identifier. */
    readonly gitSha: string;
    /** Build identifier. */
    readonly buildId: string;
}

/** Functions, queue and clock supplied by the shell to the host API. */
export interface ApiHost extends ClockHost {
    /** The native spawn: argv and options as JSON; answers `{id, pid}` or `{error}` as JSON. */
    spawn(argv: string, options: string): string;
    /** The native kill: an error as JSON, or undefined when the signal was sent. */
    kill(id: number, signal: string): string | undefined;
    /** The operating system's process id of a process, or 0 while its program has not started or after it has ended. */
    pid(id: number): number;
    /** A fresh operation id. */
    operationId(): number;
    /** The error a failed call throws: `code` says why, `target` names the call. */
    error(code: ErrorCode, target: string, message: string): Error;
    /** Starts a session verb and answers `{ticket}` or `{error}` as JSON. */
    begin(verb: string): string;
    /** The session's published state as JSON, or undefined when the shell has no session connection. */
    status(): string | undefined;
    /** The build identity as JSON. */
    version(): string;
    /** True while the user's entry module is being evaluated. */
    loading(): boolean;
}

/** The host API's view of the shell functions. */
export type Host = ApiHost;

/** The error code and message for a failed call. */
export interface Failure {
    /** Error code returned by the failed call. */
    code: string;
    /** Human-readable error message. */
    message: string;
}

/** A compositor call response, identified by its ticket. */
export interface OperationEvent {
    /** Identifies this message as a compositor operation response. */
    kind: "op";
    /** Ticket identifying the pending call. */
    ticket: number;
    /** Whether the operation succeeded. */
    ok: boolean;
    /** Successful operation result, when present. */
    value?: unknown;
    /** Failure details, when the operation failed. */
    error?: Failure;
}

/** A compositor signal as received by the host. */
export interface SignalMessage {
    /** Identifies this message as a compositor signal. */
    kind: "signal";
    /** Signal name. */
    name: string;
    /** Identity of the object that raised the signal. */
    object: number;
    /** Signal fields, keyed by their wire names. */
    fields: Record<string, unknown>;
}

interface LostMessage {
    kind: "lost";
    message: string;
}

/** A compositor `track_ended` signal, collected by the host. */
interface TrackEndedMessage extends TrackEndedEvent {
    kind: "trackEnded";
}

/** Events delivered to `HostApi.frame`. */
export type HostEvent = OperationEvent | SignalMessage | LostMessage | ExitEvent | TrackEndedMessage;
