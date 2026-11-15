/**
 * The runtime object, also reachable as `import { gnoblin } from "gnoblin"`. Its type is the real class of the library
 * (`gnoblin/host/gnoblin`), so a member is declared where it is implemented.
 */
declare const gnoblin: import("gnoblin/host/gnoblin").Gnoblin;

/**
 * The shell's log, as in Node. A call writes one line to the log (component `js`, area `script`; `log` and `info` at
 * info level, `warn`, `error` and `debug` at theirs) that starts with the call site as `file.ts:line:col`, so
 * `gnoblinctl logs` shows it and `gnoblinctl console` prints what an expression printed. Strings print as they are,
 * `%s %d %i %f %j %o %O %c %%` are format specifiers, an object prints as one line of JSON with `[Circular]` where it
 * loops, and an error prints its stack. A call is cut at 4 KiB and a generation logs at most 200 lines a second: the
 * ones past it are dropped, and a notice says how many.
 */
declare const console: {
    log(...data: unknown[]): void;
    info(...data: unknown[]): void;
    warn(...data: unknown[]): void;
    error(...data: unknown[]): void;
    debug(...data: unknown[]): void;
    /** Logs at error level, starting with "Assertion failed", when `condition` is falsy; never throws. */
    assert(condition?: unknown, ...data: unknown[]): void;
    /** Logs `data` as one line of JSON; it does not draw a table. */
    table(data: unknown): void;
};
