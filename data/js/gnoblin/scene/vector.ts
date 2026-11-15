// 3D geometry primitives used by scene objects. Positions use their parent's units; ray and plane operations use the
// coordinate space of their inputs.

/** Returns a finite number or throws when the value is not finite. */
export function finite(target: string, value: unknown): number {
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new TypeError(`${target}: must be a finite number`);
    }
    return value;
}

/** A three-dimensional point or direction. */
export class Vector3 {
    /** X coordinate. */
    x: number;
    /** Y coordinate. */
    y: number;
    /** Z coordinate. */
    z: number;

    /** Creates a vector from three coordinates, each defaulting to zero. */
    constructor(x = 0, y = 0, z = 0) {
        this.x = x;
        this.y = y;
        this.z = z;
    }

    /** Creates a vector with all coordinates set to zero. */
    static zero(): Vector3 {
        return new Vector3();
    }

    /** Creates a vector from an array, using zero for missing coordinates. */
    static from(parts: readonly number[]): Vector3 {
        return new Vector3(parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0);
    }

    /** Returns an independent copy of this vector. */
    clone(): Vector3 {
        return new Vector3(this.x, this.y, this.z);
    }

    /** Adds the coordinates of another vector. */
    add(other: Vector3): Vector3 {
        return new Vector3(this.x + other.x, this.y + other.y, this.z + other.z);
    }

    /** Subtracts another vector's coordinates. */
    subtract(other: Vector3): Vector3 {
        return new Vector3(this.x - other.x, this.y - other.y, this.z - other.z);
    }

    /** Multiplies each coordinate by `factor`. */
    scale(factor: number): Vector3 {
        return new Vector3(this.x * factor, this.y * factor, this.z * factor);
    }

    /** Returns the dot product with another vector. */
    dot(other: Vector3): number {
        return this.x * other.x + this.y * other.y + this.z * other.z;
    }

    /** Returns the cross product with another vector. */
    cross(other: Vector3): Vector3 {
        return new Vector3(
            this.y * other.z - this.z * other.y,
            this.z * other.x - this.x * other.z,
            this.x * other.y - this.y * other.x,
        );
    }

    /** Returns the vector's length. */
    length(): number {
        return Math.hypot(this.x, this.y, this.z);
    }

    /** The unit vector the same way, or zero for a zero vector. */
    normalize(): Vector3 {
        const length = this.length();
        return length === 0 ? new Vector3() : this.scale(1 / length);
    }

    /** Returns the distance to another vector. */
    distanceTo(other: Vector3): number {
        return this.subtract(other).length();
    }

    /** Returns the coordinates as an array. */
    toArray(): [number, number, number] {
        return [this.x, this.y, this.z];
    }
}

/** A half line from `origin` along `direction`, which need not be a unit vector. */
export class Ray {
    /** Starting point of the ray. */
    readonly origin: Vector3;
    /** Direction of the ray; it need not be normalised. */
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
/** A plane through `point` with the given normal vector. */
export class Plane {
    /** Any point on the plane. */
    readonly point: Vector3;
    /** Normal vector to the plane. */
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
