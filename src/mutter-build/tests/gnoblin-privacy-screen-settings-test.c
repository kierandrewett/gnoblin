/*
 * Copyright (C) 2026 Gnoblin contributors
 * This program is free software; you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation; either version 2 of the License, or
 * (at your option) any later version.
 */

#include "config.h"

#include <gio/gio.h>

#include "backends/meta-backend-private.h"
#include "backends/meta-settings-private.h"
#include "meta/meta-context.h"
#include "meta-test/meta-context-test.h"

static MetaContext* test_context;

static GVariant* create_privacy_preferences(gboolean enabled) {
    GVariantBuilder preferences;

    g_variant_builder_init(&preferences, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&preferences, "{sv}", "privacy-screen", g_variant_new_boolean(enabled));

    return g_variant_ref_sink(g_variant_builder_end(&preferences));
}

static void drain_main_context(void) {
    while (g_main_context_iteration(NULL, FALSE))
        ;
}

static void meta_test_gnoblin_privacy_screen_preference(void) {
    MetaBackend* backend = meta_context_get_backend(test_context);
    MetaSettings* meta_settings = meta_backend_get_settings(backend);
    g_autoptr(GSettings) privacy_settings = g_settings_new("org.gnome.desktop.privacy");
    g_autoptr(GVariant) disabled = create_privacy_preferences(FALSE);

    g_settings_set_boolean(privacy_settings, "privacy-screen", TRUE);
    drain_main_context();

    meta_settings_apply_gnoblin_privacy_screen_preferences(meta_settings, disabled);
    g_assert_false(meta_settings_is_privacy_screen_enabled(meta_settings));
    g_assert_true(g_settings_get_boolean(privacy_settings, "privacy-screen"));

    g_settings_set_boolean(privacy_settings, "privacy-screen", FALSE);
    drain_main_context();
    g_settings_set_boolean(privacy_settings, "privacy-screen", TRUE);
    drain_main_context();
    g_assert_false(meta_settings_is_privacy_screen_enabled(meta_settings));

    g_settings_set_boolean(privacy_settings, "privacy-screen", FALSE);
    drain_main_context();
    meta_settings_set_privacy_screen_enabled(meta_settings, TRUE);
    g_assert_true(meta_settings_is_privacy_screen_enabled(meta_settings));
    g_assert_false(g_settings_get_boolean(privacy_settings, "privacy-screen"));

    meta_settings_apply_gnoblin_privacy_screen_preferences(meta_settings, NULL);
    g_assert_false(meta_settings_is_privacy_screen_enabled(meta_settings));

    g_settings_set_boolean(privacy_settings, "privacy-screen", TRUE);
    drain_main_context();
    g_assert_true(meta_settings_is_privacy_screen_enabled(meta_settings));
}

static void meta_test_gnoblin_privacy_screen_runtime_override(void) {
    MetaBackend* backend = meta_context_get_backend(test_context);
    MetaSettings* meta_settings = meta_backend_get_settings(backend);
    g_autoptr(GSettings) privacy_settings = g_settings_new("org.gnome.desktop.privacy");
    g_autoptr(GVariant) disabled = create_privacy_preferences(FALSE);

    meta_settings_apply_gnoblin_privacy_screen_runtime_override(meta_settings, FALSE, FALSE);
    meta_settings_apply_gnoblin_privacy_screen_preferences(meta_settings, NULL);
    g_settings_set_boolean(privacy_settings, "privacy-screen", TRUE);
    drain_main_context();

    meta_settings_apply_gnoblin_privacy_screen_preferences(meta_settings, disabled);
    g_assert_false(meta_settings_is_privacy_screen_enabled(meta_settings));

    meta_settings_apply_gnoblin_privacy_screen_runtime_override(meta_settings, TRUE, TRUE);
    g_assert_true(meta_settings_is_privacy_screen_enabled(meta_settings));
    g_assert_true(g_settings_get_boolean(privacy_settings, "privacy-screen"));

    g_settings_set_boolean(privacy_settings, "privacy-screen", FALSE);
    drain_main_context();
    g_assert_true(meta_settings_is_privacy_screen_enabled(meta_settings));

    meta_settings_apply_gnoblin_privacy_screen_runtime_override(meta_settings, FALSE, FALSE);
    g_assert_false(meta_settings_is_privacy_screen_enabled(meta_settings));
    g_assert_false(g_settings_get_boolean(privacy_settings, "privacy-screen"));

    meta_settings_apply_gnoblin_privacy_screen_preferences(meta_settings, NULL);
    g_assert_false(meta_settings_is_privacy_screen_enabled(meta_settings));
    g_settings_set_boolean(privacy_settings, "privacy-screen", TRUE);
    drain_main_context();
    g_assert_true(meta_settings_is_privacy_screen_enabled(meta_settings));
}

int main(int argc, char** argv) {
    g_autoptr(MetaContext) context = NULL;
    g_autoptr(GError) error = NULL;
    int ret;

    g_setenv("GSETTINGS_BACKEND", "memory", TRUE);

    context = meta_create_test_context(META_CONTEXT_TEST_TYPE_TEST, META_CONTEXT_TEST_FLAG_NO_X11);
    g_assert_true(meta_context_configure(context, &argc, &argv, &error));

    test_context = context;
    g_test_add_func("/settings/gnoblin-privacy-screen-preference",
                    meta_test_gnoblin_privacy_screen_preference);
    g_test_add_func("/settings/gnoblin-privacy-screen-runtime-override",
                    meta_test_gnoblin_privacy_screen_runtime_override);

    ret = meta_context_test_run_tests(META_CONTEXT_TEST(context), META_TEST_RUN_FLAG_CAN_SKIP);

    return ret;
}
