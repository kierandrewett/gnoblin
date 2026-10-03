#include <glib.h>

#include "../src/native-control/gnoblin-runtime-cache.h"

static GVariant* make_document(gboolean enabled, gint64 count, double scale, const char* label,
                               const char* const* names) {
    GVariantBuilder settings;
    GVariantBuilder document;

    g_variant_builder_init(&settings, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&settings, "{sv}", "enabled", g_variant_new_boolean(enabled));
    g_variant_builder_add(&settings, "{sv}", "count", g_variant_new_int64(count));
    g_variant_builder_add(&settings, "{sv}", "scale", g_variant_new_double(scale));
    g_variant_builder_add(&settings, "{sv}", "label", g_variant_new_string(label));
    g_variant_builder_add(&settings, "{sv}", "names", g_variant_new_strv(names, -1));

    g_variant_builder_init(&document, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&document, "{sv}", "test", g_variant_builder_end(&settings));
    g_variant_builder_add(&document, "{sv}", "window-rules",
                          g_variant_new_array(G_VARIANT_TYPE_VARDICT, NULL, 0));
    return g_variant_ref_sink(g_variant_builder_end(&document));
}

static void test_initial_state_and_defaults(void) {
    g_autoptr(GnoblinRuntimeCache) cache = gnoblin_runtime_cache_new();

    g_assert_null(gnoblin_runtime_cache_get_document(cache));
    g_assert_cmpuint(gnoblin_runtime_cache_get_settings_revision(cache), ==, 0);
    g_assert_false(gnoblin_runtime_cache_get_bool(cache, "test", "enabled", FALSE));
    g_assert_true(gnoblin_runtime_cache_get_bool(cache, "test", "missing", TRUE));
    g_assert_cmpint(gnoblin_runtime_cache_get_int(cache, "test", "count", -7), ==, -7);
    g_assert_cmpfloat(gnoblin_runtime_cache_get_double(cache, "test", "scale", 2.5), ==, 2.5);
    g_assert_null(gnoblin_runtime_cache_get_string(cache, "test", "label"));
    g_assert_null(gnoblin_runtime_cache_dup_strv(cache, "test", "names"));
}

static void test_typed_accessors_and_snapshot(void) {
    const char* names[] = {"alpha", "beta", NULL};
    g_autoptr(GnoblinRuntimeCache) cache = gnoblin_runtime_cache_new();
    g_autoptr(GVariant) document = make_document(TRUE, 42, 0.75, "first", names);
    g_autoptr(GError) error = NULL;

    g_assert_true(gnoblin_runtime_cache_replace(cache, document, 17, &error));
    g_assert_no_error(error);
    g_assert_cmpuint(gnoblin_runtime_cache_get_settings_revision(cache), ==, 17);
    g_assert_true(gnoblin_runtime_cache_get_bool(cache, "test", "enabled", FALSE));
    g_assert_cmpint(gnoblin_runtime_cache_get_int(cache, "test", "count", 0), ==, 42);
    g_assert_cmpfloat(gnoblin_runtime_cache_get_double(cache, "test", "scale", 0.0), ==, 0.75);
    g_autofree char* label = gnoblin_runtime_cache_get_string(cache, "test", "label");
    g_assert_cmpstr(label, ==, "first");
    g_auto(GStrv) actual_names = gnoblin_runtime_cache_dup_strv(cache, "test", "names");
    g_assert_nonnull(actual_names);
    g_assert_cmpstr(actual_names[0], ==, "alpha");
    g_assert_cmpstr(actual_names[1], ==, "beta");
    g_assert_null(actual_names[2]);

    g_autoptr(GVariant) snapshot = gnoblin_runtime_cache_get_document(cache);
    g_assert_nonnull(snapshot);
    g_assert_true(g_variant_equal(snapshot, document));
    g_autoptr(GVariant) rules =
        g_variant_lookup_value(snapshot, "window-rules", G_VARIANT_TYPE("aa{sv}"));
    g_assert_nonnull(rules);
    g_assert_cmpuint(g_variant_n_children(rules), ==, 0);
}

static void test_replacement_and_revision(void) {
    const char* first_names[] = {"old", NULL};
    const char* next_names[] = {"new", NULL};
    g_autoptr(GnoblinRuntimeCache) cache = gnoblin_runtime_cache_new();
    g_autoptr(GVariant) first = make_document(FALSE, 1, 1.0, "old", first_names);
    g_autoptr(GVariant) next = make_document(TRUE, 2, 0.5, "new", next_names);
    g_autoptr(GError) error = NULL;

    g_assert_true(gnoblin_runtime_cache_replace(cache, first, 1, &error));
    g_assert_no_error(error);
    g_assert_true(gnoblin_runtime_cache_replace(cache, next, 2, &error));
    g_assert_no_error(error);
    g_assert_cmpuint(gnoblin_runtime_cache_get_settings_revision(cache), ==, 2);
    g_assert_true(gnoblin_runtime_cache_get_bool(cache, "test", "enabled", FALSE));
    g_autofree char* label = gnoblin_runtime_cache_get_string(cache, "test", "label");
    g_assert_cmpstr(label, ==, "new");
    g_autoptr(GVariant) snapshot = gnoblin_runtime_cache_get_document(cache);
    g_assert_true(g_variant_equal(snapshot, next));
}

static void test_invalid_document_preserves_snapshot(void) {
    const char* names[] = {"kept", NULL};
    g_autoptr(GnoblinRuntimeCache) cache = gnoblin_runtime_cache_new();
    g_autoptr(GVariant) valid = make_document(TRUE, 9, 1.25, "kept", names);
    g_autoptr(GVariant) invalid = g_variant_ref_sink(g_variant_new_string("not a document"));
    g_autoptr(GError) error = NULL;

    g_assert_true(gnoblin_runtime_cache_replace(cache, valid, 23, &error));
    g_assert_no_error(error);
    g_assert_false(gnoblin_runtime_cache_replace(cache, invalid, 24, &error));
    g_assert_error(error, g_quark_from_static_string("gnoblin-runtime-cache-error"), 0);
    g_clear_error(&error);

    g_assert_cmpuint(gnoblin_runtime_cache_get_settings_revision(cache), ==, 23);
    g_autoptr(GVariant) snapshot = gnoblin_runtime_cache_get_document(cache);
    g_assert_true(g_variant_equal(snapshot, valid));
    g_assert_true(gnoblin_runtime_cache_get_bool(cache, "test", "enabled", FALSE));
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/runtime-cache/initial-state-and-defaults", test_initial_state_and_defaults);
    g_test_add_func("/runtime-cache/typed-accessors-and-snapshot",
                    test_typed_accessors_and_snapshot);
    g_test_add_func("/runtime-cache/replacement-and-revision", test_replacement_and_revision);
    g_test_add_func("/runtime-cache/invalid-document-preserves-snapshot",
                    test_invalid_document_preserves_snapshot);
    return g_test_run();
}
