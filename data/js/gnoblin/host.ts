// The object model of the `gnoblin` global, as a script imports it: `import { Workspace } from "gnoblin"`
// reaches these, because the shell appends `export * from "gnoblin/host"` to the stock index. Importing
// this module builds the generation: `host/facade` installs the layers and the entry the shell calls.

export { gnoblin } from "gnoblin/host/facade";
export { GnoblinError } from "gnoblin/errors";
export { ClientListener, CollectionListener, OutputListener } from "gnoblin/host/listeners";
export type { ClientPointerEvent } from "gnoblin/host/listeners";
export { Client } from "gnoblin/host/client";
export type { AnimateOptions, ClientAnimation } from "gnoblin/host/client";
export { Collection, MutableCollection } from "gnoblin/host/collections";
export { Output } from "gnoblin/host/output";
export type { OutputApplication } from "gnoblin/host/output_apply";
export { Workspace } from "gnoblin/host/workspace";
export { SLIDE_DURATION } from "gnoblin/host/show";
export type { ShowOptions } from "gnoblin/host/show";
export { startDecorations, startNotRespondingAlert, startWindowMenu } from "gnoblin/host/services";
export type { Easing } from "gnoblin/transition/easing";
export type { Query } from "gnoblin/types";
