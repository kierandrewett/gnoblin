// Installs the host services and public object model for one generation.
// It creates root collections, adopts existing nodes, and gives the shell its frame entry.

import { adopt } from "gnoblin-core/host/adoption";
import { host } from "gnoblin-core/host/bridge";
import { registerInspectors } from "gnoblin-core/host/console";
import { ctx } from "gnoblin-core/host/context";
import { installDisplays } from "gnoblin-std/host/displays";
import { makeEntry } from "gnoblin-core/host/entry";
import { Gnoblin } from "gnoblin-core/host/gnoblin";
import { installInput } from "gnoblin-core/host/input_bridge";
import { installFocus, installPolicy } from "gnoblin-std/host/policy";
import { installTransitions } from "gnoblin-std/host/rollback";
import { installRoots } from "gnoblin-core/host/roots";
import { installServices, trackLaunches } from "gnoblin-core/host/services";
import { installHostApi } from "gnoblin-core/host/session_bridge";
import { installStaging } from "gnoblin-std/host/staging";
import { installScene } from "gnoblin-std/host/xr_bridge";
import { inspect } from "gnoblin-core/inspect";

installStaging();
installRoots();
installDisplays();
installInput();
installPolicy();
installServices();
installScene();
installHostApi();
installTransitions();

/** The object model installed as `globalThis.gnoblin` for this generation. */
export const gnoblin = new Gnoblin();

trackLaunches();

installFocus();

registerInspectors();

for (const node of host.screens()) {
    adopt(node, false);
}

for (const node of host.windows()) {
    adopt(node, false);
}

ctx.displayManager.reflow();

Object.defineProperty(globalThis, "gnoblin", { value: gnoblin, enumerable: false });

// The console's helpers: `inspect.clients()` and the rest are typed at a console prompt without an import. Unlike
// `gnoblin` the name is a convenience and not part of the object model, so a script may replace it.
Object.defineProperty(globalThis, "inspect", { value: inspect, enumerable: false, writable: true, configurable: true });

host.entry = makeEntry();
