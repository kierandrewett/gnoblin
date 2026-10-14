#include "gnoblin-vr-scene.h"

#include <math.h>
#include <string.h>

typedef struct {
    double position[3];
    double rotation[4];
} Pose;

typedef struct {
    char* id;
    char* space_id;
    Pose pose;
} Anchor;

typedef struct {
    char* id;
    char* anchor_id;
    gboolean docked; /* docked to a viewport anchor, otherwise free in anchor space */
    Pose pose;
    double width;
    double height;
    guint64 rank;
} Surface;

static GHashTable* anchors;  /* id -> Anchor */
static GHashTable* surfaces; /* id -> Surface */
static guint64 revision;
static guint64 next_rank;
/* Head pose in world space; anchors in the "view" space are relative to it. */
static Pose head_pose = {{0.0, 0.0, 0.0}, {0.0, 0.0, 0.0, 1.0}};

static void anchor_free(gpointer data) {
    Anchor* anchor = data;
    g_free(anchor->id);
    g_free(anchor->space_id);
    g_free(anchor);
}

static void surface_free(gpointer data) {
    Surface* surface = data;
    g_free(surface->id);
    g_free(surface->anchor_id);
    g_free(surface);
}

static void ensure_tables(void) {
    if (!anchors)
        anchors = g_hash_table_new_full(g_str_hash, g_str_equal, NULL, anchor_free);
    if (!surfaces)
        surfaces = g_hash_table_new_full(g_str_hash, g_str_equal, NULL, surface_free);
}

void gnoblin_vr_scene_reset(void) {
    g_clear_pointer(&anchors, g_hash_table_unref);
    g_clear_pointer(&surfaces, g_hash_table_unref);
    revision = 0;
    next_rank = 0;
    head_pose = (Pose){{0.0, 0.0, 0.0}, {0.0, 0.0, 0.0, 1.0}};
}

static gboolean number_of(GVariant* dict, const char* key, double* out) {
    g_autoptr(GVariant) value = g_variant_lookup_value(dict, key, NULL);
    if (!value)
        return FALSE;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *out = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *out = (double)g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *out = (double)g_variant_get_int32(value);
    else
        return FALSE;
    return isfinite(*out);
}

static gboolean vector_of(GVariant* dict, const char* key, const char* const* names, guint count,
                          double* out) {
    g_autoptr(GVariant) vector = g_variant_lookup_value(dict, key, G_VARIANT_TYPE_VARDICT);
    if (!vector)
        return FALSE;
    for (guint i = 0; i < count; i++)
        if (!number_of(vector, names[i], &out[i]))
            return FALSE;
    return TRUE;
}

static gboolean pose_of(GVariant* arguments, Pose* pose, GError** error) {
    static const char* const position[] = {"x", "y", "z"};
    static const char* const rotation[] = {"x", "y", "z", "w"};
    g_autoptr(GVariant) value = g_variant_lookup_value(arguments, "pose", G_VARIANT_TYPE_VARDICT);
    gboolean valid = value && vector_of(value, "position", position, 3, pose->position) &&
                     vector_of(value, "rotation", rotation, 4, pose->rotation);
    if (valid) {
        double norm = 0.0;
        for (guint i = 0; i < 4; i++)
            norm += pose->rotation[i] * pose->rotation[i];
        valid = fabs(norm - 1.0) < 0.01;
    }
    if (!valid)
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "pose needs finite position and a unit rotation quaternion");
    return valid;
}

static gboolean size_of(GVariant* arguments, double* width, double* height, GError** error) {
    g_autoptr(GVariant) value = g_variant_lookup_value(arguments, "size", G_VARIANT_TYPE_VARDICT);
    gboolean valid = value && number_of(value, "width", width) &&
                     number_of(value, "height", height) && *width > 0.0 && *height > 0.0;
    if (!valid)
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "size needs positive width and height in metres");
    return valid;
}

static const char* id_of(GVariant* arguments, const char* key, GError** error) {
    const char* id = NULL;
    if (!g_variant_lookup(arguments, key, "&s", &id) || !*id || strlen(id) > 128) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "%s must be a nonempty string up to 128 bytes", key);
        return NULL;
    }
    return id;
}

static GVariant* pose_variant(const Pose* pose) {
    GVariantBuilder position, rotation, pose_builder;
    static const char* const p[] = {"x", "y", "z"};
    static const char* const r[] = {"x", "y", "z", "w"};
    g_variant_builder_init(&position, G_VARIANT_TYPE_VARDICT);
    for (guint i = 0; i < 3; i++)
        g_variant_builder_add(&position, "{sv}", p[i], g_variant_new_double(pose->position[i]));
    g_variant_builder_init(&rotation, G_VARIANT_TYPE_VARDICT);
    for (guint i = 0; i < 4; i++)
        g_variant_builder_add(&rotation, "{sv}", r[i], g_variant_new_double(pose->rotation[i]));
    g_variant_builder_init(&pose_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&pose_builder, "{sv}", "position", g_variant_builder_end(&position));
    g_variant_builder_add(&pose_builder, "{sv}", "rotation", g_variant_builder_end(&rotation));
    return g_variant_builder_end(&pose_builder);
}

static GVariant* anchor_variant(const Anchor* anchor) {
    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&b, "{sv}", "id", g_variant_new_string(anchor->id));
    g_variant_builder_add(&b, "{sv}", "space_id", g_variant_new_string(anchor->space_id));
    g_variant_builder_add(&b, "{sv}", "pose", pose_variant(&anchor->pose));
    return g_variant_builder_end(&b);
}

static GVariant* surface_variant(const Surface* surface) {
    GVariantBuilder b, size;
    g_variant_builder_init(&size, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&size, "{sv}", "width", g_variant_new_double(surface->width));
    g_variant_builder_add(&size, "{sv}", "height", g_variant_new_double(surface->height));
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&b, "{sv}", "id", g_variant_new_string(surface->id));
    g_variant_builder_add(&b, "{sv}", surface->docked ? "viewport_id" : "anchor_id",
                          g_variant_new_string(surface->anchor_id));
    g_variant_builder_add(&b, "{sv}", "mode",
                          g_variant_new_string(surface->docked ? "docked" : "free"));
    g_variant_builder_add(&b, "{sv}", "pose", pose_variant(&surface->pose));
    g_variant_builder_add(&b, "{sv}", "size", g_variant_builder_end(&size));
    g_variant_builder_add(&b, "{sv}", "rank", g_variant_new_uint64(surface->rank));
    return g_variant_builder_end(&b);
}

static GVariant* accepted(void) {
    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&b, "{sv}", "revision", g_variant_new_uint64(revision));
    return g_variant_ref_sink(g_variant_builder_end(&b));
}

static Surface* find_surface(const char* id, GError** error) {
    Surface* surface = g_hash_table_lookup(surfaces, id);
    if (!surface)
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "no spatial surface '%s'", id);
    return surface;
}

static GVariant* place_or_attach(GVariant* arguments, gboolean docked, GError** error) {
    const char* surface_id = id_of(arguments, "surface_id", error);
    const char* parent_id =
        surface_id ? id_of(arguments, docked ? "viewport_id" : "anchor_id", error) : NULL;
    Pose pose;
    double width, height;
    if (!parent_id || !pose_of(arguments, &pose, error) ||
        !size_of(arguments, &width, &height, error))
        return NULL;
    if (!g_hash_table_contains(anchors, parent_id)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "no spatial anchor '%s'", parent_id);
        return NULL;
    }
    Surface* surface = g_hash_table_lookup(surfaces, surface_id);
    if (!surface) {
        surface = g_new0(Surface, 1);
        surface->id = g_strdup(surface_id);
        g_hash_table_replace(surfaces, surface->id, surface);
    }
    g_free(surface->anchor_id);
    surface->anchor_id = g_strdup(parent_id);
    surface->docked = docked;
    surface->pose = pose;
    surface->width = width;
    surface->height = height;
    surface->rank = ++next_rank;
    revision++;
    return accepted();
}

/* --- Ray hit testing -------------------------------------------------------------------- */

static void rotate_vector(const double q[4], const double v[3], double out[3]) {
    /* v' = v + 2 * cross(qv, cross(qv, v) + w * v) for a unit quaternion (x, y, z, w). */
    const double* u = q;
    double c[3] = {u[1] * v[2] - u[2] * v[1] + q[3] * v[0], u[2] * v[0] - u[0] * v[2] + q[3] * v[1],
                   u[0] * v[1] - u[1] * v[0] + q[3] * v[2]};
    out[0] = v[0] + 2.0 * (u[1] * c[2] - u[2] * c[1]);
    out[1] = v[1] + 2.0 * (u[2] * c[0] - u[0] * c[2]);
    out[2] = v[2] + 2.0 * (u[0] * c[1] - u[1] * c[0]);
}

static void multiply_quaternion(const double a[4], const double b[4], double out[4]) {
    out[0] = a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1];
    out[1] = a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0];
    out[2] = a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3];
    out[3] = a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2];
}

/* Composes an anchor pose with a surface pose that is local to it. */
static Pose world_pose(const Pose* anchor, const Pose* local) {
    Pose world;
    double offset[3];
    rotate_vector(anchor->rotation, local->position, offset);
    for (guint i = 0; i < 3; i++)
        world.position[i] = anchor->position[i] + offset[i];
    multiply_quaternion(anchor->rotation, local->rotation, world.rotation);
    return world;
}

static gboolean vector3_of(GVariant* arguments, const char* key, double* out, GError** error) {
    static const char* const names[] = {"x", "y", "z"};
    if (!vector_of(arguments, key, names, 3, out)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "%s must be a table of finite x, y, z numbers", key);
        return FALSE;
    }
    return TRUE;
}

/* Casts a ray against every surface that is visible in world space. A surface is a flat
 * rectangle facing local +Z, centred on its pose, with width along X and height along Y.
 * The nearest hit wins; coplanar overlaps fall back to stacking rank. Surfaces under a
 * head-locked ("view") anchor follow the head pose last set with vr.spatial.head. */
static GVariant* hit_test(GVariant* arguments, GError** error) {
    double origin[3], direction[3];
    if (!vector3_of(arguments, "origin", origin, error) ||
        !vector3_of(arguments, "direction", direction, error))
        return NULL;
    double length = sqrt(direction[0] * direction[0] + direction[1] * direction[1] +
                         direction[2] * direction[2]);
    if (length < 1e-9) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "direction must not be zero");
        return NULL;
    }
    for (guint i = 0; i < 3; i++)
        direction[i] /= length;

    const Surface* best = NULL;
    double best_distance = 0.0, best_u = 0.0, best_v = 0.0;
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, surfaces);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        const Surface* surface = value;
        const Anchor* anchor = g_hash_table_lookup(anchors, surface->anchor_id);
        if (!anchor)
            continue;
        Pose anchor_world = g_str_equal(anchor->space_id, "view")
                                ? world_pose(&head_pose, &anchor->pose)
                                : anchor->pose;
        Pose world = world_pose(&anchor_world, &surface->pose);
        double conjugate[4] = {-world.rotation[0], -world.rotation[1], -world.rotation[2],
                               world.rotation[3]};
        double relative[3] = {origin[0] - world.position[0], origin[1] - world.position[1],
                              origin[2] - world.position[2]};
        double local_origin[3], local_direction[3];
        rotate_vector(conjugate, relative, local_origin);
        rotate_vector(conjugate, direction, local_direction);
        if (fabs(local_direction[2]) < 1e-9)
            continue; /* parallel to the surface */
        double distance = -local_origin[2] / local_direction[2];
        if (distance <= 0.0)
            continue; /* behind the ray origin */
        double x = local_origin[0] + distance * local_direction[0];
        double y = local_origin[1] + distance * local_direction[1];
        if (fabs(x) > surface->width / 2.0 || fabs(y) > surface->height / 2.0)
            continue;
        gboolean nearer = !best || distance < best_distance - 1e-6;
        gboolean tie_higher =
            best && fabs(distance - best_distance) <= 1e-6 && surface->rank > best->rank;
        if (nearer || tie_higher) {
            best = surface;
            best_distance = distance;
            best_u = x / surface->width + 0.5;
            best_v = 0.5 - y / surface->height;
        }
    }

    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&b, "{sv}", "hit", g_variant_new_boolean(best != NULL));
    if (best) {
        g_variant_builder_add(&b, "{sv}", "surface_id", g_variant_new_string(best->id));
        g_variant_builder_add(&b, "{sv}", "u", g_variant_new_double(best_u));
        g_variant_builder_add(&b, "{sv}", "v", g_variant_new_double(best_v));
        g_variant_builder_add(&b, "{sv}", "distance", g_variant_new_double(best_distance));
    }
    return g_variant_ref_sink(g_variant_builder_end(&b));
}

static GVariant* scene_call(const char* method, GVariant* arguments, GError** error) {
    ensure_tables();
    if (g_str_equal(method, "vr.spatial.snapshot")) {
        GVariantBuilder anchor_list, surface_list, result;
        GHashTableIter iter;
        gpointer value;
        g_variant_builder_init(&anchor_list, G_VARIANT_TYPE("aa{sv}"));
        g_hash_table_iter_init(&iter, anchors);
        while (g_hash_table_iter_next(&iter, NULL, &value))
            g_variant_builder_add_value(&anchor_list, anchor_variant(value));
        g_variant_builder_init(&surface_list, G_VARIANT_TYPE("aa{sv}"));
        g_hash_table_iter_init(&iter, surfaces);
        while (g_hash_table_iter_next(&iter, NULL, &value))
            g_variant_builder_add_value(&surface_list, surface_variant(value));
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "revision", g_variant_new_uint64(revision));
        g_variant_builder_add(&result, "{sv}", "head", pose_variant(&head_pose));
        g_variant_builder_add(&result, "{sv}", "anchors", g_variant_builder_end(&anchor_list));
        g_variant_builder_add(&result, "{sv}", "surfaces", g_variant_builder_end(&surface_list));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }
    if (g_str_equal(method, "vr.spatial.head")) {
        Pose pose;
        if (!pose_of(arguments, &pose, error))
            return NULL;
        head_pose = pose;
        revision++;
        return accepted();
    }
    if (g_str_equal(method, "vr.spatial.anchor")) {
        const char* id = id_of(arguments, "id", error);
        Pose pose;
        const char* space = "world";
        if (!id || !pose_of(arguments, &pose, error))
            return NULL;
        g_variant_lookup(arguments, "space_id", "&s", &space);
        if (!g_str_equal(space, "world") && !g_str_equal(space, "view")) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "space_id must be 'world' or 'view'");
            return NULL;
        }
        Anchor* anchor = g_new0(Anchor, 1);
        anchor->id = g_strdup(id);
        anchor->space_id = g_strdup(space);
        anchor->pose = pose;
        g_hash_table_replace(anchors, anchor->id, anchor);
        revision++;
        return accepted();
    }
    if (g_str_equal(method, "vr.spatial.hit"))
        return hit_test(arguments, error);
    if (g_str_equal(method, "vr.spatial.place"))
        return place_or_attach(arguments, FALSE, error);
    if (g_str_equal(method, "vr.spatial.attach"))
        return place_or_attach(arguments, TRUE, error);

    const char* surface_id = id_of(arguments, "surface_id", error);
    if (!surface_id)
        return NULL;
    Surface* surface = find_surface(surface_id, error);
    if (!surface)
        return NULL;
    if (g_str_equal(method, "vr.spatial.surface"))
        return g_variant_ref_sink(surface_variant(surface));
    if (g_str_equal(method, "vr.spatial.raise")) {
        surface->rank = ++next_rank;
        revision++;
        return accepted();
    }
    if (g_str_equal(method, "vr.spatial.update_pose") || g_str_equal(method, "vr.spatial.move")) {
        Pose pose;
        if (!pose_of(arguments, &pose, error))
            return NULL;
        surface->pose = pose;
        revision++;
        return accepted();
    }
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED, "unknown spatial method '%s'", method);
    return NULL;
}

/* --- scene.ini ---------------------------------------------------------------------------- */

static void format_pose(GString* out, const char* prefix, const Pose* pose) {
    g_string_append_printf(out, "%s_pos=%.6f;%.6f;%.6f\n", prefix, pose->position[0],
                           pose->position[1], pose->position[2]);
    g_string_append_printf(out, "%s_rot=%.6f;%.6f;%.6f;%.6f\n", prefix, pose->rotation[0],
                           pose->rotation[1], pose->rotation[2], pose->rotation[3]);
}

static gint compare_rank(gconstpointer a, gconstpointer b) {
    const Surface* left = *(const Surface* const*)a;
    const Surface* right = *(const Surface* const*)b;
    return left->rank < right->rank ? -1 : left->rank > right->rank;
}

/* The presentation client renders from this file: every surface with the pose of its anchor and
 * its own pose, left separate so head-locked anchors follow the live head pose in the client.
 * Surfaces are written lowest rank first. */
static void publish_scene(void) {
    g_autofree char* directory = g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", NULL);
    g_mkdir_with_parents(directory, 0700);
    g_autoptr(GPtrArray) ordered = g_ptr_array_new();
    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, surfaces);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        const Surface* surface = value;
        if (g_hash_table_contains(anchors, surface->anchor_id))
            g_ptr_array_add(ordered, value);
    }
    g_ptr_array_sort(ordered, compare_rank);
    GString* out = g_string_new(NULL);
    g_string_append_printf(out, "[scene]\nrevision=%" G_GUINT64_FORMAT "\ncount=%u\n", revision,
                           ordered->len);
    for (guint i = 0; i < ordered->len; i++) {
        const Surface* surface = g_ptr_array_index(ordered, i);
        const Anchor* anchor = g_hash_table_lookup(anchors, surface->anchor_id);
        g_autofree char* frame = gnoblin_vr_frame_name(surface->id);
        g_string_append_printf(out, "\n[surface.%u]\nid=%s\nframe=%s\nspace=%s\ndocked=%d\n", i,
                               surface->id, frame, anchor->space_id, surface->docked ? 1 : 0);
        format_pose(out, "anchor", &anchor->pose);
        format_pose(out, "local", &surface->pose);
        g_string_append_printf(out, "width=%.6f\nheight=%.6f\n", surface->width, surface->height);
    }
    g_autofree char* path = g_build_filename(directory, "scene.ini", NULL);
    g_autoptr(GError) error = NULL;
    if (!g_file_set_contents(path, out->str, (gssize)out->len, &error))
        g_warning("gnoblin-vr: cannot publish the scene: %s", error->message);
    g_string_free(out, TRUE);
}

GVariant* gnoblin_vr_scene_call(const char* method, GVariant* arguments, GError** error) {
    guint64 before = revision;
    GVariant* result = scene_call(method, arguments, error);
    if (result && revision != before)
        publish_scene();
    return result;
}

char* gnoblin_vr_frame_name(const char* surface_id) {
    GString* name = g_string_new(NULL);
    for (const char* c = surface_id; *c; c++)
        g_string_append_c(name, g_ascii_isalnum(*c) || *c == '-' || *c == '_' ? *c : '_');
    return g_string_free(name, FALSE);
}

gboolean gnoblin_vr_scene_has_surfaces(void) {
    return surfaces && g_hash_table_size(surfaces) > 0;
}

char** gnoblin_vr_scene_surface_ids(void) {
    GPtrArray* ids = g_ptr_array_new();
    GHashTableIter iter;
    gpointer key;
    if (surfaces) {
        g_hash_table_iter_init(&iter, surfaces);
        while (g_hash_table_iter_next(&iter, &key, NULL))
            g_ptr_array_add(ids, g_strdup(key));
    }
    g_ptr_array_add(ids, NULL);
    return (char**)g_ptr_array_free(ids, FALSE);
}

static GVariant* status_views(GKeyFile* file) {
    gint views = CLAMP(g_key_file_get_integer(file, "vr", "view_count", NULL), 0, 8);
    GVariantBuilder view_list;
    g_variant_builder_init(&view_list, G_VARIANT_TYPE("aa{sv}"));
    for (gint i = 0; i < views; i++) {
        g_autofree char* width_key = g_strdup_printf("view_%d_width", i);
        g_autofree char* height_key = g_strdup_printf("view_%d_height", i);
        GVariantBuilder view;
        g_variant_builder_init(&view, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(
            &view, "{sv}", "width",
            g_variant_new_int32(g_key_file_get_integer(file, "vr", width_key, NULL)));
        g_variant_builder_add(
            &view, "{sv}", "height",
            g_variant_new_int32(g_key_file_get_integer(file, "vr", height_key, NULL)));
        g_variant_builder_add_value(&view_list, g_variant_builder_end(&view));
    }
    return g_variant_builder_end(&view_list);
}

static GKeyFile* load_runtime_file(const char* name) {
    g_autofree char* path = g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", name, NULL);
    GKeyFile* file = g_key_file_new();
    if (!g_key_file_load_from_file(file, path, G_KEY_FILE_NONE, NULL))
        g_clear_pointer(&file, g_key_file_unref);
    return file;
}

static gboolean headset_session_state(const char* state) {
    return g_str_equal(state, "ready") || g_str_equal(state, "connected");
}

/* One xr record per headset that gnoblin-vr --watch found, in devices.ini order. The session
 * status in status.ini (state, per-eye views) belongs to the first headset that can host one.
 * Without devices.ini the extension reports a single xr:0 from the status file alone, which is
 * how a Monado preview window with no hardware appears. */
GVariant* gnoblin_vr_output_records(void) {
    g_autoptr(GKeyFile) status = load_runtime_file("status.ini");
    g_autoptr(GKeyFile) devices = load_runtime_file("devices.ini");
    g_autofree char* status_state =
        status ? g_key_file_get_string(status, "vr", "state", NULL) : NULL;
    g_autofree char* status_diagnostic =
        status ? g_key_file_get_string(status, "vr", "diagnostic", NULL) : NULL;
    gint count = devices ? g_key_file_get_integer(devices, "streamer", "device_count", NULL) : 0;
    count = CLAMP(count, 0, 16);
    GVariantBuilder records;
    g_variant_builder_init(&records, G_VARIANT_TYPE("aa{sv}"));
    gboolean session_assigned = FALSE;
    gint emitted = 0;

    for (gint i = 0; i < count; i++) {
        g_autofree char* group = g_strdup_printf("device.%d", i);
        g_autofree char* state = g_key_file_get_string(devices, group, "state", NULL);
        g_autofree char* name = g_key_file_get_string(devices, group, "name", NULL);
        g_autofree char* vendor = g_key_file_get_string(devices, group, "vendor", NULL);
        g_autofree char* device_id = g_key_file_get_string(devices, group, "id", NULL);
        if (!state || !device_id)
            continue;
        gboolean hosts_session = !session_assigned && status_state && headset_session_state(state);
        gboolean running = hosts_session && g_str_equal(status_state, "running");
        if (hosts_session)
            session_assigned = TRUE;
        g_autofree char* diagnostic = g_key_file_get_string(devices, group, "diagnostic", NULL);
        if (hosts_session && status_diagnostic && *status_diagnostic &&
            !g_str_equal(status_state, "absent")) {
            g_free(diagnostic);
            diagnostic = g_strdup(status_diagnostic);
        }

        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_autofree char* id = g_strdup_printf("xr:%d", emitted++);
        g_autofree char* label = (vendor && *vendor && name && !g_str_has_prefix(name, vendor))
                                     ? g_strdup_printf("%s %s", vendor, name)
                                     : g_strdup(name && *name ? name : "OpenXR headset");
        g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&record, "{sv}", "kind", g_variant_new_string("xr"));
        g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(label));
        g_variant_builder_add(&record, "{sv}", "enabled", g_variant_new_boolean(running));
        g_variant_builder_add(&record, "{sv}", "primary", g_variant_new_boolean(FALSE));
        g_variant_builder_add(&record, "{sv}", "state",
                              g_variant_new_string(running ? "running" : state));
        if (diagnostic && *diagnostic)
            g_variant_builder_add(&record, "{sv}", "diagnostic", g_variant_new_string(diagnostic));
        static const char* const copied[] = {"transport", "backend", "vendor"};
        for (guint f = 0; f < G_N_ELEMENTS(copied); f++) {
            g_autofree char* value = g_key_file_get_string(devices, group, copied[f], NULL);
            if (value && *value)
                g_variant_builder_add(&record, "{sv}", copied[f], g_variant_new_string(value));
        }
        g_autofree char* connection = g_key_file_get_string(devices, group, "kind", NULL);
        if (connection && *connection)
            g_variant_builder_add(&record, "{sv}", "connection", g_variant_new_string(connection));
        g_variant_builder_add(&record, "{sv}", "device_id", g_variant_new_string(device_id));
        g_variant_builder_add(&record, "{sv}", "views",
                              running ? status_views(status)
                                      : g_variant_new_array(G_VARIANT_TYPE("a{sv}"), NULL, 0));
        g_variant_builder_add_value(&records, g_variant_builder_end(&record));
    }

    if (count == 0 && status_state && *status_state) {
        GVariantBuilder record;
        g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string("xr:0"));
        g_variant_builder_add(&record, "{sv}", "kind", g_variant_new_string("xr"));
        g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string("OpenXR headset"));
        g_variant_builder_add(&record, "{sv}", "enabled",
                              g_variant_new_boolean(g_str_equal(status_state, "running")));
        g_variant_builder_add(&record, "{sv}", "primary", g_variant_new_boolean(FALSE));
        g_variant_builder_add(&record, "{sv}", "state", g_variant_new_string(status_state));
        if (status_diagnostic && *status_diagnostic)
            g_variant_builder_add(&record, "{sv}", "diagnostic",
                                  g_variant_new_string(status_diagnostic));
        g_variant_builder_add(&record, "{sv}", "views", status_views(status));
        g_variant_builder_add_value(&records, g_variant_builder_end(&record));
    }
    return g_variant_ref_sink(g_variant_builder_end(&records));
}

GVariant* gnoblin_vr_output_record(void) {
    g_autoptr(GVariant) records = gnoblin_vr_output_records();
    if (g_variant_n_children(records) == 0)
        return NULL;
    return g_variant_get_child_value(records, 0);
}
