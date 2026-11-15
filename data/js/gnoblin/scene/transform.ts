// A pose combines position, rotation and uniform scale. Rotations use Euler angles in radians about X, then Y, then Z.
// The rotation matrix is `Rz * Ry * Rx` (`xr_scene::math::euler_degrees`).

import { Plane, Vector3 } from "gnoblin/scene/vector";

/** Multiplies two row-major 3x3 matrices. */
function product(a: readonly number[], b: readonly number[]): number[] {
    const out = new Array<number>(9).fill(0);
    for (let row = 0; row < 3; row++) {
        for (let col = 0; col < 3; col++) {
            out[row * 3 + col] = a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col];
        }
    }
    return out;
}

/** Builds the rotation matrix for Euler angles in radians about X, Y and Z. */
function matrixOf(euler: readonly number[]): number[] {
    const [a, b, c] = euler;
    const [sa, ca, sb, cb, sc, cc] = [Math.sin(a), Math.cos(a), Math.sin(b), Math.cos(b), Math.sin(c), Math.cos(c)];
    // Rz * Ry * Rx, written out.
    return [
        cc * cb,
        cc * sb * sa - sc * ca,
        cc * sb * ca + sc * sa,
        sc * cb,
        sc * sb * sa + cc * ca,
        sc * sb * ca - cc * sa,
        -sb,
        cb * sa,
        cb * ca,
    ];
}

/** Extracts Euler angles from a rotation matrix, choosing zero for Z at a quarter turn about Y. */
function eulerOf(m: readonly number[]): [number, number, number] {
    const b = Math.asin(Math.max(-1, Math.min(1, -m[6])));
    if (Math.abs(m[6]) < 1 - 1e-9) {
        return [Math.atan2(m[7], m[8]), b, Math.atan2(m[3], m[0])];
    }
    return m[6] < 0 ? [Math.atan2(m[1], m[2]), b, 0] : [Math.atan2(-m[1], -m[2]), b, 0];
}

/** A position, rotation and uniform scale that can transform points and directions. */
export class Transform3D {
    /** Translation in the parent coordinate space. */
    position: Vector3;
    /** Uniform scale applied to points before rotation. */
    scale: number;
    /** Row-major 3x3 rotation matrix. */
    #matrix: number[];

    /** Creates a pose from a position, Euler rotation in radians and uniform scale. */
    constructor(position = new Vector3(), rotation: readonly number[] = [0, 0, 0], scale = 1) {
        this.position = position;
        this.scale = scale;
        this.#matrix = matrixOf(rotation);
    }
    /** Creates a pose with no translation, rotation or scaling. */
    static identity(): Transform3D {
        return new Transform3D();
    }
    /** Creates a pose that translates by the given coordinates. */
    static translation(x: number, y: number, z: number): Transform3D {
        return new Transform3D(new Vector3(x, y, z));
    }
    /** The rotation as Euler angles in radians about X, Y and Z. */
    get rotation(): Vector3 {
        return Vector3.from(eulerOf(this.#matrix));
    }
    /** Sets the rotation from Euler angles in radians. */
    set rotation(value: Vector3 | readonly number[]) {
        this.#matrix = matrixOf(value instanceof Vector3 ? value.toArray() : value);
    }
    /** Returns an independent copy of this pose. */
    clone(): Transform3D {
        const copy = new Transform3D(this.position.clone(), [0, 0, 0], this.scale);
        copy.#matrix = [...this.#matrix];
        return copy;
    }
    /** Returns the pose formed by placing `other` inside this one. */
    multiply(other: Transform3D): Transform3D {
        const result = new Transform3D(this.transformPoint(other.position), [0, 0, 0], this.scale * other.scale);
        result.#matrix = product(this.#matrix, other.#matrix);
        return result;
    }
    /** Returns the inverse pose. */
    inverse(): Transform3D {
        const m = this.#matrix;
        const transposed = [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];
        const scale = 1 / this.scale;
        const back = new Transform3D(new Vector3(), [0, 0, 0], scale);
        back.#matrix = transposed;
        back.position = back.transformDirection(this.position).scale(-scale);
        return back;
    }
    /** Applies this pose's rotation to a direction without translating or scaling it. */
    transformDirection(direction: Vector3): Vector3 {
        const m = this.#matrix;
        const { x, y, z } = direction;
        return new Vector3(
            m[0] * x + m[1] * y + m[2] * z,
            m[3] * x + m[4] * y + m[5] * z,
            m[6] * x + m[7] * y + m[8] * z,
        );
    }
    /** Applies this pose to a point. */
    transformPoint(point: Vector3): Vector3 {
        return this.position.add(this.transformDirection(point.scale(this.scale)));
    }
    /** Returns the plane through this pose's position, facing along local +Z. */
    plane(): Plane {
        return new Plane(this.position.clone(), this.transformDirection(new Vector3(0, 0, 1)));
    }
}
