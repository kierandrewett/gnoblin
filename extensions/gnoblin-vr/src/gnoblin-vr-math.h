#pragma once

#include <math.h>
#include <string.h>

/* Small column-major 4x4 matrix helpers, enough to place quads and project them per eye. */

typedef struct {
    double position[3];
    double rotation[4]; /* x, y, z, w */
} VrPose;

static inline void vr_quat_rotate(const double q[4], const double v[3], double out[3]) {
    double c[3] = {q[1] * v[2] - q[2] * v[1] + q[3] * v[0], q[2] * v[0] - q[0] * v[2] + q[3] * v[1],
                   q[0] * v[1] - q[1] * v[0] + q[3] * v[2]};
    out[0] = v[0] + 2.0 * (q[1] * c[2] - q[2] * c[1]);
    out[1] = v[1] + 2.0 * (q[2] * c[0] - q[0] * c[2]);
    out[2] = v[2] + 2.0 * (q[0] * c[1] - q[1] * c[0]);
}

static inline void vr_quat_multiply(const double a[4], const double b[4], double out[4]) {
    out[0] = a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1];
    out[1] = a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0];
    out[2] = a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3];
    out[3] = a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2];
}

/* Composes a parent pose with a pose that is local to it. */
static inline VrPose vr_pose_compose(const VrPose* parent, const VrPose* local) {
    VrPose world;
    double offset[3];
    vr_quat_rotate(parent->rotation, local->position, offset);
    for (int i = 0; i < 3; i++)
        world.position[i] = parent->position[i] + offset[i];
    vr_quat_multiply(parent->rotation, local->rotation, world.rotation);
    return world;
}

/* out = a * b */
static inline void vr_mat4_multiply(const float a[16], const float b[16], float out[16]) {
    float result[16];
    for (int column = 0; column < 4; column++)
        for (int row = 0; row < 4; row++) {
            float sum = 0.0f;
            for (int k = 0; k < 4; k++)
                sum += a[k * 4 + row] * b[column * 4 + k];
            result[column * 4 + row] = sum;
        }
    memcpy(out, result, sizeof result);
}

/* Model matrix of a pose with a non-uniform scale in X and Y. */
static inline void vr_mat4_from_pose(const VrPose* pose, double scale_x, double scale_y,
                                     float out[16]) {
    const double* q = pose->rotation;
    double x = q[0], y = q[1], z = q[2], w = q[3];
    double r[9] = {1 - 2 * (y * y + z * z), 2 * (x * y + z * w),     2 * (x * z - y * w),
                   2 * (x * y - z * w),     1 - 2 * (x * x + z * z), 2 * (y * z + x * w),
                   2 * (x * z + y * w),     2 * (y * z - x * w),     1 - 2 * (x * x + y * y)};
    memset(out, 0, 16 * sizeof(float));
    for (int i = 0; i < 3; i++) {
        out[i] = (float)(r[i] * scale_x);
        out[4 + i] = (float)(r[3 + i] * scale_y);
        out[8 + i] = (float)r[6 + i];
        out[12 + i] = (float)pose->position[i];
    }
    out[15] = 1.0f;
}

/* View matrix: the inverse of a rigid pose. */
static inline void vr_mat4_view(const VrPose* pose, float out[16]) {
    double conjugate[4] = {-pose->rotation[0], -pose->rotation[1], -pose->rotation[2],
                           pose->rotation[3]};
    VrPose inverse = {{0, 0, 0}, {conjugate[0], conjugate[1], conjugate[2], conjugate[3]}};
    double negative[3] = {-pose->position[0], -pose->position[1], -pose->position[2]};
    vr_quat_rotate(conjugate, negative, inverse.position);
    vr_mat4_from_pose(&inverse, 1.0, 1.0, out);
}

/* Perspective from OpenXR half-angles (radians), depth range 0..1, camera looking down -Z with +Y
 * up. A Vulkan renderer flips Y with a negative-height viewport. */
static inline void vr_mat4_projection(double left, double right, double up, double down,
                                      float near_plane, float far_plane, float out[16]) {
    double tan_left = tan(left), tan_right = tan(right), tan_up = tan(up), tan_down = tan(down);
    double width = tan_right - tan_left, height = tan_up - tan_down;
    memset(out, 0, 16 * sizeof(float));
    out[0] = (float)(2.0 / width);
    out[5] = (float)(2.0 / height);
    out[8] = (float)((tan_right + tan_left) / width);
    out[9] = (float)((tan_up + tan_down) / height);
    out[10] = -far_plane / (far_plane - near_plane);
    out[11] = -1.0f;
    out[14] = -(far_plane * near_plane) / (far_plane - near_plane);
}
