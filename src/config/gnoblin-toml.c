/* Shared TOML decoding for Mutter and its introspected shell interface. */
#include "gnoblin-config.h"
#include "tomlc99/toml.h"
#include <stdlib.h>

static GVariant* table_value(toml_table_t* table);

static GVariant* scalar_value(const char* raw) {
    char* string;
    int boolean;
    int64_t integer;
    double number;
    toml_timestamp_t timestamp;
    if (toml_rtos(raw, &string) == 0) {
        GVariant* value = g_variant_new_string(string);
        free(string);
        return value;
    }
    if (toml_rtob(raw, &boolean) == 0)
        return g_variant_new_boolean(boolean);
    if (toml_rtoi(raw, &integer) == 0)
        return g_variant_new_int64(integer);
    if (toml_rtod(raw, &number) == 0)
        return g_variant_new_double(number);
    if (toml_rtots(raw, &timestamp) == 0)
        return g_variant_new_string(raw);
    return NULL;
}

static GVariant* array_value(toml_array_t* array) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE("av"));
    for (int i = 0; i < toml_array_nelem(array); i++) {
        toml_table_t* table = toml_table_at(array, i);
        toml_array_t* child = toml_array_at(array, i);
        GVariant* value = table   ? table_value(table)
                          : child ? array_value(child)
                                  : scalar_value(toml_raw_at(array, i));
        if (!value) {
            g_variant_builder_clear(&builder);
            return NULL;
        }
        g_variant_builder_add(&builder, "v", value);
    }
    return g_variant_builder_end(&builder);
}

static GVariant* table_value(toml_table_t* table) {
    GVariantBuilder builder;
    const char* key;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    for (int i = 0; (key = toml_key_in(table, i)); i++) {
        toml_table_t* child = toml_table_in(table, key);
        toml_array_t* array = toml_array_in(table, key);
        GVariant* value = child   ? table_value(child)
                          : array ? array_value(array)
                                  : scalar_value(toml_raw_in(table, key));
        if (!value) {
            g_variant_builder_clear(&builder);
            return NULL;
        }
        g_variant_builder_add(&builder, "{sv}", key, value);
    }
    return g_variant_builder_end(&builder);
}

GVariant* gnoblin_config_parse_toml(const char* contents, GError** error) {
    g_autofree char* copy = g_strdup(contents);
    char message[256];
    toml_table_t* table;
    GVariant* value;
    if (!g_utf8_validate(contents, -1, NULL)) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "TOML must be UTF-8");
        return NULL;
    }
    table = toml_parse(copy, message, sizeof message);
    if (!table) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, message);
        return NULL;
    }
    if (toml_key_exists(table, "protocols")) {
        toml_table_t* protocols = toml_table_in(table, "protocols");
        const char* key;
        gboolean valid = protocols != NULL;
        for (int i = 0; valid && (key = toml_key_in(protocols, i)); i++)
            valid = toml_bool_in(protocols, key).ok;
        if (!valid) {
            g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                "protocols must be a table of booleans");
            toml_free(table);
            return NULL;
        }
    }
    value = table_value(table);
    toml_free(table);
    if (!value) {
        g_set_error_literal(error, G_FILE_ERROR, G_FILE_ERROR_INVAL, "invalid TOML scalar value");
        return NULL;
    }
    return g_variant_ref_sink(value);
}
