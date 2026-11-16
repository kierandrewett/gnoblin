// What the types must refuse. Each `// expect:` line names the error the line after it must produce, and
// scripts/js-types-check.sh fails when a line has a different one, none, or when another line has any.
import { Client } from "gnoblin-core";
import { Workspace } from "gnoblin-std";
import { Camera3D, Node } from "gnoblin-std/scene";

declare function focusClient(client: Client): void;
declare const workspace: Workspace;
declare const node: Node;

// expect: TS2322
node.opacity = "half";

// expect: TS2339
node.explode();

// A workspace is not a client.
// expect: TS2345|TS2740
focusClient(workspace);

// A plain node is not a camera.
// expect: TS2740
export const camera: Camera3D = node;

// expect: TS2322
export const projected = new Camera3D({ projection: "isometric" });
