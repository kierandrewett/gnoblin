// Deferred listener calls and the failures they leave. Listeners never run inside the code that raised the event:
// the call is queued here and the shell drains the queue frame by frame under a budget for each call, so one
// failing or slow listener cannot stop the frame. A failure is collected as a report and handed to the shell.

import { ctx } from "gnoblin/host/context";
import type { ErrorReport } from "gnoblin/host/contract";

/** A listener source and its attached handlers. */
export interface Source {
    listeners: object[];
}

/** A call the module itself queues (an input hook), run beside the listeners' under the same budget. */
export interface Task {
    site: string;
    task(): void;
}

/** One deferred listener invocation, including its source and call arguments. */
export interface Invocation {
    source: Source;
    listener: object;
    method: string;
    args: unknown[];
}

/** Reads the current handler property from a listener. */
export function handlerOf(listener: object, method: string): unknown {
    return Reflect.get(listener, method);
}

/** Names a listener constructor for error reports, falling back to `listener`. */
export function describeListener(listener: object): string {
    const name = Reflect.get(listener, "constructor")?.name;
    return typeof name === "string" && name ? name : "listener";
}

/** Runs one queued call if its listener is still attached; a throw is reported, never propagated. */
export function run(call: Invocation | Task): void {
    if ("task" in call) {
        try {
            call.task();
        } catch (error) {
            ctx.reports.push({ site: call.site, error });
        }
        return;
    }
    if (!call.source.listeners.includes(call.listener)) {
        return;
    }
    const handler = handlerOf(call.listener, call.method);
    if (typeof handler !== "function") {
        return;
    }
    try {
        handler.apply(call.listener, call.args);
    } catch (error) {
        ctx.reports.push({ site: `${describeListener(call.listener)}.${call.method}`, error });
    }
}

/** Queues each listener currently implementing `method` on a source. */
export function queueFor(source: Source, method: string, args: unknown[]): void {
    for (const listener of source.listeners) {
        if (typeof handlerOf(listener, method) === "function") {
            ctx.queue.push({ source, listener, method, args });
        }
    }
}

/** Runs a host callback while collecting its error under `site` instead of aborting the frame. */
export function guard(site: string, body: () => void): void {
    try {
        body();
    } catch (error) {
        ctx.reports.push({ site, error });
    }
}

/** Deduplicates a notice so one unsupported style is reported only once per generation. */
export function say(text: string): void {
    if (!ctx.noticed.has(text)) {
        ctx.noticed.add(text);
        ctx.notices.push(text);
    }
}

/** Number of deferred callbacks still waiting for the shell pump. */
export function pending(): number {
    return ctx.queue.length;
}

/** Runs one deferred callback and returns the remaining queue length. */
export function next(): number {
    const call = ctx.queue.shift();
    if (call) {
        run(call);
    }
    return ctx.queue.length;
}

/** Transfers collected callback failures to the shell and starts an empty error batch. */
export function takeErrors(): ErrorReport[] {
    const taken = ctx.reports;
    ctx.reports = [];
    return taken;
}
