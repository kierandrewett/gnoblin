// Spatial value parsing: the shapes the specification gives a position, a rotation, a scale and a transform
// (`{ x, y, z }` objects, a quaternion, a bare number) reduced to the three-number lists the tree stores.
// This module owns the shapes and their errors only; the nodes that store them are `gnoblin/scene` (script nodes) and
// the host's `Client`. It reads no host state, so it runs anywhere.

/** Three numbers: a position, the angles about X, Y and Z in radians, or the scale along each axis. */
export type Triple = readonly [number, number, number];

/** A triple that also answers to `x`, `y` and `z`, so `node.rotation[1]` and `node.rotation.y` read the same turn. */
export type AxisTriple = [number, number, number] & { readonly x: number; readonly y: number; readonly z: number };

/** A point or a vector as an object. */
export interface Vec3Like {
    x: number;
    y: number;
    z: number;
}

/** A rotation as a unit quaternion. */
export interface QuaternionLike extends Vec3Like {
    w: number;
}

/** What a script may give as a rotation: radians about X, Y and Z as a list or an object, or a quaternion. */
export type RotationInit = Triple | Vec3Like | QuaternionLike;

/** What a script may give as a scale: one number for every axis, or a value per axis. */
export type ScaleInit = number | Triple | Vec3Like;

/** What a script may give as a transform: each part is optional, and a part left out is not written. */
export interface TransformInit {
    position?: Triple | Vec3Like;
    rotation?: RotationInit;
    scale?: ScaleInit;
}

/** A transform with each part turned into the three numbers the tree stores, or null where the script gave none. */
export interface ParsedTransform {
    position: [number, number, number] | null;
    rotation: [number, number, number] | null;
    scale: [number, number, number] | null;
}

/**
 * Returns `parts` as a frozen copy that also has `x`, `y` and `z`. The properties are not enumerable and the copy
 * is still an array, so code that indexes it and code that names the axis both work.
 */
export function withAxes(parts: readonly number[]): AxisTriple {
    const copy = [parts[0], parts[1], parts[2]];
    Object.defineProperties(copy, {
        x: { get: () => copy[0] },
        y: { get: () => copy[1] },
        z: { get: () => copy[2] },
    });
    return Object.freeze(copy) as unknown as AxisTriple;
}

function finite(target: string, value: unknown, what: string): number {
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new TypeError(`${target}: ${what} must be a finite number`);
    }
    return value;
}

function isObject(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Three finite numbers from a list of three or an `{ x, y, z }` object. Throws a TypeError naming `target`. */
export function vectorFrom(target: string, value: unknown): [number, number, number] {
    if (Array.isArray(value) && value.length === 3) {
        return [
            finite(target, value[0], "every component"),
            finite(target, value[1], "every component"),
            finite(target, value[2], "every component"),
        ];
    }
    if (isObject(value) && "x" in value && "y" in value && "z" in value) {
        return [finite(target, value.x, "x"), finite(target, value.y, "y"), finite(target, value.z, "z")];
    }
    throw new TypeError(`${target}: must be three numbers or { x, y, z }`);
}

/**
 * The angles about X, Y and Z, in radians, of a unit quaternion, for the order the tree uses (turn about X, then Y,
 * then Z: the matrix is `Rz * Ry * Rx`). A quaternion that is not of unit length is normalised first.
 */
export function eulerOfQuaternion(q: QuaternionLike): [number, number, number] {
    const length = Math.hypot(q.x, q.y, q.z, q.w);
    if (length === 0) {
        throw new RangeError("a quaternion of length 0 is not a rotation");
    }
    const [x, y, z, w] = [q.x / length, q.y / length, q.z / length, q.w / length];
    const pitch = Math.asin(Math.max(-1, Math.min(1, 2 * (w * y - z * x))));
    return [
        Math.atan2(2 * (w * x + y * z), 1 - 2 * (x * x + y * y)),
        pitch,
        Math.atan2(2 * (w * z + x * y), 1 - 2 * (y * y + z * z)),
    ];
}

/** Three angles in radians from a list, an `{ x, y, z }` Euler object or a `{ x, y, z, w }` quaternion. */
export function rotationFrom(target: string, value: unknown): [number, number, number] {
    if (isObject(value) && "w" in value) {
        const q = {
            x: finite(target, value.x, "x"),
            y: finite(target, value.y, "y"),
            z: finite(target, value.z, "z"),
            w: finite(target, value.w, "w"),
        };
        try {
            return eulerOfQuaternion(q);
        } catch (error) {
            throw new RangeError(`${target}: ${(error as Error).message}`);
        }
    }
    try {
        return vectorFrom(target, value);
    } catch {
        throw new TypeError(`${target}: must be three angles, { x, y, z } or a quaternion { x, y, z, w }`);
    }
}

/** The scale along each axis from one number (every axis), a list of three or an `{ x, y, z }` object. */
export function scaleFrom(target: string, value: unknown): [number, number, number] {
    if (typeof value === "number") {
        const uniform = finite(target, value, "a uniform scale");
        return [uniform, uniform, uniform];
    }
    try {
        return vectorFrom(target, value);
    } catch {
        throw new TypeError(`${target}: must be a number, three numbers or { x, y, z }`);
    }
}

const TRANSFORM_KEYS = ["position", "rotation", "scale"];

/**
 * Reads a transform: a plain `{ position, rotation, scale }` object or an object with those members (a `Transform3D`).
 * A part that is missing or undefined stays null. Throws a TypeError naming `target` for a part that is not valid or a
 * key that is not one of the three.
 */
export function transformFrom(target: string, value: unknown): ParsedTransform {
    if (!isObject(value)) {
        throw new TypeError(`${target}: must be an object with position, rotation and scale`);
    }
    for (const key of Object.keys(value)) {
        if (!TRANSFORM_KEYS.includes(key)) {
            throw new TypeError(`${target}.${key}: a transform has position, rotation and scale only`);
        }
    }
    return {
        position: value.position === undefined ? null : vectorFrom(`${target}.position`, value.position),
        rotation: value.rotation === undefined ? null : rotationFrom(`${target}.rotation`, value.rotation),
        scale: value.scale === undefined ? null : scaleFrom(`${target}.scale`, value.scale),
    };
}
