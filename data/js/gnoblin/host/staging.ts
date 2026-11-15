// Intercepts host reads, writes and calls while a transition callback is running.

import { GnoblinError } from "gnoblin/errors";
import { host, nodeName } from "gnoblin/host/bridge";
import { ctx } from "gnoblin/host/context";
import { noteHostRead, noteHostWrite } from "gnoblin/properties";
import { Staging } from "gnoblin/transition/staging";
import type { HostValue } from "gnoblin/types";

/** Installs transition staging around host reads, writes and calls. Called once, before anything uses `host`. */
export function installStaging(): void {
    // The natives are captured first: the guards below replace the host's functions and call these when
    // nothing is staged.
    const staging = new Staging(
        { get: host.get, set: host.set, animate: host.animate, check: host.check },
        (code, target, message) => new GnoblinError(code, target, message),
    );

    const nativeCall = host.call;

    const nativeRaise = host.raise;

    const nativeLower = host.lower;

    const nativeApi = { spawn: host.api.spawn, kill: host.api.kill, begin: host.api.begin };

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

    host.animate = (node, key, value, duration, samples) => staging.animate(node, key, value, duration, samples);

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
    ctx.staging = staging;
}
