#include <gio/gio.h>

#include "gnoblin-vr-scene.h"

static GVariant* pose_args(const char* surface_key, const char* surface_id, const char* parent_key,
                           const char* parent_id, double z, gboolean with_size) {
    GVariantBuilder pos, rot, pose, args, size;
    g_variant_builder_init(&pos, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&pos, "{sv}", "x", g_variant_new_double(0.0));
    g_variant_builder_add(&pos, "{sv}", "y", g_variant_new_double(1.5));
    g_variant_builder_add(&pos, "{sv}", "z", g_variant_new_double(z));
    g_variant_builder_init(&rot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&rot, "{sv}", "x", g_variant_new_double(0.0));
    g_variant_builder_add(&rot, "{sv}", "y", g_variant_new_double(0.0));
    g_variant_builder_add(&rot, "{sv}", "z", g_variant_new_double(0.0));
    g_variant_builder_add(&rot, "{sv}", "w", g_variant_new_double(1.0));
    g_variant_builder_init(&pose, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&pose, "{sv}", "position", g_variant_builder_end(&pos));
    g_variant_builder_add(&pose, "{sv}", "rotation", g_variant_builder_end(&rot));
    g_variant_builder_init(&args, G_VARIANT_TYPE_VARDICT);
    if (surface_key)
        g_variant_builder_add(&args, "{sv}", surface_key, g_variant_new_string(surface_id));
    if (parent_key)
        g_variant_builder_add(&args, "{sv}", parent_key, g_variant_new_string(parent_id));
    g_variant_builder_add(&args, "{sv}", "pose", g_variant_builder_end(&pose));
    if (with_size) {
        g_variant_builder_init(&size, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&size, "{sv}", "width", g_variant_new_double(1.2));
        g_variant_builder_add(&size, "{sv}", "height", g_variant_new_int64(1));
        g_variant_builder_add(&args, "{sv}", "size", g_variant_builder_end(&size));
    }
    return g_variant_ref_sink(g_variant_builder_end(&args));
}

static guint64 rank_of(const char* id) {
    GError* error = NULL;
    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&b, "{sv}", "surface_id", g_variant_new_string(id));
    g_autoptr(GVariant) args = g_variant_ref_sink(g_variant_builder_end(&b));
    g_autoptr(GVariant) record = gnoblin_vr_scene_call("vr.spatial.surface", args, &error);
    g_assert_no_error(error);
    guint64 rank = 0;
    g_assert_true(g_variant_lookup(record, "rank", "t", &rank));
    return rank;
}

int main(void) {
    GError* error = NULL;

    g_autoptr(GVariant) anchor = pose_args("id", "world-origin", NULL, NULL, -1.0, FALSE);
    g_autoptr(GVariant) r1 = gnoblin_vr_scene_call("vr.spatial.anchor", anchor, &error);
    g_assert_no_error(error);
    g_assert_nonnull(r1);

    /* Placement requires an existing anchor. */
    g_autoptr(GVariant) orphan = pose_args("surface_id", "a", "anchor_id", "nowhere", -1.0, TRUE);
    g_assert_null(gnoblin_vr_scene_call("vr.spatial.place", orphan, &error));
    g_assert_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND);
    g_clear_error(&error);

    /* Mixed double and integer sizes are accepted; a zero size is not. */
    g_autoptr(GVariant) place_a =
        pose_args("surface_id", "a", "anchor_id", "world-origin", -1.0, TRUE);
    g_autoptr(GVariant) place_b =
        pose_args("surface_id", "b", "anchor_id", "world-origin", -1.2, TRUE);
    g_autoptr(GVariant) ok_a = gnoblin_vr_scene_call("vr.spatial.place", place_a, &error);
    g_assert_no_error(error);
    g_autoptr(GVariant) ok_b = gnoblin_vr_scene_call("vr.spatial.place", place_b, &error);
    g_assert_no_error(error);
    g_assert_nonnull(ok_a);
    g_assert_nonnull(ok_b);
    g_assert_cmpuint(rank_of("b"), >, rank_of("a"));

    /* Raising a puts it above b. */
    GVariantBuilder raise;
    g_variant_builder_init(&raise, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&raise, "{sv}", "surface_id", g_variant_new_string("a"));
    g_autoptr(GVariant) raise_args = g_variant_ref_sink(g_variant_builder_end(&raise));
    g_autoptr(GVariant) raised = gnoblin_vr_scene_call("vr.spatial.raise", raise_args, &error);
    g_assert_no_error(error);
    g_assert_nonnull(raised);
    g_assert_cmpuint(rank_of("a"), >, rank_of("b"));

    /* Moving a surface updates its pose. */
    g_autoptr(GVariant) move = pose_args("surface_id", "a", NULL, NULL, -0.5, FALSE);
    g_autoptr(GVariant) moved = gnoblin_vr_scene_call("vr.spatial.update_pose", move, &error);
    g_assert_no_error(error);
    g_assert_nonnull(moved);

    g_autoptr(GVariant) snapshot = gnoblin_vr_scene_call("vr.spatial.snapshot", NULL, &error);
    g_assert_no_error(error);
    g_autoptr(GVariant) surfaces =
        g_variant_lookup_value(snapshot, "surfaces", G_VARIANT_TYPE("aa{sv}"));
    g_assert_cmpuint(g_variant_n_children(surfaces), ==, 2);

    gnoblin_vr_scene_reset();
    g_autoptr(GVariant) empty = gnoblin_vr_scene_call("vr.spatial.snapshot", NULL, &error);
    g_autoptr(GVariant) none = g_variant_lookup_value(empty, "surfaces", G_VARIANT_TYPE("aa{sv}"));
    g_assert_cmpuint(g_variant_n_children(none), ==, 0);
    return 0;
}
