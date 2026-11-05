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

static GVariant* create_global_scale_preferences(GVariant* global_scale) {
    GVariantBuilder preferences;

    g_variant_builder_init(&preferences, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&preferences, "{sv}", "global-scale", global_scale);

    return g_variant_ref_sink(g_variant_builder_end(&preferences));
}

static void drain_main_context(void) {
    while (g_main_context_iteration(NULL, FALSE))
        ;
}

static void meta_test_gnoblin_global_monitor_scale(void) {
    MetaBackend* backend = meta_context_get_backend(test_context);
    MetaSettings* meta_settings = meta_backend_get_settings(backend);
    g_autoptr(GSettings) interface_settings = g_settings_new("org.gnome.desktop.interface");
    g_autoptr(GVariant) forced_scale = create_global_scale_preferences(g_variant_new_int64(3));
    g_autoptr(GVariant) automatic_scale = create_global_scale_preferences(g_variant_new_int64(0));
    g_autoptr(GVariant) inherit_scale =
        create_global_scale_preferences(g_variant_new_string("inherit"));
    int scale;

    g_settings_set_uint(interface_settings, "scaling-factor", 2);
    drain_main_context();
    g_assert_true(meta_settings_get_global_scaling_factor(meta_settings, &scale));
    g_assert_cmpint(scale, ==, 2);

    meta_settings_apply_gnoblin_monitor_preferences(meta_settings, forced_scale);
    g_assert_true(meta_settings_get_global_scaling_factor(meta_settings, &scale));
    g_assert_cmpint(scale, ==, 3);
    g_assert_cmpuint(g_settings_get_uint(interface_settings, "scaling-factor"), ==, 2);

    g_settings_set_uint(interface_settings, "scaling-factor", 1);
    drain_main_context();
    g_assert_true(meta_settings_get_global_scaling_factor(meta_settings, &scale));
    g_assert_cmpint(scale, ==, 3);

    meta_settings_apply_gnoblin_monitor_preferences(meta_settings, inherit_scale);
    g_assert_true(meta_settings_get_global_scaling_factor(meta_settings, &scale));
    g_assert_cmpint(scale, ==, 1);

    g_settings_set_uint(interface_settings, "scaling-factor", 4);
    drain_main_context();
    g_assert_true(meta_settings_get_global_scaling_factor(meta_settings, &scale));
    g_assert_cmpint(scale, ==, 4);

    meta_settings_apply_gnoblin_monitor_preferences(meta_settings, automatic_scale);
    g_assert_false(meta_settings_get_global_scaling_factor(meta_settings, &scale));
}

int main(int argc, char** argv) {
    g_autoptr(MetaContext) context = NULL;
    g_autoptr(GError) error = NULL;
    int ret;

    g_setenv("GSETTINGS_BACKEND", "memory", TRUE);

    context = meta_create_test_context(META_CONTEXT_TEST_TYPE_TEST, META_CONTEXT_TEST_FLAG_NO_X11);
    g_assert_true(meta_context_configure(context, &argc, &argv, &error));

    test_context = context;
    g_test_add_func("/settings/gnoblin-global-monitor-scale",
                    meta_test_gnoblin_global_monitor_scale);

    ret = meta_context_test_run_tests(META_CONTEXT_TEST(context), META_TEST_RUN_FLAG_CAN_SKIP);

    return ret;
}
