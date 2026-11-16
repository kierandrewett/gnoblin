// The runtime objects of `gnoblin-core`, as a script imports them: `import { Client, gnoblin } from "gnoblin-core"`
// reaches these, because the shell appends `export * from "gnoblin-core/host"` to the package entry. Importing this
// module builds the generation: `host/facade` installs the layers and the entry the shell calls.

export { gnoblin } from "gnoblin-core/host/facade";
export { GnoblinError } from "gnoblin-core/errors";
export { ClientListener, CollectionListener, OutputListener } from "gnoblin-core/host/listeners";
export type { ClientPointerEvent } from "gnoblin-core/host/listeners";
export { Client } from "gnoblin-core/host/client";
export type { FramePolicy } from "gnoblin-core/host/client";
export { Collection, MutableCollection } from "gnoblin-core/host/collections";
export { Output } from "gnoblin-core/host/output";
