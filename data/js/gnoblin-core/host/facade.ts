// Installs the host services and public object model for one generation.
// It creates root collections, adopts existing nodes, and gives the shell its frame entry. This is everything a shell
// that imports only `gnoblin-core` gets; a package built on it (`gnoblin-std`) installs itself on top when imported.

import { adoptExisting } from "gnoblin-core/host/adoption";
import { host, installTracking } from "gnoblin-core/host/bridge";
import { registerInspectors } from "gnoblin-core/host/console";
import { makeEntry } from "gnoblin-core/host/entry";
import { Gnoblin } from "gnoblin-core/host/gnoblin";
import { installInput } from "gnoblin-core/host/input_bridge";
import { installRoots } from "gnoblin-core/host/roots";
import { installServices, trackLaunches } from "gnoblin-core/host/services";
import { installHostApi } from "gnoblin-core/host/session_bridge";
import { inspect } from "gnoblin-core/inspect";

installTracking();
installRoots();
installInput();
installServices();
installHostApi();

/** The object model installed as `globalThis.gnoblin` for this generation. */
export const gnoblin = new Gnoblin();

trackLaunches();

registerInspectors();

adoptExisting();

Object.defineProperty(globalThis, "gnoblin", { value: gnoblin, enumerable: false });

// The console's helpers: `inspect.clients()` and the rest are typed at a console prompt without an import. Unlike
// `gnoblin` the name is a convenience and not part of the object model, so a script may replace it.
Object.defineProperty(globalThis, "inspect", { value: inspect, enumerable: false, writable: true, configurable: true });

host.entry = makeEntry();
