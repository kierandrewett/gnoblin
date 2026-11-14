// `gnoblin.process`: the programs a script starts, and how each one ends.
//
//     import { ProcessListener } from "gnoblin";
//
//     const job = gnoblin.process.spawn(["date", "+%H:%M"], { stdout: "capture" });
//     job.wait().onComplete((exit) => console.log(exit?.stdout));
//
//     class Watch extends ProcessListener {
//         onExit(process, code) {
//             console.log(process.argv.join(" "), "ended with", code);
//         }
//     }
//     gnoblin.process.addListener(new Watch());
//
// The shell's host owns the children (nothing else can reap them), so a `Process` is a handle: `alive`, `exitCode`
// and the rest change as the host reports, once a frame, and the listeners run as queued calls under the usual
// budget. A string is one command line for `/bin/sh`; an array is the program, found on `PATH`, and its arguments.
// Each program starts in a process group of its own, so `kill` and `timeout` reach the programs it starts too.
//
// The module is plain TypeScript over the host's functions (`ProcessHost`), so it runs without a shell.

import { Operation } from "gnoblin/operation";
import type { OperationHost } from "gnoblin/operation";

/** Where one standard stream goes: the session log, nowhere, or into the end event (up to `outputLimit` bytes). */
export type StreamMode = "inherit" | "discard" | "capture";

/** The signals `Process.kill` sends, to the process group. */
export type ProcessSignal = "term" | "kill" | "int" | "hup" | "usr1" | "usr2" | "stop" | "cont";

export interface SpawnOptions {
    /** The working directory: an absolute path. The runtime's own by default. */
    cwd?: string;
    /** Variables added to the session environment, at most 128. */
    env?: Record<string, string>;
    /** Text written to standard input, at most 1 MiB, which is then closed. Standard input is empty by default. */
    stdin?: string;
    stdout?: StreamMode;
    stderr?: StreamMode;
    /** Bytes kept from each captured stream, 0 to 4194304. 65536 by default. */
    outputLimit?: number;
    /** Seconds before the group gets SIGTERM, and SIGKILL two seconds later. More than 0, at most 86400. */
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
    readonly stderr: string | null;
    /** Standard output passed `outputLimit` and the rest was dropped. */
    readonly stdoutTruncated: boolean;
    readonly stderrTruncated: boolean;
}

/** Observes the processes the script started. Every method is optional. */
export class ProcessListener {
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
    /** A fresh operation id. */
    operationId(): number;
    /** The error a failed call throws: `code` says why, `target` names the call. */
    error(code: string, target: string, message: string): Error;
}

const STREAMS: readonly string[] = ["inherit", "discard", "capture"];
const SIGNALS: readonly string[] = ["term", "kill", "int", "hup", "usr1", "usr2", "stop", "cont"];

/** The event the host reports when a process ends. */
export interface ExitEvent extends ProcessExit {
    kind: "exit";
    id: number;
}

/**
 * One started program. `alive` is true from the spawn until its end is reported, and the fields that say how it
 * ended are `null` until then.
 */
export class Process {
    readonly id: number;
    readonly pid: number;
    readonly argv: readonly string[];
    #table: ProcessTable;
    #exit: ProcessExit | null = null;
    #waiting: Operation<ProcessExit> | null = null;

    /** Only the table makes processes: a script receives them from `spawn`, `list` and `byId`. */
    constructor(table: ProcessTable, id: number, pid: number, argv: readonly string[]) {
        this.#table = table;
        this.id = id;
        this.pid = pid;
        this.argv = argv;
    }

    get alive(): boolean {
        return this.#exit === null;
    }

    get status(): "running" | "exited" {
        return this.#exit === null ? "running" : "exited";
    }

    get exitCode(): number | null {
        return this.#exit?.exitCode ?? null;
    }

    get signal(): string | null {
        return this.#exit?.signal ?? null;
    }

    get timedOut(): boolean {
        return this.#exit?.timedOut ?? false;
    }

    /** What the process ended with, or null while it runs. */
    get exit(): ProcessExit | null {
        return this.#exit;
    }

    /**
     * An operation that succeeds when the process ends, whatever its exit status, with how it ended. Every call
     * returns the same operation.
     */
    wait(): Operation<ProcessExit> {
        if (this.#waiting === null) {
            this.#waiting = this.#table.waitOperation(this);
            if (this.#exit !== null) {
                this.#waiting.succeed(this.#exit);
            }
        }
        return this.#waiting;
    }

    /**
     * Sends a signal to the process group (`term` by default). The operation fails `not_found` once the process has
     * ended. Ending a process is allowed while the session is locked.
     */
    kill(signal: ProcessSignal = "term"): Operation<null> {
        return this.#table.killOperation(this, signal);
    }

    /** Records the end the host reported and settles `wait()`. Host-only. */
    end(exit: ProcessExit): void {
        if (this.#exit === null) {
            this.#exit = exit;
            this.#waiting?.succeed(exit);
        }
    }
}

function parse(text: string): Record<string, unknown> {
    return JSON.parse(text) as Record<string, unknown>;
}

/** The options of a spawn as JSON, or the TypeError that names the one that is wrong. */
function checkOptions(options: SpawnOptions | undefined): string {
    const given = options ?? {};
    if (typeof given !== "object" || given === null || Array.isArray(given)) {
        throw new TypeError("gnoblin.process.spawn: options must be an object");
    }
    const known = ["cwd", "env", "stdin", "stdout", "stderr", "outputLimit", "timeout"];
    const unknown = Object.keys(given).find((key) => !known.includes(key));
    if (unknown !== undefined) {
        throw new TypeError(`gnoblin.process.spawn: no option named ${unknown}; the options are ${known.join(", ")}`);
    }
    if (given.cwd !== undefined && typeof given.cwd !== "string") {
        throw new TypeError("gnoblin.process.spawn: cwd must be a path string");
    }
    if (given.stdin !== undefined && typeof given.stdin !== "string") {
        throw new TypeError("gnoblin.process.spawn: stdin must be a string");
    }
    for (const stream of ["stdout", "stderr"] as const) {
        const mode = given[stream];
        if (mode !== undefined && !STREAMS.includes(mode)) {
            throw new TypeError(`gnoblin.process.spawn: ${stream} must be ${STREAMS.join(", ")}`);
        }
    }
    if (given.env !== undefined) {
        const env = given.env as unknown;
        if (
            typeof env !== "object" ||
            env === null ||
            Array.isArray(env) ||
            Object.values(env).some((value) => typeof value !== "string")
        ) {
            throw new TypeError("gnoblin.process.spawn: env must be an object of strings");
        }
    }
    for (const field of ["outputLimit", "timeout"] as const) {
        const value = given[field];
        if (value !== undefined && (typeof value !== "number" || !Number.isFinite(value))) {
            throw new TypeError(`gnoblin.process.spawn: ${field} must be a finite number`);
        }
    }
    return JSON.stringify(given);
}

/** `gnoblin.process`: the programs this generation started, and the listeners told when each ends. */
export class ProcessTable {
    #host: ProcessHost;
    #running = new Map<number, Process>();
    #listeners: ProcessListener[] = [];

    constructor(host: ProcessHost) {
        this.#host = host;
    }

    /**
     * Starts a program and returns its handle at once. A string is a command line for `/bin/sh`; an array is the
     * program and its arguments. Refuses (throws) when the program cannot start, naming why.
     */
    spawn(command: string | readonly string[], options?: SpawnOptions): Process {
        const valid =
            typeof command === "string"
                ? command.length > 0
                : Array.isArray(command) && command.length > 0 && command.every((part) => typeof part === "string");
        if (!valid) {
            throw new TypeError("gnoblin.spawn: expects a command string or a non-empty array of strings");
        }
        const argv = typeof command === "string" ? ["/bin/sh", "-c", command] : [...command];
        const answer = parse(this.#host.spawn(JSON.stringify(argv), checkOptions(options)));
        const error = answer.error as { code: string; message: string } | undefined;
        if (error !== undefined) {
            if (error.code === "invalid_argument") {
                throw new TypeError(`gnoblin.spawn: ${error.message}`);
            }
            throw this.#host.error("spawn-failed", "gnoblin.spawn", error.message);
        }
        const process = new Process(this, answer.id as number, answer.pid as number, argv);
        this.#running.set(process.id, process);
        return process;
    }

    /** The processes this generation started that have not ended, oldest first. */
    list(): Process[] {
        return [...this.#running.values()];
    }

    /** The running process with this id, or null: an ended process is known only to whoever holds its handle. */
    byId(id: number): Process | null {
        return this.#running.get(id) ?? null;
    }

    addListener(listener: ProcessListener): void {
        if (listener === null || typeof listener !== "object") {
            throw new TypeError("gnoblin.process.addListener: expects a ProcessListener");
        }
        if (!this.#listeners.includes(listener)) {
            this.#listeners.push(listener);
        }
    }

    removeListener(listener: ProcessListener): void {
        const index = this.#listeners.indexOf(listener);
        if (index >= 0) {
            this.#listeners.splice(index, 1);
        }
    }

    /** The end of a process, as the host reports it. A process of another generation is not ours and is ignored. */
    deliver(event: ExitEvent): void {
        const process = this.#running.get(event.id);
        if (process === undefined) {
            return;
        }
        this.#running.delete(event.id);
        const { kind: _kind, id: _id, ...exit } = event;
        process.end(exit);
        for (const listener of [...this.#listeners]) {
            if (typeof listener.onExit === "function") {
                this.#host.schedule(`${describe(listener)}.onExit`, () => {
                    if (this.#listeners.includes(listener)) {
                        listener.onExit?.(process, process.exitCode);
                    }
                });
            }
        }
    }

    waitOperation(process: Process): Operation<ProcessExit> {
        return new Operation<ProcessExit>(this.#host, this.#host.operationId(), `process.wait#${process.id}`);
    }

    killOperation(process: Process, signal: ProcessSignal): Operation<null> {
        if (typeof signal !== "string" || !SIGNALS.includes(signal)) {
            throw new TypeError(`process.kill: the signal is one of ${SIGNALS.join(", ")}`);
        }
        const operation = new Operation<null>(this.#host, this.#host.operationId(), `process.kill#${process.id}`);
        const answer = this.#host.kill(process.id, signal);
        if (answer === undefined) {
            operation.succeed(null);
        } else {
            const { error } = parse(answer) as { error: { code: string; message: string } };
            operation.fail(error.code as "not_found", error.message);
        }
        return operation;
    }
}

function describe(listener: object): string {
    const name: string | undefined = listener.constructor?.name;
    return name !== undefined && name !== "" ? name : "listener";
}
