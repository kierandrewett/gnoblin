// Intercepts host reads, writes and calls while a transition callback is running.

import { GnoblinError } from "gnoblin-core/errors";
import { host, native, nodeName } from "gnoblin-core/host/bridge";
import { std } from "gnoblin-std/host/state";
import { noteHostRead, noteHostWrite } from "gnoblin-core/properties";
import { Staging } from "gnoblin-std/transition/staging";
import type { HostValue } from "gnoblin-core/types";

/**
 * Installs transition staging around host reads, writes and calls. Called once, before anything uses `host`. It wraps
 * the functions as the shell handed them over (`native`) and replaces the read and write tracking core installed with
 * its own, which does the same and stages in between.
 */
export function installStaging(): void {
    // The guards below replace the host's functions and call the natives when nothing is staged.
    const staging = new Staging(
        { get: native.get, set: native.set, animate: native.animate, check: native.check },
        (code, target, message) => new GnoblinError(code, target, message),
    );

    const nativeCall = native.call;

    const nativeRaise = native.raise;

    const nativeLower = native.lower;

    const nativeApi = native.api;

    host.get = (node, key) => {
        const value = staging.get(node, key) as HostValue | undefined;
        noteHostRead(node, key, value);
        return value;
    };

    host.set = (node, key, value) => {
        const refused = staging.set(node, key, value);
        if (refused == null) {
            noteHostWrite(node, key, value);
        }
        return refused;
    };

    host.call = (node, method) => {
        staging.forbid(`${nodeName(node)}.${method}()`);
        return nativeCall(node, method);
    };

    host.raise = (node) => {
        staging.forbid(`${nodeName(node)}.raise()`);
        return nativeRaise(node);
    };

    host.lower = (node) => {
        staging.forbid(`${nodeName(node)}.lower()`);
        return nativeLower(node);
    };

    host.api.spawn = (argv, options) => {
        staging.forbid("gnoblin.spawn()");
        return nativeApi.spawn(argv, options);
    };

    host.api.kill = (id, signal) => {
        staging.forbid("Process.kill()");
        return nativeApi.kill(id, signal);
    };

    host.api.begin = (verb) => {
        staging.forbid(`gnoblin.session.${verb}()`);
        return nativeApi.begin(verb);
    };
    std.staging = staging;
}
