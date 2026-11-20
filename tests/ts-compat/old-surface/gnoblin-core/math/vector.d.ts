/** Returns a finite number or throws when the value is not finite. */
export declare function finite(target: string, value: unknown): number;
/** A three-dimensional point or direction. */
export declare class Vector3 {
    /** X coordinate. */
    x: number;
    /** Y coordinate. */
    y: number;
    /** Z coordinate. */
    z: number;
    /** Creates a vector from three coordinates, each defaulting to zero. */
    constructor(x?: number, y?: number, z?: number);
    /** Creates a vector with all coordinates set to zero. */
    static zero(): Vector3;
    /** Creates a vector from an array, using zero for missing coordinates. */
    static from(parts: readonly number[]): Vector3;
    /** Returns an independent copy of this vector. */
    clone(): Vector3;
    /** Adds the coordinates of another vector. */
    add(other: Vector3): Vector3;
    /** Subtracts another vector's coordinates. */
    subtract(other: Vector3): Vector3;
    /** Multiplies each coordinate by `factor`. */
    scale(factor: number): Vector3;
    /** Returns the dot product with another vector. */
    dot(other: Vector3): number;
    /** Returns the cross product with another vector. */
    cross(other: Vector3): Vector3;
    /** Returns the vector's length. */
    length(): number;
    /** The unit vector the same way, or zero for a zero vector. */
    normalize(): Vector3;
    /** Returns the distance to another vector. */
    distanceTo(other: Vector3): number;
    /** Returns the coordinates as an array. */
    toArray(): [number, number, number];
}
/** A half line from `origin` along `direction`, which need not be a unit vector. */
export declare class Ray {
    /** Starting point of the ray. */
    readonly origin: Vector3;
    /** Direction of the ray; it need not be normalised. */
    readonly direction: Vector3;
    constructor(origin: Vector3, direction: Vector3);
    /** The point `distance` along the ray, in units of `direction`'s length. */
    at(distance: number): Vector3;
}
/** A plane through `point` with the given normal vector. */
export declare class Plane {
    /** Any point on the plane. */
    readonly point: Vector3;
    /** Normal vector to the plane. */
    readonly normal: Vector3;
    constructor(point: Vector3, normal: Vector3);
    /** Where `ray` crosses the plane, or null when it is parallel to it or the plane is behind its origin. */
    intersect(ray: Ray): Vector3 | null;
}
