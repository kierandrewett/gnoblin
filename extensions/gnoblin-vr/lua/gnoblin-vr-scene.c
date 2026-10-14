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

GVariant* gnoblin_vr_scene_call(const char* method, GVariant* arguments, GError** error) {
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
        g_variant_builder_add(&result, "{sv}", "anchors", g_variant_builder_end(&anchor_list));
        g_variant_builder_add(&result, "{sv}", "surfaces", g_variant_builder_end(&surface_list));
        return g_variant_ref_sink(g_variant_builder_end(&result));
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
