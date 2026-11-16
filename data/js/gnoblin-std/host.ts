// The runtime objects of `gnoblin-std`, as a script imports them: `import { Workspace } from "gnoblin-std"` reaches
// these, because the shell appends `export * from "gnoblin-std/host"` to the package entry. The core object model is
// built first (`gnoblin-core/host`); importing this module is what installs the desktop policy on it (`./host/install`).

import "gnoblin-core/host";
import "gnoblin-std/host/install";

export { Workspace } from "gnoblin-std/host/workspace";
export { SLIDE_DURATION } from "gnoblin-std/host/show";
export type { ShowOptions } from "gnoblin-std/host/show";
export { startDecorations, startNotRespondingAlert, startWindowMenu } from "gnoblin-std/host/services";
export type { Easing } from "gnoblin-std/transition/easing";
