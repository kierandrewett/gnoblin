// The shapes the host API shares: what the session and the runtime answer with, the events the compositor raises,
// and what the shell hands `HostApi.frame`. Types only.

import type { ErrorCode } from "gnoblin/errors";
import type { ClockHost } from "gnoblin/operation";
import type { ExitEvent } from "gnoblin/process";
import type { TrackEndedEvent } from "gnoblin/transition/types";

/** The lock states the compositor reports. */
export type LockState = "unlocked" | "covering" | "locked" | "failsafe";

/**
 * The session as the compositor last published it. `state` is `running` while the shell has a connection to the
 * compositor and `unavailable` when it has none. `lockState` is present only while `lockAvailable` is true:
 * unavailable does not mean unlocked.
 */
export interface SessionStatus {
    readonly state: "running" | "unavailable";
    readonly lockAvailable: boolean;
    /** How many lock state changes the session has published. */
    readonly revision: number;
    readonly lockState?: LockState;
}

/** What `logout`, `poweroff`, `reboot`, `suspend` and `hibernate` succeed with: the compositor took the request. */
export interface Accepted {
    readonly accepted: true;
}

/** What `lock` succeeds with: a shell client received the request. Read `status()` to see the session lock. */
export interface LockRequest {
    readonly dispatched: boolean;
}

/** What `gnoblin.runtime.reloadConfig()` succeeds with once the compositor accepted the reloaded document. */
export interface ReloadResult {
    readonly ok: true;
    readonly action: "config reload";
    /** The revision of the settings the compositor now holds. */
    readonly settingsRevision: number;
    /** The sections the commit dropped, one sentence each. */
    readonly warnings: readonly string[];
    /** The settings accepted but not applied (read only at a process's start, or by nothing), one sentence each. */
    readonly notApplied: readonly string[];
}

/** The build identity. A field the build does not know is `"unknown"`; remote credentials are removed. */
export interface Version {
    readonly gnoblin: string;
    readonly gnome: string;
    readonly mutter: string;
    /** The script engine. */
    readonly js: string;
    /** The control API version, `major.minor.patch`. */
    readonly api: string;
    readonly gitRemote: string;
    readonly gitSha: string;
    readonly buildId: string;
}

/** A schema signal as a handler receives it: its name, the object that raised it, and its fields in camelCase. */
export interface SignalEvent {
    /** `namespace.object.signal`, with underscores. */
    readonly name: string;
    /** The identity of the object that raised it. */
    readonly object: number;
    readonly [field: string]: unknown;
}

/** What the shell hands the module: the host's functions, the queue and the clock. */
export interface ApiHost extends ClockHost {
    /** The native spawn: argv and options as JSON; answers `{id, pid}` or `{error}` as JSON. */
    spawn(argv: string, options: string): string;
    /** The native kill: an error as JSON, or undefined when the signal was sent. */
    kill(id: number, signal: string): string | undefined;
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
    /** The schema's signals as JSON: `[{name, fields}]`. */
    signals(): string;
    /** True while the user's entry module is being evaluated. */
    loading(): boolean;
}

/** What the host API reads of the shell. */
export type Host = ApiHost;

/** A failed call: its error code and message. */
export interface Failure {
    code: string;
    message: string;
}

/** A compositor call that finished, by its token. */
export interface OperationEvent {
    kind: "op";
    ticket: number;
    ok: boolean;
    value?: unknown;
    error?: Failure;
}

/** A signal the compositor raised. */
export interface SignalMessage {
    kind: "signal";
    name: string;
    object: number;
    fields: Record<string, unknown>;
}

interface LostMessage {
    kind: "lost";
    message: string;
}

/** The compositor's `track_ended` signal, as the host collects it (`data/js/gnoblin/transition.ts`). */
interface TrackEndedMessage extends TrackEndedEvent {
    kind: "trackEnded";
}

/** Anything the shell hands `HostApi.frame`. */
export type HostEvent = OperationEvent | SignalMessage | LostMessage | ExitEvent | TrackEndedMessage;
