// The composition root. Importing this module builds one generation of the object model: it puts the staging layer
// in front of the host, makes the root collections and the services in the order they depend on each other,
// makes the `gnoblin` object, adopts the nodes that are already in the tree and hands the shell its entry. Nothing
// else in `gnoblin/host/*` runs anything when it is imported.

import { adopt } from "gnoblin/host/adoption";
import { host } from "gnoblin/host/bridge";
import { registerInspectors } from "gnoblin/host/console";
import { ctx } from "gnoblin/host/context";
import { installDisplays } from "gnoblin/host/displays";
import { makeEntry } from "gnoblin/host/entry";
import { Gnoblin } from "gnoblin/host/gnoblin";
import { installInput } from "gnoblin/host/input_bridge";
import { installFocus, installPolicy } from "gnoblin/host/policy";
import { installTransitions } from "gnoblin/host/rollback";
import { installRoots } from "gnoblin/host/roots";
import { installServices, trackLaunches } from "gnoblin/host/services";
import { installHostApi } from "gnoblin/host/session_bridge";
import { installStaging } from "gnoblin/host/staging";
import { installScene } from "gnoblin/host/xr_bridge";
import { inspect } from "gnoblin/inspect";

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

host.entry = makeEntry(gnoblin);
