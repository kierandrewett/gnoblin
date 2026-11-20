/**
 * A generation that frames no window gives back the room an earlier generation's frames took, which the library still holds when a
 * reload stays in one core generation (a runtime reload opens a new one, whose library holds none).
 */
export declare function clearInheritedFrames(): void;
/**
 * Runs once a frame, after the listeners. The layout itself is not run here: it is the library's turn, which follows the
 * engine's frame, and it writes the placements as tree writes of its own.
 */
export declare function layout(): number;
