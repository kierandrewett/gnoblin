#include "gnoblin-runtime-cache.h"

struct _GnoblinRuntimeCache {
    GVariant* document;
    guint64 settings_revision;
};

typedef enum {
    GNOBLIN_RUNTIME_CACHE_ERROR_INVALID_DOCUMENT,
} GnoblinRuntimeCacheError;

static GQuark gnoblin_runtime_cache_error_quark(void) {
    return g_quark_from_static_string("gnoblin-runtime-cache-error");
}

GnoblinRuntimeCache* gnoblin_runtime_cache_new(void) {
    return g_new0(GnoblinRuntimeCache, 1);
}

void gnoblin_runtime_cache_free(GnoblinRuntimeCache* cache) {
    if (!cache)
        return;
    g_clear_pointer(&cache->document, g_variant_unref);
    g_free(cache);
}

gboolean gnoblin_runtime_cache_replace(GnoblinRuntimeCache* cache, GVariant* document,
                                       guint64 settings_revision, GError** error) {
    g_return_val_if_fail(cache != NULL, FALSE);
    if (!document || !g_variant_is_of_type(document, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_is_normal_form(document)) {
        g_set_error_literal(error, gnoblin_runtime_cache_error_quark(),
                            GNOBLIN_RUNTIME_CACHE_ERROR_INVALID_DOCUMENT,
                            "configuration snapshot must be a normal-form a{sv} variant");
        return FALSE;
    }

    GVariant* replacement = g_variant_ref(document);
    g_clear_pointer(&cache->document, g_variant_unref);
    cache->document = replacement;
    cache->settings_revision = settings_revision;
    return TRUE;
}

GVariant* gnoblin_runtime_cache_get_document(GnoblinRuntimeCache* cache) {
    return cache && cache->document ? g_variant_ref(cache->document) : NULL;
}

guint64 gnoblin_runtime_cache_get_settings_revision(GnoblinRuntimeCache* cache) {
    return cache ? cache->settings_revision : 0;
}

static GVariant* lookup_setting(GnoblinRuntimeCache* cache, const char* section, const char* key) {
    if (!cache || !cache->document || !section || !key)
        return NULL;
    g_autoptr(GVariant) section_value =
        g_variant_lookup_value(cache->document, section, G_VARIANT_TYPE_VARDICT);
    if (!section_value)
        return NULL;
    return g_variant_lookup_value(section_value, key, NULL);
}

gboolean gnoblin_runtime_cache_get_bool(GnoblinRuntimeCache* cache, const char* section,
                                        const char* key, gboolean fallback) {
    g_autoptr(GVariant) value = lookup_setting(cache, section, key);
    return value && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)
               ? g_variant_get_boolean(value)
               : fallback;
}

static gboolean variant_to_int64(GVariant* value, gint64* result) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64)) {
        *result = g_variant_get_int64(value);
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)) {
        *result = g_variant_get_int32(value);
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT32)) {
        *result = g_variant_get_uint32(value);
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_UINT64)) {
        guint64 unsigned_value = g_variant_get_uint64(value);
        if (unsigned_value > G_MAXINT64)
            return FALSE;
        *result = (gint64)unsigned_value;
        return TRUE;
    }
    return FALSE;
}

gint64 gnoblin_runtime_cache_get_int(GnoblinRuntimeCache* cache, const char* section,
                                     const char* key, gint64 fallback) {
    g_autoptr(GVariant) value = lookup_setting(cache, section, key);
    gint64 result;
    return value && variant_to_int64(value, &result) ? result : fallback;
}

double gnoblin_runtime_cache_get_double(GnoblinRuntimeCache* cache, const char* section,
                                        const char* key, double fallback) {
    g_autoptr(GVariant) value = lookup_setting(cache, section, key);
    if (!value)
        return fallback;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        return g_variant_get_double(value);
    gint64 integer;
    return variant_to_int64(value, &integer) ? (double)integer : fallback;
}

char* gnoblin_runtime_cache_get_string(GnoblinRuntimeCache* cache, const char* section,
                                       const char* key) {
    g_autoptr(GVariant) value = lookup_setting(cache, section, key);
    return value && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
               ? g_variant_dup_string(value, NULL)
               : NULL;
}

char** gnoblin_runtime_cache_dup_strv(GnoblinRuntimeCache* cache, const char* section,
                                      const char* key) {
    g_autoptr(GVariant) value = lookup_setting(cache, section, key);
    if (!value || !g_variant_is_of_type(value, G_VARIANT_TYPE_STRING_ARRAY))
        return NULL;
    return g_variant_dup_strv(value, NULL);
}
