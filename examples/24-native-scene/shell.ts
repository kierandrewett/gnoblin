// Example 24 in TypeScript: the scene of libgnoblin-std, through the classes generated from its catalog.
//
//     gnoblin ./shell.ts
//
// Makes a perspective camera and a view drawn through it, mounts both under the scene's root and logs the placement it reads back. Every
// line starts with `scene:`, as the Rust shell of the same example spells it.
import { Camera3D, Scene, View } from "gnoblin-std/generated/objects";
import type {} from "gnoblin-std";

const scene = Scene.get();
const camera = new Camera3D({ id: "cam", projection: "perspective", fov: 70, position: { x: 1, y: 2, z: 3 } });
scene.add(camera);
const view = new View({
    id: "panel",
    size: { width: 400, height: 300 },
    position: { x: 0.5, y: 0.2, z: -1.4 },
    camera,
});
scene.add(view);
const at = view.position;
console.log(
    `scene: the view ${view.id} is node ${view.node} at ${at.x} ${at.y} ${at.z}, the camera ${camera.id} is node ${camera.node} with fov ${camera.fov}`,
);
