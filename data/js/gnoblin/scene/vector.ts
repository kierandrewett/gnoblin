// Space, part one: points and directions (`Vector3`), rays and planes. A pose is a position, a rotation and one
// uniform scale, as the XR program places a View (it reads only the first component of a node's `scale`). Positions
// are in the units of the node's parent; the XR room's unit is the millimetre.

export function finite(target: string, value: unknown): number {
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new TypeError(`${target}: must be a finite number`);
    }
    return value;
}

/** A point or a direction. */
export class Vector3 {
    x: number;
    y: number;
    z: number;

    constructor(x = 0, y = 0, z = 0) {
        this.x = x;
        this.y = y;
        this.z = z;
    }

    static zero(): Vector3 {
        return new Vector3();
    }

    static from(parts: readonly number[]): Vector3 {
        return new Vector3(parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0);
    }

    clone(): Vector3 {
        return new Vector3(this.x, this.y, this.z);
    }

    add(other: Vector3): Vector3 {
        return new Vector3(this.x + other.x, this.y + other.y, this.z + other.z);
    }

    subtract(other: Vector3): Vector3 {
        return new Vector3(this.x - other.x, this.y - other.y, this.z - other.z);
    }

    scale(factor: number): Vector3 {
        return new Vector3(this.x * factor, this.y * factor, this.z * factor);
    }

    dot(other: Vector3): number {
        return this.x * other.x + this.y * other.y + this.z * other.z;
    }

    cross(other: Vector3): Vector3 {
        return new Vector3(
            this.y * other.z - this.z * other.y,
            this.z * other.x - this.x * other.z,
            this.x * other.y - this.y * other.x,
        );
    }

    length(): number {
        return Math.hypot(this.x, this.y, this.z);
    }

    /** The unit vector the same way, or zero for a zero vector. */
    normalize(): Vector3 {
        const length = this.length();
        return length === 0 ? new Vector3() : this.scale(1 / length);
    }

    distanceTo(other: Vector3): number {
        return this.subtract(other).length();
    }

    toArray(): [number, number, number] {
        return [this.x, this.y, this.z];
    }
}

/** A half line: from `origin` along `direction`, which need not be a unit vector. */
export class Ray {
    readonly origin: Vector3;
    readonly direction: Vector3;

    constructor(origin: Vector3, direction: Vector3) {
        this.origin = origin;
        this.direction = direction;
    }

    /** The point `distance` along the ray, in units of `direction`'s length. */
    at(distance: number): Vector3 {
        return this.origin.add(this.direction.scale(distance));
    }
}

/** A plane through `point` with the given `normal`. */
export class Plane {
    readonly point: Vector3;
    readonly normal: Vector3;

    constructor(point: Vector3, normal: Vector3) {
        this.point = point;
        this.normal = normal;
    }

    /** Where `ray` crosses the plane, or null when it is parallel to it or the plane is behind its origin. */
    intersect(ray: Ray): Vector3 | null {
        const denominator = this.normal.dot(ray.direction);
        if (Math.abs(denominator) < 1e-12) {
            return null;
        }
        const t = this.normal.dot(this.point.subtract(ray.origin)) / denominator;
        return t > 0 ? ray.at(t) : null;
    }
}
