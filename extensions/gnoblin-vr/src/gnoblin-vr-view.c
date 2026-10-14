#define _GNU_SOURCE
#include "gnoblin-vr-view.h"
#include "gnoblin-vr-frame.h"

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static void surface_free(gpointer data) {
    VrSurface* surface = data;
    g_free(surface->id);
    g_free(surface->frame);
    g_free(surface->pixels);
    g_free(surface);
}

VrView* vr_view_new(const char* directory) {
    VrView* view = g_new0(VrView, 1);
    view->surfaces = g_ptr_array_new_with_free_func(surface_free);
    view->directory = g_strdup(directory);
    return view;
}

void vr_view_free(VrView* view) {
    if (!view)
        return;
    g_ptr_array_unref(view->surfaces);
    g_free(view->directory);
    g_free(view->environment);
    g_free(view);
}

static gboolean parse_numbers(const char* text, double* out, guint count) {
    if (!text)
        return FALSE;
    g_auto(GStrv) parts = g_strsplit(text, ";", -1);
    if (g_strv_length(parts) != count)
        return FALSE;
    for (guint i = 0; i < count; i++) {
        char* end = NULL;
        out[i] = g_ascii_strtod(parts[i], &end);
        if (end == parts[i] || !isfinite(out[i]))
            return FALSE;
    }
    return TRUE;
}

static gboolean read_pose(GKeyFile* file, const char* group, const char* prefix, VrPose* pose) {
    g_autofree char* position_key = g_strdup_printf("%s_pos", prefix);
    g_autofree char* rotation_key = g_strdup_printf("%s_rot", prefix);
    g_autofree char* position = g_key_file_get_string(file, group, position_key, NULL);
    g_autofree char* rotation = g_key_file_get_string(file, group, rotation_key, NULL);
    return parse_numbers(position, pose->position, 3) && parse_numbers(rotation, pose->rotation, 4);
}

static VrSurface* find_surface(GPtrArray* surfaces, const char* id) {
    for (guint i = 0; i < surfaces->len; i++) {
        VrSurface* surface = g_ptr_array_index(surfaces, i);
        if (g_str_equal(surface->id, id))
            return surface;
    }
    return NULL;
}

static gboolean reload_scene(VrView* view, const char* path) {
    g_autoptr(GKeyFile) file = g_key_file_new();
    if (!g_key_file_load_from_file(file, path, G_KEY_FILE_NONE, NULL))
        return FALSE;
    GPtrArray* next = g_ptr_array_new_with_free_func(surface_free);
    guint64 revision = g_key_file_get_uint64(file, "scene", "revision", NULL);
    gint count = CLAMP(g_key_file_get_integer(file, "scene", "count", NULL), 0, 256);
    for (gint i = 0; i < count; i++) {
        g_autofree char* group = g_strdup_printf("surface.%d", i);
        g_autofree char* id = g_key_file_get_string(file, group, "id", NULL);
        g_autofree char* frame = g_key_file_get_string(file, group, "frame", NULL);
        g_autofree char* space = g_key_file_get_string(file, group, "space", NULL);
        VrPose anchor, local;
        if (!id || !frame || !read_pose(file, group, "anchor", &anchor) ||
            !read_pose(file, group, "local", &local))
            continue;
        VrSurface* surface = g_new0(VrSurface, 1);
        surface->id = g_strdup(id);
        surface->frame = g_strdup(frame);
        surface->head_locked = space && g_str_equal(space, "view");
        surface->anchor = anchor;
        surface->local = local;
        surface->width = g_key_file_get_double(file, group, "width", NULL);
        surface->height = g_key_file_get_double(file, group, "height", NULL);
        /* Keep the pixels already loaded for this window so a pose change does not flash. */
        VrSurface* previous = find_surface(view->surfaces, id);
        if (previous) {
            surface->pixels = g_steal_pointer(&previous->pixels);
            surface->pixel_width = previous->pixel_width;
            surface->pixel_height = previous->pixel_height;
            surface->serial = previous->serial;
            surface->texture_key = previous->texture_key;
        }
        g_ptr_array_add(next, surface);
    }
    g_ptr_array_unref(view->surfaces);
    view->surfaces = next;
    view->revision = revision;
    g_free(view->environment);
    view->environment = g_key_file_get_string(file, "scene", "environment", NULL);
    return TRUE;
}

static gboolean read_frame(VrView* view, VrSurface* surface) {
    g_autofree char* name = g_strdup_printf("%s.rgba", surface->frame);
    g_autofree char* path = g_build_filename(view->directory, "frames", name, NULL);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return FALSE;
    GnoblinVrFrameHeader header;
    gboolean changed = FALSE;
    if (pread(fd, &header, sizeof header, 0) == (ssize_t)sizeof header &&
        header.magic == GNOBLIN_VR_FRAME_MAGIC && header.width > 0 && header.height > 0 &&
        header.width <= GNOBLIN_VR_FRAME_MAX_DIMENSION &&
        header.height <= GNOBLIN_VR_FRAME_MAX_DIMENSION && header.stride >= header.width * 4 &&
        header.serial != surface->serial) {
        gsize size = (gsize)header.stride * header.height;
        guint8* pixels = g_malloc(size);
        if (pread(fd, pixels, size, sizeof header) == (ssize_t)size) {
            g_free(surface->pixels);
            surface->pixels = pixels;
            surface->pixel_width = header.width;
            surface->pixel_height = header.height;
            surface->serial = header.serial;
            surface->fresh = TRUE;
            changed = TRUE;
        } else {
            g_free(pixels);
        }
    }
    close(fd);
    return changed;
}

gboolean vr_view_refresh(VrView* view) {
    g_autofree char* path = g_build_filename(view->directory, "scene.ini", NULL);
    struct stat info;
    gboolean changed = FALSE;
    if (stat(path, &info) == 0) {
        gint64 mtime = (gint64)info.st_mtim.tv_sec * G_USEC_PER_SEC + info.st_mtim.tv_nsec / 1000;
        if (mtime != view->scene_mtime || (gint64)info.st_size != view->scene_size) {
            if (reload_scene(view, path)) {
                view->scene_mtime = mtime;
                view->scene_size = (gint64)info.st_size;
                changed = TRUE;
            }
        }
    }
    for (guint i = 0; i < view->surfaces->len; i++)
        changed |= read_frame(view, g_ptr_array_index(view->surfaces, i));
    return changed;
}

VrPose vr_surface_world_pose(const VrSurface* surface, const VrPose* head) {
    VrPose anchor = surface->anchor;
    if (surface->head_locked && head)
        anchor = vr_pose_compose(head, &surface->anchor);
    return vr_pose_compose(&anchor, &surface->local);
}

gboolean vr_view_hit(const VrView* view, const VrPose* head, const double origin[3],
                     const double direction[3], VrHit* hit) {
    double length = sqrt(direction[0] * direction[0] + direction[1] * direction[1] +
                         direction[2] * direction[2]);
    if (length < 1e-9)
        return FALSE;
    double unit[3] = {direction[0] / length, direction[1] / length, direction[2] / length};
    gboolean found = FALSE;
    /* Surfaces are stored lowest rank first, so on a tie the later one wins. */
    for (guint i = 0; i < view->surfaces->len; i++) {
        const VrSurface* surface = g_ptr_array_index(view->surfaces, i);
        VrPose world = vr_surface_world_pose(surface, head);
        double conjugate[4] = {-world.rotation[0], -world.rotation[1], -world.rotation[2],
                               world.rotation[3]};
        double relative[3] = {origin[0] - world.position[0], origin[1] - world.position[1],
                              origin[2] - world.position[2]};
        double local_origin[3], local_direction[3];
        vr_quat_rotate(conjugate, relative, local_origin);
        vr_quat_rotate(conjugate, unit, local_direction);
        if (fabs(local_direction[2]) < 1e-9)
            continue;
        double distance = -local_origin[2] / local_direction[2];
        if (distance <= 0.0)
            continue;
        double x = local_origin[0] + distance * local_direction[0];
        double y = local_origin[1] + distance * local_direction[1];
        if (fabs(x) > surface->width / 2.0 || fabs(y) > surface->height / 2.0)
            continue;
        if (found && distance > hit->distance + 1e-6)
            continue;
        found = TRUE;
        hit->surface = surface;
        hit->distance = distance;
        hit->u = x / surface->width + 0.5;
        hit->v = 0.5 - y / surface->height;
        for (int axis = 0; axis < 3; axis++)
            hit->point[axis] = origin[axis] + unit[axis] * distance;
    }
    return found;
}
