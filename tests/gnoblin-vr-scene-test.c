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

static GVariant* head_args(double qy, double qw) {
    GVariantBuilder pos, rot, pose, args;
    g_variant_builder_init(&pos, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&pos, "{sv}", "x", g_variant_new_double(0.0));
    g_variant_builder_add(&pos, "{sv}", "y", g_variant_new_double(1.5));
    g_variant_builder_add(&pos, "{sv}", "z", g_variant_new_double(0.0));
    g_variant_builder_init(&rot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&rot, "{sv}", "x", g_variant_new_double(0.0));
    g_variant_builder_add(&rot, "{sv}", "y", g_variant_new_double(qy));
    g_variant_builder_add(&rot, "{sv}", "z", g_variant_new_double(0.0));
    g_variant_builder_add(&rot, "{sv}", "w", g_variant_new_double(qw));
    g_variant_builder_init(&pose, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&pose, "{sv}", "position", g_variant_builder_end(&pos));
    g_variant_builder_add(&pose, "{sv}", "rotation", g_variant_builder_end(&rot));
    g_variant_builder_init(&args, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&args, "{sv}", "pose", g_variant_builder_end(&pose));
    return g_variant_ref_sink(g_variant_builder_end(&args));
}

static GVariant* ray_args(double ox, double oy, double oz, double dx, double dy, double dz) {
    GVariantBuilder origin, direction, args;
    g_variant_builder_init(&origin, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&origin, "{sv}", "x", g_variant_new_double(ox));
    g_variant_builder_add(&origin, "{sv}", "y", g_variant_new_double(oy));
    g_variant_builder_add(&origin, "{sv}", "z", g_variant_new_double(oz));
    g_variant_builder_init(&direction, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&direction, "{sv}", "x", g_variant_new_double(dx));
    g_variant_builder_add(&direction, "{sv}", "y", g_variant_new_double(dy));
    g_variant_builder_add(&direction, "{sv}", "z", g_variant_new_double(dz));
    g_variant_builder_init(&args, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&args, "{sv}", "origin", g_variant_builder_end(&origin));
    g_variant_builder_add(&args, "{sv}", "direction", g_variant_builder_end(&direction));
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

    /* Anchor (0,1.5,-1) plus surface a (0,1.5,-0.5) puts a at (0,3,-1.5), b behind it at z -2.2. */
    g_autoptr(GVariant) ray = ray_args(0.0, 3.0, 0.0, 0.0, 0.0, -1.0);
    g_autoptr(GVariant) hit = gnoblin_vr_scene_call("vr.spatial.hit", ray, &error);
    g_assert_no_error(error);
    gboolean did_hit = FALSE;
    const char* hit_id = NULL;
    double u = 0.0, v = 0.0, distance = 0.0;
    g_assert_true(g_variant_lookup(hit, "hit", "b", &did_hit) && did_hit);
    g_assert_true(g_variant_lookup(hit, "surface_id", "&s", &hit_id));
    g_assert_cmpstr(hit_id, ==, "a");
    g_assert_true(g_variant_lookup(hit, "u", "d", &u) && g_variant_lookup(hit, "v", "d", &v));
    g_assert_true(g_variant_lookup(hit, "distance", "d", &distance));
    g_assert_cmpfloat_with_epsilon(u, 0.5, 1e-9);
    g_assert_cmpfloat_with_epsilon(v, 0.5, 1e-9);
    g_assert_cmpfloat_with_epsilon(distance, 1.5, 1e-9);

    /* Off the edge of the 1.2 m wide panel, and pointing away from it, both miss. */
    g_autoptr(GVariant) wide = ray_args(0.7, 3.0, 0.0, 0.0, 0.0, -1.0);
    g_autoptr(GVariant) miss = gnoblin_vr_scene_call("vr.spatial.hit", wide, &error);
    g_assert_true(g_variant_lookup(miss, "hit", "b", &did_hit) && !did_hit);
    g_autoptr(GVariant) away = ray_args(0.0, 3.0, 0.0, 0.0, 0.0, 1.0);
    g_autoptr(GVariant) behind = gnoblin_vr_scene_call("vr.spatial.hit", away, &error);
    g_assert_true(g_variant_lookup(behind, "hit", "b", &did_hit) && !did_hit);

    /* A head-locked anchor follows the head pose. Head (0,1.5,0), view anchor (0,1.5,-2) and a
     * docked panel at (0,1.5,0) from it put the panel at (0,4.5,-2). */
    gnoblin_vr_scene_reset();
    g_autoptr(GVariant) head = head_args(0.0, 1.0);
    g_assert_nonnull(gnoblin_vr_scene_call("vr.spatial.head", head, &error));
    g_assert_no_error(error);
    GVariantDict view_dict;
    g_autoptr(GVariant) view_base = pose_args("id", "v", NULL, NULL, -2.0, FALSE);
    g_variant_dict_init(&view_dict, view_base);
    g_variant_dict_insert(&view_dict, "space_id", "s", "view");
    g_autoptr(GVariant) view_anchor = g_variant_ref_sink(g_variant_dict_end(&view_dict));
    g_assert_nonnull(gnoblin_vr_scene_call("vr.spatial.anchor", view_anchor, &error));
    g_autoptr(GVariant) docked = pose_args("surface_id", "h", "viewport_id", "v", 0.0, TRUE);
    g_assert_nonnull(gnoblin_vr_scene_call("vr.spatial.attach", docked, &error));
    g_assert_no_error(error);
    g_autoptr(GVariant) view_ray = ray_args(0.0, 4.5, 0.0, 0.0, 0.0, -1.0);
    g_autoptr(GVariant) view_hit = gnoblin_vr_scene_call("vr.spatial.hit", view_ray, &error);
    g_assert_true(g_variant_lookup(view_hit, "hit", "b", &did_hit) && did_hit);
    g_assert_true(g_variant_lookup(view_hit, "distance", "d", &distance));
    g_assert_cmpfloat_with_epsilon(distance, 2.0, 1e-9);

    /* Turning the head 90 degrees swings the panel out of that ray. */
    g_autoptr(GVariant) turned = head_args(0.70710678118654752, 0.70710678118654752);
    g_assert_nonnull(gnoblin_vr_scene_call("vr.spatial.head", turned, &error));
    g_autoptr(GVariant) after_turn = gnoblin_vr_scene_call("vr.spatial.hit", view_ray, &error);
    g_assert_true(g_variant_lookup(after_turn, "hit", "b", &did_hit) && !did_hit);

    gnoblin_vr_scene_reset();
    g_autoptr(GVariant) empty = gnoblin_vr_scene_call("vr.spatial.snapshot", NULL, &error);
    g_autoptr(GVariant) none = g_variant_lookup_value(empty, "surfaces", G_VARIANT_TYPE("aa{sv}"));
    g_assert_cmpuint(g_variant_n_children(none), ==, 0);
    return 0;
}
