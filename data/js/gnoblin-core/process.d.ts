import { Operation } from "gnoblin-core/operation";
import type { OperationHost } from "gnoblin-core/operation";
import type { ErrorCode } from "gnoblin-core/errors";
/** Where one standard stream goes: the session log, nowhere, or into the end event (up to `outputLimit` bytes). */
export type StreamMode = "inherit" | "discard" | "capture";
/** The signals `Process.kill` sends, to the process group. */
export type ProcessSignal = "term" | "kill" | "int" | "hup" | "usr1" | "usr2" | "stop" | "cont";
/** The options of `spawn`. Every one is optional; a name that is not listed is refused with a `TypeError`. */
export interface SpawnOptions {
    /** The working directory: an absolute path. The runtime's own by default. */
    cwd?: string;
    /** Variables added to the session environment, at most 128. */
    env?: Record<string, string>;
    /** Text written to standard input, at most 1 MiB, which is then closed. Standard input is empty by default. */
    stdin?: string;
    /** Where standard output goes. `inherit` by default. */
    stdout?: StreamMode;
    /** Where standard error goes. `inherit` by default. */
    stderr?: StreamMode;
    /** Bytes kept from each captured stream, 0 to 4194304. 65536 by default. */
    outputLimit?: number;
    /** Milliseconds before the group gets SIGTERM, and SIGKILL 2000 ms later. More than 0, at most 86400000. */
    timeout?: number;
}
/** How a process ended. A captured stream is decoded as UTF-8, with the replacement character for bad bytes. */
export interface ProcessExit {
    /** The exit status after a normal exit; null when a signal ended it. */
    readonly exitCode: number | null;
    /** The name of the signal that ended it (`term`, `kill`, ...); null after a normal exit. */
    readonly signal: string | null;
    /** The `timeout` option ended it. */
    readonly timedOut: boolean;
    /** What standard output held, for a stream set to `capture`; null otherwise. */
    readonly stdout: string | null;
    /** What standard error held, for a stream set to `capture`; null otherwise. */
    readonly stderr: string | null;
    /** Standard output passed `outputLimit` and the rest was dropped. */
    readonly stdoutTruncated: boolean;
    /** Standard error passed `outputLimit` and the rest was dropped. */
    readonly stderrTruncated: boolean;
}
/** Observes the processes the script started. Every method is optional. */
export declare class ProcessListener {
    /**
     * A process ended, however it ended. `code` is its exit status, or null when a signal ended it (`process.signal`
     * says which).
     */
    onExit?(process: Process, code: number | null): void;
}
/** What the table needs of the shell, beside the queue and the clock. */
export interface ProcessHost extends OperationHost {
    /** The native spawn: argv as JSON, options as JSON; answers `{id, pid}` or `{error}` as JSON. */
    spawn(argv: string, options: string): string;
    /** The native kill: an error as JSON, or undefined when the signal was sent. */
    kill(id: number, signal: string): string | undefined;
    /** The operating system's process id of a process, or 0 while its program has not started or after it has ended. */
    pid(id: number): number;
    /** A fresh operation id. */
    operationId(): number;
    /** The error a failed call throws: `code` says why, `target` names the call. */
    error(code: ErrorCode, target: string, message: string): Error;
}
/** The event the host reports when a process ends. */
export interface ExitEvent extends ProcessExit {
    /** Discriminant identifying this as a process-end event. */
    kind: "exit";
    /** The id of the process that ended. */
    id: number;
}
/**
 * One started program. `alive` is true from the spawn until its end is reported, and the fields that say how it
 * ended are `null` until then.
 */
export declare class Process {
    #private;
    /** The id the shell gave the process. It is the handle's identity within this generation. */
    readonly id: number;
    /** The program and its arguments. A command string is `["/bin/sh", "-c", command]`. */
    readonly argv: readonly string[];
    /** Only the table makes processes: a script receives them from `spawn`, `list` and `byId`. */
    constructor(table: ProcessTable, id: number, pid: number, argv: readonly string[]);
    /**
     * The operating system's process id. A program the config's top level spawns starts when the config takes over, so
     * until then its `pid` is 0; a program the same config started before and that a reload keeps has its pid at once.
     */
    get pid(): number;
    /** True from the spawn until the host reports the end. */
    get alive(): boolean;
    /** `running` until the host reports the end, then `exited`. */
    get status(): "running" | "exited";
    /** The exit status, or null while the process runs and after a signal ended it. */
    get exitCode(): number | null;
    /** The name of the signal that ended the process, or null. */
    get signal(): string | null;
    /** True when the `timeout` option ended the process. */
    get timedOut(): boolean;
    /** What the process ended with, or null while it runs. */
    get exit(): ProcessExit | null;
    /**
     * An operation that succeeds when the process ends, whatever its exit status, with how it ended. Every call
     * returns the same operation.
     */
    wait(): Operation<ProcessExit>;
    /**
     * Sends a signal to the process group (`term` by default). The operation fails `not_found` once the process has
     * ended. Ending a process is allowed while the session is locked.
     */
    kill(signal?: ProcessSignal): Operation<null>;
    /** Records the end the host reported and settles `wait()`. Host-only. */
    end(exit: ProcessExit): void;
}
/** `gnoblin.process`: the programs this generation started, and the listeners told when each ends. */
export declare class ProcessTable {
    #private;
    /** `host` owns the children and the clock; the table only keeps the handles and the listeners. */
    constructor(host: ProcessHost);
    /**
     * Starts a program and returns its handle at once. A string is a command line for `/bin/sh`; an array is the
     * program and its arguments. Refuses (throws) when the program cannot start, naming why.
     */
    spawn(command: string | readonly string[], options?: SpawnOptions): Process;
    /** The processes this generation started that have not ended, oldest first. */
    list(): Process[];
    /** The running process with this id, or null: an ended process is known only to whoever holds its handle. */
    byId(id: number): Process | null;
    /** Adds a listener once. Throws a `TypeError` when `listener` is not an object. */
    addListener(listener: ProcessListener): void;
    /** Removes a listener. A listener that is not there is ignored. */
    removeListener(listener: ProcessListener): void;
    /** The end of a process, as the host reports it. A process of another generation is not ours and is ignored. */
    deliver(event: ExitEvent): void;
    /** The operating system's process id of `process`, 0 while its program has not started. Only a `Process` calls this. */
    pidOf(process: Process): number;
    /** The operation behind `Process.wait`. Only a `Process` calls this. */
    waitOperation(process: Process): Operation<ProcessExit>;
    /** Sends `signal` to the process group. The host's answer decides: `not_found` once the process has ended. */
    killOperation(process: Process, signal: ProcessSignal): Operation<null>;
}
