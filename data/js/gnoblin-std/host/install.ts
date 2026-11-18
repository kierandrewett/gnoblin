// Installs `gnoblin-std` on the object model `gnoblin-core` built: the staging in front of the host's functions, the
// roots and services of the desktop, the hooks of the object model and the members std adds to its classes. It runs when
// `gnoblin-std/host` is imported, which is when a config imports `gnoblin-std`; a shell that only imports `gnoblin-core`
// never runs it.
//
// The core host has always booted by then (the shell loads `gnoblin-core/host` before any config), and has adopted the
// compositor's outputs and windows. What std does for an object as it is adopted it does for the outputs that were
// already there at the end of this install, in the order core adopted them.

import { host } from "gnoblin-core/host/bridge";
import { ctx } from "gnoblin-core/host/context";
import { hooks, installFeature } from "gnoblin-core/host/hooks";
import type { Output } from "gnoblin-core/host/output";
import { installDisplays } from "gnoblin-std/host/displays";
import { registerInspectors } from "gnoblin-std/host/console";
import { lifecycle } from "gnoblin-std/host/lifecycle";
import { extendObjectModel, registerQueryKeys } from "gnoblin-std/host/members";
import { openLibrary } from "gnoblin-std/host/library";
import { syncMembership } from "gnoblin-std/host/membership";
import { installFocus, installPolicy } from "gnoblin-std/host/policy";
import { installTransitions } from "gnoblin-std/host/rollback";
import { installRoots } from "gnoblin-std/host/roots";
import { installSnap } from "gnoblin-std/host/snap_host";
import { installServices } from "gnoblin-std/host/services";
import { installStaging } from "gnoblin-std/host/staging";
import { std } from "gnoblin-std/host/state";
import { installScene } from "gnoblin-std/host/xr_bridge";
import { attachUi } from "gnoblin-std/ui/state";

/** Attaches the UI toolkit to the shell's UI services and its clock. */
function installUi(): void {
    attachUi({ ...host.ui, now: () => host.input.now() }, (site, error) => {
        ctx.reports.push({ site, error });
    });
}

/** Installs the package on the host. */
function installStd(): void {
    // The library is the desktop: its workspaces, desks and layout pass start here, and every shell that imports this package has them.
    openLibrary();
    registerQueryKeys();
    installStaging();
    installRoots();
    installSnap();
    installDisplays();
    installUi();
    installPolicy();
    installServices();
    installScene();
    installTransitions();
    installFeature("gnoblin-std", lifecycle);
    extendObjectModel();
    installFocus();
    registerInspectors();
    for (const output of ctx.outputRoot.list as Output[]) {
        hooks.outputAdded(output);
    }
    // A package imported after the shell opened (frames ran before the entry) gets what the opening gives it: the stock
    // workspace and a workspace for each window that has none.
    if (ctx.flags.opened) {
        hooks.finishLoad();
        hooks.opened();
    }
    std.displayManager.reflow();
    // The frames the host ran before this import are not lost: the XR program's session is read from the tree by a
    // frame, and a config that imports std late still finds `gnoblin.extensions.xr` there.
    try {
        std.xrRuntime.frame();
    } catch (error) {
        ctx.reports.push({ site: "XRRuntime.frame", error });
    }
}

installStd();
// A library that outlives this generation (a reload that stays in one core generation) holds workspaces already: they are in the list.
syncMembership();
