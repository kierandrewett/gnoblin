// Space, part two: `Transform3D`, a position, a rotation and a scale in one value. A rotation is the node's own:
// Euler angles in radians, turned about X, then Y, then Z, so the rotation matrix is `Rz * Ry * Rx`
// (`xr_scene::math::euler_degrees`).

import { Plane, Vector3 } from "gnoblin/scene/vector";

function product(a: readonly number[], b: readonly number[]): number[] {
    const out = new Array<number>(9).fill(0);
    for (let row = 0; row < 3; row++) {
        for (let col = 0; col < 3; col++) {
            out[row * 3 + col] = a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col];
        }
    }
    return out;
}

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

/** The angles about X, Y and Z that `matrixOf` turns into `m`. At a quarter turn about Y the turn about Z is zero. */
function eulerOf(m: readonly number[]): [number, number, number] {
    const b = Math.asin(Math.max(-1, Math.min(1, -m[6])));
    if (Math.abs(m[6]) < 1 - 1e-9) {
        return [Math.atan2(m[7], m[8]), b, Math.atan2(m[3], m[0])];
    }
    return m[6] < 0 ? [Math.atan2(m[1], m[2]), b, 0] : [Math.atan2(-m[1], -m[2]), b, 0];
}

/** A pose: a position, a rotation and one uniform scale. `a.multiply(b)` is `b` placed inside `a`. */
export class Transform3D {
    position: Vector3;
    scale: number;
    /** Row-major 3x3 rotation matrix. */
    #matrix: number[];

    constructor(position = new Vector3(), rotation: readonly number[] = [0, 0, 0], scale = 1) {
        this.position = position;
        this.scale = scale;
        this.#matrix = matrixOf(rotation);
    }

    static identity(): Transform3D {
        return new Transform3D();
    }

    static translation(x: number, y: number, z: number): Transform3D {
        return new Transform3D(new Vector3(x, y, z));
    }

    /** The rotation as Euler angles in radians about X, Y and Z. */
    get rotation(): Vector3 {
        return Vector3.from(eulerOf(this.#matrix));
    }

    set rotation(value: Vector3 | readonly number[]) {
        this.#matrix = matrixOf(value instanceof Vector3 ? value.toArray() : value);
    }

    clone(): Transform3D {
        const copy = new Transform3D(this.position.clone(), [0, 0, 0], this.scale);
        copy.#matrix = [...this.#matrix];
        return copy;
    }

    /** `other` placed inside `this`: the point `p` of `other` is `this.transformPoint(other.transformPoint(p))`. */
    multiply(other: Transform3D): Transform3D {
        const result = new Transform3D(this.transformPoint(other.position), [0, 0, 0], this.scale * other.scale);
        result.#matrix = product(this.#matrix, other.#matrix);
        return result;
    }

    inverse(): Transform3D {
        const m = this.#matrix;
        const transposed = [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];
        const scale = 1 / this.scale;
        const back = new Transform3D(new Vector3(), [0, 0, 0], scale);
        back.#matrix = transposed;
        back.position = back.transformDirection(this.position).scale(-scale);
        return back;
    }

    /** A direction turned by the rotation alone. */
    transformDirection(direction: Vector3): Vector3 {
        const m = this.#matrix;
        const { x, y, z } = direction;
        return new Vector3(
            m[0] * x + m[1] * y + m[2] * z,
            m[3] * x + m[4] * y + m[5] * z,
            m[6] * x + m[7] * y + m[8] * z,
        );
    }

    transformPoint(point: Vector3): Vector3 {
        return this.position.add(this.transformDirection(point.scale(this.scale)));
    }

    /** The plane of a quad that sits at this pose: through the position, facing along the local +Z. */
    plane(): Plane {
        return new Plane(this.position.clone(), this.transformDirection(new Vector3(0, 0, 1)));
    }
}
