// A script that builds a scene: `Camera3D` is a `Node` of the one scene module, and the XR camera is a `Camera3D`.
import { XRCamera3D } from "gnoblin-std/extensions/xr";
import { Camera3D, Node, View } from "gnoblin-std/scene";

export const camera = new Camera3D({ fov: 70, near: 0.1, far: 100 });
export const cameraAsNode: Node = camera;
export const head: Camera3D = new XRCamera3D({ near: 0.05, far: 100 });
export const headAsNode: Node = head;
export const root = new Node({ name: "root" });
root.add(camera).opacity = 0.5;
export const view = new View({ camera, width: 640, height: 360 });
export const nodes: Node[] = [camera, head, view, root];
export const parent: Node | null = nodes[0]?.parent ?? null;
