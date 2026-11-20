import type { SceneBackend } from "gnoblin-std/scene/backend";
/**
 * The scene's backend: what a script makes is the library's, everything else is the engine's. `engine` answers for the nodes the
 * library did not make and for what only the script's objects know (the clients and workspaces a View shows).
 */
export declare function nativeScene(engine: SceneBackend): SceneBackend;
