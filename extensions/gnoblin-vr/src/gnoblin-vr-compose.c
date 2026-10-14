#define _GNU_SOURCE
#include "gnoblin-vr-compose.h"

#include <fcntl.h>
#include <math.h>
#include <string.h>
#include <unistd.h>

static VrDrawItem* append_item(GArray* items) {
    g_array_set_size(items, items->len + 1);
    VrDrawItem* item = &g_array_index(items, VrDrawItem, items->len - 1);
    memset(item, 0, sizeof *item);
    item->opacity = 1.0f;
    return item;
}

void vr_compose_surfaces(const VrView* view, const VrPose* head, GArray* items) {
    for (guint i = 0; i < view->surfaces->len; i++) {
        const VrSurface* surface = g_ptr_array_index(view->surfaces, i);
        if (!surface->pixels)
            continue;
        VrDrawItem* item = append_item(items);
        VrPose world = vr_surface_world_pose(surface, head);
        item->key = surface->id;
        item->pixels = surface->pixels;
        item->pixel_width = surface->pixel_width;
        item->pixel_height = surface->pixel_height;
        item->serial = surface->serial;
        vr_mat4_from_pose(&world, surface->width, surface->height, item->model);
        item->size[0] = (float)surface->width;
        item->size[1] = (float)surface->height;
        item->radius = VR_WINDOW_RADIUS;
    }
}

static void normalise(double v[3]) {
    double length = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    if (length < 1e-12)
        return;
    for (int i = 0; i < 3; i++)
        v[i] /= length;
}

static void cross(const double a[3], const double b[3], double out[3]) {
    out[0] = a[1] * b[2] - a[2] * b[1];
    out[1] = a[2] * b[0] - a[0] * b[2];
    out[2] = a[0] * b[1] - a[1] * b[0];
}

/* Column-major model matrix from three axes (already scaled) and a translation. */
static void model_from_axes(const double x[3], const double y[3], const double z[3],
                            const double t[3], float out[16]) {
    memset(out, 0, 16 * sizeof(float));
    for (int i = 0; i < 3; i++) {
        out[i] = (float)x[i];
        out[4 + i] = (float)y[i];
        out[8 + i] = (float)z[i];
        out[12 + i] = (float)t[i];
    }
    out[15] = 1.0f;
}

void vr_compose_solid_pixel(guint8 out[4], guint8 red, guint8 green, guint8 blue, guint8 alpha) {
    out[0] = (guint8)(blue * alpha / 255);
    out[1] = (guint8)(green * alpha / 255);
    out[2] = (guint8)(red * alpha / 255);
    out[3] = alpha;
}

void vr_compose_ray(GArray* items, const char* key, const double origin[3],
                    const double direction[3], double length, const double eye[3],
                    const guint8 pixel[4], float width) {
    double axis[3] = {direction[0], direction[1], direction[2]};
    normalise(axis);
    double to_eye[3] = {eye[0] - origin[0], eye[1] - origin[1], eye[2] - origin[2]};
    double side[3];
    cross(axis, to_eye, side);
    if (fabs(side[0]) + fabs(side[1]) + fabs(side[2]) < 1e-9) {
        double fallback[3] = {0, 1, 0};
        cross(axis, fallback, side);
    }
    normalise(side);
    double normal[3];
    cross(side, axis, normal);
    double x[3] = {side[0] * width, side[1] * width, side[2] * width};
    double y[3] = {axis[0] * length, axis[1] * length, axis[2] * length};
    double middle[3] = {origin[0] + axis[0] * length / 2, origin[1] + axis[1] * length / 2,
                        origin[2] + axis[2] * length / 2};
    VrDrawItem* item = append_item(items);
    item->key = key;
    item->pixels = pixel;
    item->pixel_width = 1;
    item->pixel_height = 1;
    item->serial = 1;
    model_from_axes(x, y, normal, middle, item->model);
    item->size[0] = width;
    item->size[1] = (float)length;
    item->radius = width / 2.0f;
}

/* --- Cursor ------------------------------------------------------------------------------ */

void vr_cursor_clear(VrCursor* cursor) {
    g_clear_pointer(&cursor->pixels, g_free);
    memset(cursor, 0, sizeof *cursor);
}

void vr_cursor_default(VrCursor* cursor) {
    vr_cursor_clear(cursor);
    const guint32 size = 32;
    guint8* pixels = g_malloc0((gsize)size * size * 4);
    for (guint32 y = 0; y < size; y++)
        for (guint32 x = 0; x < size; x++) {
            double dx = x + 0.5 - size / 2.0, dy = y + 0.5 - size / 2.0;
            double distance = sqrt(dx * dx + dy * dy);
            /* A white disc with a dark rim, antialiased over one pixel. */
            double disc = CLAMP(9.0 - distance + 0.5, 0.0, 1.0);
            double rim = CLAMP(12.0 - distance + 0.5, 0.0, 1.0) * (1.0 - disc);
            double alpha = disc + rim * 0.85;
            double value = disc * 255.0;
            guint8* out = pixels + ((gsize)y * size + x) * 4;
            out[0] = out[1] = out[2] = (guint8)(value);
            out[3] = (guint8)(alpha * 255.0);
        }
    cursor->pixels = pixels;
    cursor->width = cursor->height = size;
    cursor->hot_x = cursor->hot_y = (gint)(size / 2);
    cursor->serial = 1;
}

gboolean vr_cursor_refresh(VrCursor* cursor, const char* directory) {
    g_autofree char* ini_path = g_build_filename(directory, "cursor.ini", NULL);
    g_autoptr(GKeyFile) file = g_key_file_new();
    if (!g_key_file_load_from_file(file, ini_path, G_KEY_FILE_NONE, NULL))
        return cursor->pixels != NULL;
    guint64 serial = g_key_file_get_uint64(file, "cursor", "serial", NULL);
    if (cursor->pixels && serial == cursor->serial)
        return TRUE;
    gint width = g_key_file_get_integer(file, "cursor", "width", NULL);
    gint height = g_key_file_get_integer(file, "cursor", "height", NULL);
    if (width <= 0 || height <= 0 || width > 512 || height > 512)
        return cursor->pixels != NULL;
    g_autofree char* pixel_path = g_build_filename(directory, "cursor.rgba", NULL);
    g_autofree char* contents = NULL;
    gsize length = 0;
    if (!g_file_get_contents(pixel_path, &contents, &length, NULL) ||
        length != (gsize)width * height * 4)
        return cursor->pixels != NULL;
    /* The compositor writes straight RGBA. The renderer wants premultiplied BGRA. */
    guint8* pixels = g_malloc(length);
    for (gsize i = 0; i < (gsize)width * height; i++) {
        const guint8* in = (const guint8*)contents + i * 4;
        guint8 alpha = in[3];
        pixels[i * 4 + 0] = (guint8)(in[2] * alpha / 255);
        pixels[i * 4 + 1] = (guint8)(in[1] * alpha / 255);
        pixels[i * 4 + 2] = (guint8)(in[0] * alpha / 255);
        pixels[i * 4 + 3] = alpha;
    }
    g_free(cursor->pixels);
    cursor->pixels = pixels;
    cursor->width = (guint32)width;
    cursor->height = (guint32)height;
    cursor->hot_x = g_key_file_get_integer(file, "cursor", "hot_x", NULL);
    cursor->hot_y = g_key_file_get_integer(file, "cursor", "hot_y", NULL);
    cursor->serial = serial;
    return TRUE;
}

void vr_compose_cursor(GArray* items, const VrCursor* cursor, const double point[3],
                       const double eye[3], const double up[3]) {
    if (!cursor->pixels)
        return;
    double normal[3] = {eye[0] - point[0], eye[1] - point[1], eye[2] - point[2]};
    double distance = sqrt(normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]);
    normalise(normal);
    double right[3];
    cross(up, normal, right);
    if (fabs(right[0]) + fabs(right[1]) + fabs(right[2]) < 1e-9) {
        double fallback[3] = {0, 1, 0};
        cross(fallback, normal, right);
    }
    normalise(right);
    double top[3];
    cross(normal, right, top);
    /* 32 sprite pixels span about 2.5 degrees whatever the distance, so it stays readable. */
    double metres_per_pixel = distance * 0.0014;
    double width = cursor->width * metres_per_pixel;
    double height = cursor->height * metres_per_pixel;
    /* Centre the quad so the hotspot lands on the point. */
    double dx = (cursor->width / 2.0 - cursor->hot_x) * metres_per_pixel;
    double dy = (cursor->height / 2.0 - cursor->hot_y) * metres_per_pixel;
    double centre[3];
    for (int i = 0; i < 3; i++)
        centre[i] = point[i] + right[i] * dx - top[i] * dy + normal[i] * 0.004;
    double x[3] = {right[0] * width, right[1] * width, right[2] * width};
    double y[3] = {top[0] * height, top[1] * height, top[2] * height};
    VrDrawItem* item = append_item(items);
    item->key = "cursor";
    item->pixels = cursor->pixels;
    item->pixel_width = cursor->width;
    item->pixel_height = cursor->height;
    item->serial = cursor->serial;
    model_from_axes(x, y, normal, centre, item->model);
    item->size[0] = (float)width;
    item->size[1] = (float)height;
    item->radius = 0.0f;
}
