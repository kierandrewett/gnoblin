/** Three numbers: a position, the angles about X, Y and Z in radians, or the scale along each axis. */
export type Triple = readonly [number, number, number];
/** A triple that also answers to `x`, `y` and `z`, so `node.rotation[1]` and `node.rotation.y` read the same turn. */
export type AxisTriple = [number, number, number] & {
    readonly x: number;
    readonly y: number;
    readonly z: number;
};
/** A point or a vector as an object. */
export interface Vec3Like {
    /** Horizontal component. */
    x: number;
    /** Vertical component. */
    y: number;
    /** Depth component. */
    z: number;
}
/** A rotation as a unit quaternion. */
export interface QuaternionLike extends Vec3Like {
    /** Scalar component of the quaternion. */
    w: number;
}
/** What a script may give as a rotation: radians about X, Y and Z as a list or an object, or a quaternion. */
export type RotationInit = Triple | Vec3Like | QuaternionLike;
/** What a script may give as a scale: one number for every axis, or a value per axis. */
export type ScaleInit = number | Triple | Vec3Like;
/** What a script may give as a transform: each part is optional, and a part left out is not written. */
export interface TransformInit {
    /** Position in logical pixels on flat layouts. */
    position?: Triple | Vec3Like;
    /** Rotation in radians, as Euler angles or a quaternion. */
    rotation?: RotationInit;
    /** Uniform scale or scale along each axis. */
    scale?: ScaleInit;
}
/** A transform with each part turned into the three numbers the tree stores, or null where the script gave none. */
export interface ParsedTransform {
    /** Parsed position, or null when omitted. */
    position: [number, number, number] | null;
    /** Parsed rotation in radians, or null when omitted. */
    rotation: [number, number, number] | null;
    /** Parsed scale, or null when omitted. */
    scale: [number, number, number] | null;
}
/**
 * Returns `parts` as a frozen copy that also has `x`, `y` and `z`. The properties are not enumerable and the copy
 * is still an array, so code that indexes it and code that names the axis both work.
 */
export declare function withAxes(parts: readonly number[]): AxisTriple;
/** Three finite numbers from a list of three or an `{ x, y, z }` object. Throws a TypeError naming `target`. */
export declare function vectorFrom(target: string, value: unknown): [number, number, number];
/**
 * The angles about X, Y and Z, in radians, of a unit quaternion, for the order the tree uses (turn about X, then Y,
 * then Z: the matrix is `Rz * Ry * Rx`). A quaternion that is not of unit length is normalised first.
 */
export declare function eulerOfQuaternion(q: QuaternionLike): [number, number, number];
/** Three angles in radians from a list, an `{ x, y, z }` Euler object or a `{ x, y, z, w }` quaternion. */
export declare function rotationFrom(target: string, value: unknown): [number, number, number];
/** The scale along each axis from one number (every axis), a list of three or an `{ x, y, z }` object. */
export declare function scaleFrom(target: string, value: unknown): [number, number, number];
/**
 * Reads a transform: a plain `{ position, rotation, scale }` object or an object with those members (a `Transform3D`).
 * A part that is missing or undefined stays null. Throws a TypeError naming `target` for a part that is not valid or a
 * key that is not one of the three.
 */
export declare function transformFrom(target: string, value: unknown): ParsedTransform;
