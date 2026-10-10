/*
 * Read-only native snapshot of the configuration owned by the Gnoblin Lua
 * supervisor. This module deliberately contains no Lua runtime or matcher.
 */
#pragma once

#include <glib.h>

G_BEGIN_DECLS

typedef struct _GnoblinRuntimeCache GnoblinRuntimeCache;

GnoblinRuntimeCache* gnoblin_runtime_cache_new(void);
void gnoblin_runtime_cache_free(GnoblinRuntimeCache* cache);

/*
 * Atomically replace the snapshot imported from a CONFIG packet. `document`
 * must be a normal-form GVariant of type a{sv}; each section used by the
 * setting accessors must itself be an a{sv}. Array-valued data such as
 * `window-rules` is retained unchanged and available through get_document().
 * The cache takes its own reference. `settings_revision` is the Lua owner's
 * revision and is not interpreted by this read-only cache.
 */
gboolean gnoblin_runtime_cache_replace(GnoblinRuntimeCache* cache, GVariant* document,
                                       guint64 settings_revision, GError** error);

/* Returns a new reference, or NULL before the first successful replacement. */
GVariant* gnoblin_runtime_cache_get_document(GnoblinRuntimeCache* cache);
guint64 gnoblin_runtime_cache_get_settings_revision(GnoblinRuntimeCache* cache);

/* Look up a typed value in a top-level section a{sv}; missing/wrong types use fallback. */
gboolean gnoblin_runtime_cache_get_bool(GnoblinRuntimeCache* cache, const char* section,
                                        const char* key, gboolean fallback);
gint64 gnoblin_runtime_cache_get_int(GnoblinRuntimeCache* cache, const char* section,
                                     const char* key, gint64 fallback);
double gnoblin_runtime_cache_get_double(GnoblinRuntimeCache* cache, const char* section,
                                        const char* key, double fallback);
/* Caller owns the returned string, or receives NULL when absent/wrong type. */
char* gnoblin_runtime_cache_get_string(GnoblinRuntimeCache* cache, const char* section,
                                       const char* key);
/* Caller owns the returned strv; only an `as` value is accepted. */
char** gnoblin_runtime_cache_dup_strv(GnoblinRuntimeCache* cache, const char* section,
                                      const char* key);

G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinRuntimeCache, gnoblin_runtime_cache_free)

G_END_DECLS
