// The runtime objects of `gnoblin-std`, as a script imports them: `import { Workspace } from "gnoblin-std"` reaches
// these, because the shell appends `export * from "gnoblin-std/host"` to the package entry. The core object model is
// built first (`gnoblin-core/host`); this module is what adds the desktop policy to it.

import "gnoblin-core/host";

export { Workspace } from "gnoblin-std/host/workspace";
export { SLIDE_DURATION } from "gnoblin-std/host/show";
export type { ShowOptions } from "gnoblin-std/host/show";
export { startDecorations, startNotRespondingAlert, startWindowMenu } from "gnoblin-core/host/services";
export type { Easing } from "gnoblin-std/transition/easing";
