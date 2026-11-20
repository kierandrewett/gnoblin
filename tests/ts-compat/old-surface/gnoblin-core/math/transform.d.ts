import { Plane, Vector3 } from "gnoblin-core/math/vector";
/** A position, rotation and uniform scale that can transform points and directions. */
export declare class Transform3D {
    #private;
    /** Translation in the parent coordinate space. */
    position: Vector3;
    /** Uniform scale applied to points before rotation. */
    scale: number;
    /** Creates a pose from a position, Euler rotation in radians and uniform scale. */
    constructor(position?: Vector3, rotation?: readonly number[], scale?: number);
    /** Creates a pose with no translation, rotation or scaling. */
    static identity(): Transform3D;
    /** Creates a pose that translates by the given coordinates. */
    static translation(x: number, y: number, z: number): Transform3D;
    /** The rotation as Euler angles in radians about X, Y and Z. */
    get rotation(): Vector3;
    /** Sets the rotation from Euler angles in radians. */
    set rotation(value: Vector3 | readonly number[]);
    /** Returns an independent copy of this pose. */
    clone(): Transform3D;
    /** Returns the pose formed by placing `other` inside this one. */
    multiply(other: Transform3D): Transform3D;
    /** Returns the inverse pose. */
    inverse(): Transform3D;
    /** Applies this pose's rotation to a direction without translating or scaling it. */
    transformDirection(direction: Vector3): Vector3;
    /** Applies this pose to a point. */
    transformPoint(point: Vector3): Vector3;
    /** Returns the plane through this pose's position, facing along local +Z. */
    plane(): Plane;
}
