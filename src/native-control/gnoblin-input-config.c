#include "gnoblin-input-config.h"

static gboolean input_number(GVariant* value, double* number) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE)) {
        *number = g_variant_get_double(value);
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64)) {
        *number = (double)g_variant_get_int64(value);
        return TRUE;
    }
    /* Normalisation writes int32 and uint32 values (drag-threshold, double-click-time, repeat delay). A replacement
     * worker normalises Mutter's accepted document again, so these must read back unchanged. Before, an int32 read
     * back as 0, Mutter saw a different document and rejected the replacement worker. */
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)) {
        *number = (double)g_variant_get_int32(value);
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32)) {
        *number = (double)g_variant_get_uint32(value);
        return TRUE;
    }
    return FALSE;
}

/* The Lua layer can hand over any shape. Values that do not have the expected shape pass through unchanged, so the
 * validation in the compositor reports them by name. Iterating a value of the wrong type would abort the worker. */
static gboolean input_is_table(GVariant* value) {
    return g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT);
}

static gboolean input_is_empty_table(GVariant* value) {
    return input_is_table(value) && g_variant_n_children(value) == 0;
}

static gboolean input_is_string_list(GVariant* value) {
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")))
        return FALSE;
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING))
            return FALSE;
    }
    return TRUE;
}

static gboolean input_is_number_list(GVariant* value) {
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")))
        return FALSE;
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        g_autoptr(GVariant) item = g_variant_get_variant(boxed);
        double number;
        if (!input_number(item, &number))
            return FALSE;
    }
    return TRUE;
}

static GVariant* normalize_input_curve(GVariant* curve) {
    if (!input_is_table(curve))
        return g_variant_ref(curve);
    GVariantBuilder normalized;
    g_variant_builder_init(&normalized, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, curve);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        if (g_str_equal(key, "step")) {
            double step = 0;
            input_number(value, &step);
            g_variant_builder_add(&normalized, "{sv}", key, g_variant_new_double(step));
        } else if (!input_is_number_list(value)) {
            g_variant_builder_add(&normalized, "{sv}", key, value);
        } else {
            GVariantBuilder points_builder;
            g_variant_builder_init(&points_builder, G_VARIANT_TYPE("ad"));
            GVariantIter points_iter;
            GVariant* item;
            g_variant_iter_init(&points_iter, value);
            while ((item = g_variant_iter_next_value(&points_iter))) {
                g_autoptr(GVariant) point_value = g_variant_get_variant(item);
                double point = 0;
                input_number(point_value, &point);
                g_variant_builder_add(&points_builder, "d", point);
                g_variant_unref(item);
            }
            g_variant_builder_add(&normalized, "{sv}", key, g_variant_builder_end(&points_builder));
        }
        g_variant_unref(value);
    }
    return g_variant_ref_sink(g_variant_builder_end(&normalized));
}

static GVariant* normalize_tablet_area(GVariant* area) {
    if (!g_variant_is_of_type(area, G_VARIANT_TYPE("av")) || g_variant_n_children(area) != 4)
        return g_variant_ref(area);

    double values[4];
    for (gsize i = 0; i < G_N_ELEMENTS(values); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(area, i);
        g_autoptr(GVariant) value = g_variant_get_variant(boxed);
        if (!input_number(value, &values[i]))
            return g_variant_ref(area);
    }
    return g_variant_ref_sink(g_variant_new_fixed_array(G_VARIANT_TYPE_DOUBLE, values,
                                                        G_N_ELEMENTS(values), sizeof(double)));
}

static GVariant* normalize_input_fields(GVariant* fields) {
    if (!input_is_table(fields))
        return g_variant_ref(fields);
    GVariantBuilder normalized;
    g_variant_builder_init(&normalized, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, fields);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(value, NULL), "inherit")) {
            g_variant_unref(value);
            continue;
        }
        if (g_str_equal(key, "speed") || g_str_equal(key, "scroll-speed")) {
            double number = 0;
            input_number(value, &number);
            g_variant_builder_add(&normalized, "{sv}", key, g_variant_new_double(number));
        } else if (g_str_equal(key, "double-click-time") || g_str_equal(key, "drag-threshold") ||
                   g_str_equal(key, "scroll-wheel-emulation-button")) {
            double number = 0;
            input_number(value, &number);
            g_variant_builder_add(&normalized, "{sv}", key, g_variant_new_int32((gint32)number));
        } else if (g_str_equal(key, "delay") || g_str_equal(key, "repeat-interval") ||
                   g_str_equal(key, "disable-while-typing-timeout")) {
            double number = 0;
            input_number(value, &number);
            g_variant_builder_add(&normalized, "{sv}", key, g_variant_new_uint32((guint32)number));
        } else if (g_str_equal(key, "xkb-options") && !input_is_string_list(value)) {
            g_variant_builder_add(&normalized, "{sv}", key, value);
        } else if (g_str_equal(key, "xkb-options")) {
            GVariantBuilder options_builder;
            GVariantIter options_iter;
            GVariant* boxed;
            g_variant_builder_init(&options_builder, G_VARIANT_TYPE_STRING_ARRAY);
            g_variant_iter_init(&options_iter, value);
            while ((boxed = g_variant_iter_next_value(&options_iter))) {
                g_autoptr(GVariant) option = g_variant_get_variant(boxed);
                g_variant_builder_add(&options_builder, "s", g_variant_get_string(option, NULL));
                g_variant_unref(boxed);
            }
            g_variant_builder_add(&normalized, "{sv}", key,
                                  g_variant_builder_end(&options_builder));
        } else if (g_str_equal(key, "accel-curve")) {
            g_autoptr(GVariant) curve = normalize_input_curve(value);
            g_variant_builder_add(&normalized, "{sv}", key, curve);
        } else if (g_str_equal(key, "area")) {
            g_autoptr(GVariant) area = normalize_tablet_area(value);
            g_variant_builder_add(&normalized, "{sv}", key, area);
        } else {
            g_variant_builder_add(&normalized, "{sv}", key, value);
        }
        g_variant_unref(value);
    }
    return g_variant_ref_sink(g_variant_builder_end(&normalized));
}

static GVariant* normalize_input_devices(GVariant* devices) {
    if (!input_is_table(devices))
        return g_variant_ref(devices);
    GVariantBuilder normalized;
    g_variant_builder_init(&normalized, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* device;
    GVariant* fields;
    g_variant_iter_init(&iter, devices);
    while (g_variant_iter_next(&iter, "{&sv}", &device, &fields)) {
        g_autoptr(GVariant) normalized_fields = normalize_input_fields(fields);
        if (!input_is_empty_table(normalized_fields))
            g_variant_builder_add(&normalized, "{sv}", device, normalized_fields);
        g_variant_unref(fields);
    }
    return g_variant_ref_sink(g_variant_builder_end(&normalized));
}

GVariant* gnoblin_native_input_normalize(GVariant* input) {
    g_return_val_if_fail(input && g_variant_is_of_type(input, G_VARIANT_TYPE_VARDICT), NULL);
    GVariantBuilder normalized;
    g_variant_builder_init(&normalized, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* group;
    GVariant* value;
    g_variant_iter_init(&iter, input);
    while (g_variant_iter_next(&iter, "{&sv}", &group, &value)) {
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(value, NULL), "inherit")) {
            g_variant_unref(value);
            continue;
        }
        if (g_str_equal(group, "orientation-lock")) {
            g_variant_builder_add(&normalized, "{sv}", group, value);
        } else {
            g_autoptr(GVariant) normalized_group =
                (g_str_equal(group, "tablets") || g_str_equal(group, "styluses"))
                    ? normalize_input_devices(value)
                    : normalize_input_fields(value);
            if (!input_is_empty_table(normalized_group))
                g_variant_builder_add(&normalized, "{sv}", group, normalized_group);
        }
        g_variant_unref(value);
    }
    return g_variant_ref_sink(g_variant_builder_end(&normalized));
}

GVariant* gnoblin_native_input_config_normalize(GVariant* document) {
    g_return_val_if_fail(document && g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT), NULL);
    g_autoptr(GVariant) input = g_variant_lookup_value(document, "input", NULL);
    if (!input)
        return g_variant_ref(document);

    g_autoptr(GVariant) normalized_input = gnoblin_native_input_normalize(input);
    GVariantBuilder normalized_document;
    g_variant_builder_init(&normalized_document, G_VARIANT_TYPE_VARDICT);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, document);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        if (g_str_equal(key, "input"))
            g_variant_builder_add(&normalized_document, "{sv}", key, normalized_input);
        else
            g_variant_builder_add(&normalized_document, "{sv}", key, value);
        g_variant_unref(value);
    }
    return g_variant_ref_sink(g_variant_builder_end(&normalized_document));
}
