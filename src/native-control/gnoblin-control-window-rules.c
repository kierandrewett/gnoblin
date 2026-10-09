/* Window rules: rule matching and application, frame style and corner values, the corner
 * toolkit probe, window shadows, shader files and the effect glue that rules drive. The
 * public entry points are the gnoblin_control_apply_*_window_rules wrappers; the rest stays
 * file-local. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include <fcntl.h>
#include <glib/gstdio.h>
#include <math.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "compositor/meta-window-actor-private.h"
#include "compositor/meta-window-actor-x11.h"
#include "compositor/meta-window-actor-wayland.h"
#include "compositor/meta-gnoblin-window-effects.h"
#include "core/window-private.h"
#include "wayland/gnoblin-lua-pattern.h"
#include "wayland/meta-gnoblin-window-frame.h"
#include "wayland/meta-wayland-layer-shell.h"

#define MAX_CORNER_TOOLKIT_CACHE_ENTRIES 256
#define MAX_CORNER_TOOLKIT_MAPS_BYTES (4 * 1024 * 1024)

typedef struct _NativeWindowShaderFile NativeWindowShaderFile;

struct _NativeCornerToolkitCache {
    gint ref_count;
    GnoblinNativeControl* control;
    GHashTable* entries;
    guint pending_count;
};

struct _NativeWindowShaderFile {
    GnoblinNativeControl* control;
    char* path;
    char* source;
    GFileMonitor* monitor;
    guint reload_timeout_id;
};

static void native_apply_window_rules(GnoblinNativeControl* control, MetaWindow* window);
static void native_apply_all_window_rules(GnoblinNativeControl* control);

static gboolean native_rule_get_number(GVariant* record, const char* key, double* number) {
    g_autoptr(GVariant) value = g_variant_lookup_value(record, key, NULL);
    if (!value)
        return FALSE;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
        *number = g_variant_get_double(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
        *number = (double)g_variant_get_int64(value);
    else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
        *number = (double)g_variant_get_int32(value);
    else
        return FALSE;
    return isfinite(*number);
}

static GVariant* native_rule_lookup_alias(GVariant* record, const char* key,
                                          const char* alternate_key) {
    GVariant* value = g_variant_lookup_value(record, key, NULL);
    if (!value && alternate_key)
        value = g_variant_lookup_value(record, alternate_key, NULL);
    if (value && g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        GVariant* unboxed = g_variant_get_variant(value);
        g_variant_unref(value);
        return unboxed;
    }
    return value;
}

static gboolean native_frame_read_extents(GVariant* frame, const char* key, int extents[4]) {
    g_autoptr(GVariant) values = native_rule_lookup_alias(frame, key, NULL);
    if (!values || !g_variant_is_of_type(values, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(values) != 4)
        return FALSE;

    int parsed[4];
    for (gsize i = 0; i < G_N_ELEMENTS(parsed); i++) {
        g_autoptr(GVariant) item = g_variant_get_child_value(values, i);
        if (g_variant_is_of_type(item, G_VARIANT_TYPE_VARIANT)) {
            GVariant* unboxed = g_variant_get_variant(item);
            g_variant_unref(g_steal_pointer(&item));
            item = unboxed;
        }
        if (g_variant_is_of_type(item, G_VARIANT_TYPE_INT32))
            parsed[i] = g_variant_get_int32(item);
        else if (g_variant_is_of_type(item, G_VARIANT_TYPE_INT64)) {
            gint64 number = g_variant_get_int64(item);
            if (number > G_MAXINT || number < G_MININT)
                return FALSE;
            parsed[i] = (int)number;
        } else {
            return FALSE;
        }
        if (parsed[i] < 0 || parsed[i] > 256)
            return FALSE;
    }

    memcpy(extents, parsed, sizeof(parsed));
    return TRUE;
}

static void native_frame_style_merge_string(GVariantDict* style, GVariant* frame, const char* key,
                                            const char* alternate_key, const char* style_key) {
    g_autoptr(GVariant) value = native_rule_lookup_alias(frame, key, alternate_key);
    if (value && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        g_variant_dict_insert_value(style, style_key, value);
}

static void native_frame_style_merge_buttons(GVariantDict* style, GVariant* frame) {
    g_autoptr(GVariant) layout = native_rule_lookup_alias(frame, "button_layout", "button-layout");
    if (!layout || !g_variant_is_of_type(layout, G_VARIANT_TYPE("av")))
        return;

    GVariantBuilder values;
    g_variant_builder_init(&values, G_VARIANT_TYPE_STRING_ARRAY);
    gboolean valid = TRUE;
    for (gsize i = 0; i < g_variant_n_children(layout); i++) {
        g_autoptr(GVariant) item = g_variant_get_child_value(layout, i);
        if (g_variant_is_of_type(item, G_VARIANT_TYPE_VARIANT)) {
            GVariant* unboxed = g_variant_get_variant(item);
            g_variant_unref(g_steal_pointer(&item));
            item = unboxed;
        }
        if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING)) {
            valid = FALSE;
            break;
        }
        g_variant_builder_add(&values, "s", g_variant_get_string(item, NULL));
    }

    if (valid) {
        g_autoptr(GVariant) string_array = g_variant_ref_sink(g_variant_builder_end(&values));
        g_variant_dict_insert_value(style, "button-layout", string_array);
    } else {
        g_variant_builder_clear(&values);
    }
}

static int native_frame_policy_from_name(const char* mode) {
    if (g_strcmp0(mode, "auto") == 0)
        return 1;
    if (g_strcmp0(mode, "prefer-server") == 0)
        return 2;
    if (g_strcmp0(mode, "replace") == 0)
        return 3;
    return 0;
}

static gboolean native_rule_get_padding(GVariant* record, double padding[4]) {
    g_autoptr(GVariant) values = g_variant_lookup_value(record, "padding", G_VARIANT_TYPE("av"));
    if (!values || g_variant_n_children(values) != 4)
        return FALSE;

    double parsed[4];
    for (gsize i = 0; i < G_N_ELEMENTS(parsed); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(values, i);
        g_autoptr(GVariant) value = g_variant_get_variant(boxed);
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
            parsed[i] = g_variant_get_double(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            parsed[i] = (double)g_variant_get_int64(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32))
            parsed[i] = (double)g_variant_get_int32(value);
        else
            return FALSE;
        if (!isfinite(parsed[i]) || parsed[i] < -128 || parsed[i] > 128)
            return FALSE;
    }

    memcpy(padding, parsed, sizeof(parsed));
    return TRUE;
}

static gboolean native_rule_get_color(GVariant* record, const char* key, double color[4]) {
    g_autoptr(GVariant) value = g_variant_lookup_value(record, key, G_VARIANT_TYPE_STRING);
    if (!value)
        return FALSE;

    const char* text = g_variant_get_string(value, NULL);
    gsize length = strlen(text);
    if (length != 7 && length != 9)
        return FALSE;
    if (text[0] != '#')
        return FALSE;

    guint8 channels[4] = {0, 0, 0, 255};
    for (guint i = 0; i < (length == 9 ? 4u : 3u); i++) {
        int high = g_ascii_xdigit_value(text[1 + i * 2]);
        int low = g_ascii_xdigit_value(text[2 + i * 2]);
        if (high < 0 || low < 0)
            return FALSE;
        channels[i] = (guint8)(high * 16 + low);
    }
    for (guint i = 0; i < 4; i++)
        color[i] = channels[i] / 255.;
    return TRUE;
}

static gboolean native_rule_get_boolean(GVariant* record, const char* key, const char* legacy_key,
                                        gboolean* value) {
    return g_variant_lookup(record, key, "b", value) ||
           (legacy_key && g_variant_lookup(record, legacy_key, "b", value));
}

enum {
    NATIVE_CORNER_TOOLKIT_NONE = 0,
    NATIVE_CORNER_TOOLKIT_ADWAITA = 1 << 0,
    NATIVE_CORNER_TOOLKIT_HANDY = 1 << 1,
    NATIVE_CORNER_TOOLKIT_PENDING = 1 << 2,
};

typedef struct {
    pid_t pid;
    guint64 start_time;
} NativeCornerProcessIdentity;

typedef struct {
    NativeCornerToolkitCache* cache;
    NativeCornerProcessIdentity identity;
} NativeCornerToolkitProbe;

static guint native_corner_process_identity_hash(gconstpointer data) {
    const NativeCornerProcessIdentity* identity = data;
    guint64 start_time = identity->start_time;
    guint hash = (guint)identity->pid;
    hash = hash * 33u + (guint)start_time;
    hash = hash * 33u + (guint)(start_time >> 32);
    return hash;
}

static gboolean native_corner_process_identity_equal(gconstpointer a, gconstpointer b) {
    const NativeCornerProcessIdentity* left = a;
    const NativeCornerProcessIdentity* right = b;
    return left->pid == right->pid && left->start_time == right->start_time;
}

static gboolean native_corner_process_identity_get(pid_t pid,
                                                   NativeCornerProcessIdentity* identity) {
    g_autofree char* path = g_strdup_printf("/proc/%d/stat", pid);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0)
        return FALSE;

    char stat[4096];
    ssize_t count;
    do {
        count = read(fd, stat, sizeof(stat) - 1);
    } while (count < 0 && errno == EINTR);
    close(fd);
    if (count <= 0)
        return FALSE;
    stat[count] = '\0';

    /* comm is parenthesized but may itself contain spaces or ')'. The final
     * ')' separates it from state (field 3); starttime is field 22. */
    char* cursor = strrchr(stat, ')');
    if (!cursor)
        return FALSE;
    cursor++;
    for (guint field = 3; field <= 22; field++) {
        while (g_ascii_isspace(*cursor))
            cursor++;
        if (!*cursor)
            return FALSE;
        char* end = cursor;
        while (*end && !g_ascii_isspace(*end))
            end++;
        if (field == 22) {
            g_autofree char* value = g_strndup(cursor, end - cursor);
            char* parse_end = NULL;
            const guint64 start_time = g_ascii_strtoull(value, &parse_end, 10);
            if (parse_end == value || !parse_end || *parse_end)
                return FALSE;
            identity->pid = pid;
            identity->start_time = start_time;
            return TRUE;
        }
        cursor = end;
    }
    return FALSE;
}

static gpointer native_corner_toolkit_cache_lookup(NativeCornerToolkitCache* cache,
                                                   const NativeCornerProcessIdentity* identity) {
    return cache ? g_hash_table_lookup(cache->entries, identity) : NULL;
}

static void native_corner_toolkit_probe_next(GnoblinNativeControl* control,
                                             const NativeCornerProcessIdentity* after);

static NativeCornerToolkitCache* native_corner_toolkit_cache_ref(NativeCornerToolkitCache* cache) {
    g_atomic_int_inc(&cache->ref_count);
    return cache;
}

static void native_corner_toolkit_cache_unref(NativeCornerToolkitCache* cache) {
    if (!cache || !g_atomic_int_dec_and_test(&cache->ref_count))
        return;
    g_hash_table_unref(cache->entries);
    g_free(cache);
}

void gnoblin_control_corner_toolkit_cache_shutdown(GnoblinNativeControl* control) {
    if (!control || !control->corner_toolkit_cache)
        return;

    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    control->corner_toolkit_cache = NULL;
    /* Worker callbacks are dispatched on the compositor's main context. The
     * cache outlives them, but clearing this pointer makes them harmless after
     * control teardown without keeping the whole native control alive. */
    cache->control = NULL;
    native_corner_toolkit_cache_unref(cache);
}

static void native_corner_toolkit_probe_free(gpointer data) {
    NativeCornerToolkitProbe* probe = data;
    native_corner_toolkit_cache_unref(probe->cache);
    g_free(probe);
}

static void native_corner_toolkit_probe_worker(GTask* task, gpointer source_object,
                                               gpointer task_data, GCancellable* cancellable) {
    (void)source_object;
    (void)cancellable;
    NativeCornerToolkitProbe* probe = task_data;
    g_autofree char* path = g_strdup_printf("/proc/%d/maps", probe->identity.pid);
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        /* Restricted /proc access is expected in some sandboxes. Match the
         * previous behavior: failed detection leaves clipping enabled. */
        g_task_return_int(task, NATIVE_CORNER_TOOLKIT_NONE);
        return;
    }

    g_autoptr(GByteArray) contents = g_byte_array_sized_new(MAX_CORNER_TOOLKIT_MAPS_BYTES + 1);
    guint8 chunk[8192];
    while (contents->len < MAX_CORNER_TOOLKIT_MAPS_BYTES) {
        const gsize remaining = MAX_CORNER_TOOLKIT_MAPS_BYTES - contents->len;
        const ssize_t count = read(fd, chunk, MIN(sizeof(chunk), remaining));
        if (count > 0) {
            g_byte_array_append(contents, chunk, count);
            continue;
        }
        if (count < 0 && errno == EINTR)
            continue;
        break;
    }
    close(fd);
    g_byte_array_append(contents, (const guint8*)"", 1);

    guint result = NATIVE_CORNER_TOOLKIT_NONE;
    if (strstr((const char*)contents->data, "/libadwaita-1.so"))
        result |= NATIVE_CORNER_TOOLKIT_ADWAITA;
    if (strstr((const char*)contents->data, "/libhandy-1.so"))
        result |= NATIVE_CORNER_TOOLKIT_HANDY;
    g_task_return_int(task, result);
}

static void native_corner_toolkit_probe_finished(GObject* source_object, GAsyncResult* result,
                                                 gpointer user_data) {
    (void)source_object;
    (void)user_data;
    GTask* task = G_TASK(result);
    NativeCornerToolkitProbe* probe = g_task_get_task_data(task);
    NativeCornerToolkitCache* cache = probe->cache;
    GnoblinNativeControl* control = cache->control;
    g_autoptr(GError) error = NULL;
    const gssize probe_result = g_task_propagate_int(task, &error);
    const guint toolkit_flags =
        probe_result >= 0 ? (guint)probe_result : NATIVE_CORNER_TOOLKIT_NONE;

    if (!control || control->stopping || !control->display)
        return;

    gpointer previous = native_corner_toolkit_cache_lookup(cache, &probe->identity);
    if (previous && GPOINTER_TO_UINT(previous) == NATIVE_CORNER_TOOLKIT_PENDING + 1 &&
        cache->pending_count > 0)
        cache->pending_count--;
    NativeCornerProcessIdentity* key = g_new(NativeCornerProcessIdentity, 1);
    *key = probe->identity;
    g_hash_table_replace(cache->entries, key, GUINT_TO_POINTER(toolkit_flags + 1));
    GSList* windows = meta_display_list_windows(control->display, META_LIST_DEFAULT);
    for (GSList* link = windows; link; link = link->next) {
        MetaWindow* window = link->data;
        NativeCornerProcessIdentity current_identity;
        if (meta_window_get_pid(window) == probe->identity.pid &&
            native_corner_process_identity_get(probe->identity.pid, &current_identity) &&
            native_corner_process_identity_equal(&current_identity, &probe->identity))
            native_apply_window_rules(control, window);
    }
    g_slist_free(windows);

    /* A full cache can defer new PIDs while every slot is pending. Walk the
     * current windows again after each completion, starting after this PID,
     * and use the newly completed slot as room for the next pending probe. */
    native_corner_toolkit_probe_next(control, &probe->identity);
}

static gboolean native_corner_toolkit_cache_make_room(NativeCornerToolkitCache* cache) {
    if (g_hash_table_size(cache->entries) < MAX_CORNER_TOOLKIT_CACHE_ENTRIES)
        return TRUE;
    if (cache->pending_count >= MAX_CORNER_TOOLKIT_CACHE_ENTRIES)
        return FALSE;

    GHashTableIter iter;
    gpointer value;
    g_hash_table_iter_init(&iter, cache->entries);
    while (g_hash_table_iter_next(&iter, NULL, &value)) {
        if (GPOINTER_TO_UINT(value) != NATIVE_CORNER_TOOLKIT_PENDING + 1) {
            g_hash_table_iter_remove(&iter);
            return TRUE;
        }
    }
    return FALSE;
}

static gboolean native_corner_toolkit_probe(GnoblinNativeControl* control,
                                            const NativeCornerProcessIdentity* identity) {
    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    if (!cache) {
        cache = g_new0(NativeCornerToolkitCache, 1);
        cache->ref_count = 1;
        cache->control = control;
        cache->entries = g_hash_table_new_full(native_corner_process_identity_hash,
                                               native_corner_process_identity_equal, g_free, NULL);
        control->corner_toolkit_cache = cache;
    }

    if (native_corner_toolkit_cache_lookup(cache, identity) ||
        !native_corner_toolkit_cache_make_room(cache))
        return FALSE;

    NativeCornerProcessIdentity* key = g_new(NativeCornerProcessIdentity, 1);
    *key = *identity;
    g_hash_table_insert(cache->entries, key, GUINT_TO_POINTER(NATIVE_CORNER_TOOLKIT_PENDING + 1));
    cache->pending_count++;
    NativeCornerToolkitProbe* probe = g_new0(NativeCornerToolkitProbe, 1);
    probe->cache = native_corner_toolkit_cache_ref(cache);
    probe->identity = *identity;
    GTask* task = g_task_new(NULL, NULL, native_corner_toolkit_probe_finished, NULL);
    g_task_set_task_data(task, probe, native_corner_toolkit_probe_free);
    g_task_run_in_thread(task, native_corner_toolkit_probe_worker);
    g_object_unref(task);
    return TRUE;
}

static void native_corner_toolkit_probe_next(GnoblinNativeControl* control,
                                             const NativeCornerProcessIdentity* after) {
    if (!control || control->stopping || !control->display)
        return;

    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    if (!cache)
        return;

    g_autoptr(GPtrArray) identities = g_ptr_array_new_with_free_func(g_free);
    GSList* windows = meta_display_list_windows(control->display, META_LIST_DEFAULT);
    for (GSList* link = windows; link; link = link->next) {
        MetaWindow* window = link->data;
        const pid_t pid = meta_window_get_pid(window);
        if (pid <= 0)
            continue;
        NativeCornerProcessIdentity current;
        if (!native_corner_process_identity_get(pid, &current))
            continue;

        gboolean found = FALSE;
        for (guint i = 0; i < identities->len; i++) {
            if (native_corner_process_identity_equal(g_ptr_array_index(identities, i), &current)) {
                found = TRUE;
                break;
            }
        }
        if (!found) {
            NativeCornerProcessIdentity* copy = g_new(NativeCornerProcessIdentity, 1);
            *copy = current;
            g_ptr_array_add(identities, copy);
        }
    }
    if (identities->len == 0) {
        g_slist_free(windows);
        return;
    }

    guint start = 0;
    for (guint i = 0; i < identities->len; i++) {
        if (native_corner_process_identity_equal(g_ptr_array_index(identities, i), after)) {
            start = (i + 1) % identities->len;
            break;
        }
    }

    for (guint offset = 0; offset < identities->len; offset++) {
        const guint index = (start + offset) % identities->len;
        const NativeCornerProcessIdentity* identity = g_ptr_array_index(identities, index);
        if (native_corner_toolkit_cache_lookup(cache, identity))
            continue;

        NativeCornerProcessIdentity current;
        if (!native_corner_process_identity_get(identity->pid, &current) ||
            !native_corner_process_identity_equal(&current, identity))
            continue;
        const guint pending_before = cache->pending_count;
        for (GSList* link = windows; link; link = link->next) {
            MetaWindow* window = link->data;
            if (meta_window_get_pid(window) == identity->pid)
                native_apply_window_rules(control, window);
            if (cache->pending_count > pending_before)
                break;
        }
        if (cache->pending_count > pending_before)
            break;
    }
    g_slist_free(windows);
}

static gboolean native_corner_toolkit_should_skip(GnoblinNativeControl* control, MetaWindow* window,
                                                  gboolean skip_libadwaita,
                                                  gboolean skip_libhandy) {
    if ((!skip_libadwaita && !skip_libhandy) || !control || control->stopping)
        return FALSE;

    const pid_t pid = meta_window_get_pid(window);
    if (pid <= 0)
        return FALSE;

    NativeCornerProcessIdentity identity;
    if (!native_corner_process_identity_get(pid, &identity))
        return FALSE;

    NativeCornerToolkitCache* cache = control->corner_toolkit_cache;
    gpointer encoded = native_corner_toolkit_cache_lookup(cache, &identity);
    if (!encoded) {
        native_corner_toolkit_probe(control, &identity);
        /* Match the old watcher: apply auto mode while detection is pending,
         * then update the actor when the bounded maps scan completes. */
        return FALSE;
    }

    const guint toolkit_flags = GPOINTER_TO_UINT(encoded) - 1;
    if (toolkit_flags == NATIVE_CORNER_TOOLKIT_PENDING)
        return FALSE;

    return (skip_libadwaita && (toolkit_flags & NATIVE_CORNER_TOOLKIT_ADWAITA)) ||
           (skip_libhandy && (toolkit_flags & NATIVE_CORNER_TOOLKIT_HANDY));
}

static gboolean native_window_rule_matches(GVariant* match, MetaWindow* window,
                                           GnoblinNativeControl* control) {
    const char* title = meta_window_get_title(window);
    const char* gtk_app_id = meta_window_get_gtk_application_id(window);
    const char* wm_class = meta_window_get_wm_class(window);
    const char* layer_namespace = g_object_get_data(G_OBJECT(window), "gnoblin-layer-namespace");
    const char* rule_app_id = gtk_app_id && *gtk_app_id ? gtk_app_id : (wm_class ? wm_class : "");
    MetaWorkspace* workspace = meta_window_get_workspace(window);
    const char* workspace_id =
        workspace ? g_object_get_data(G_OBJECT(workspace), "gnoblin-native-id") : NULL;
    const int workspace_number = workspace ? meta_workspace_index(workspace) + 1 : 0;
    const gboolean focused = meta_display_get_focus_window(control->display) == window;
    GVariantIter iter;
    const char* key;
    GVariant* value;

    if (!g_variant_is_of_type(match, G_VARIANT_TYPE_VARDICT))
        return FALSE;

    g_variant_iter_init(&iter, match);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean matched = FALSE;

        if (g_str_equal(key, "type")) {
            if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
                const char* type = g_variant_get_string(value, NULL);
                matched = (g_str_equal(type, "window") && !layer_namespace) ||
                          (g_str_equal(type, "layer") && layer_namespace);
            }
        } else if (g_str_equal(key, "layer")) {
            const char* pattern = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                      ? g_variant_get_string(value, NULL)
                                      : NULL;
            g_autoptr(GError) error = NULL;
            gboolean pattern_matched = FALSE;
            matched =
                pattern && layer_namespace &&
                gnoblin_lua_pattern_match(pattern, layer_namespace, &pattern_matched, &error) &&
                pattern_matched;
        } else if (g_str_equal(key, "focused")) {
            matched = g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN) &&
                      g_variant_get_boolean(value) == focused;
        } else if (g_str_equal(key, "workspace_id") || g_str_equal(key, "workspace-id")) {
            const char* expected = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                       ? g_variant_get_string(value, NULL)
                                       : NULL;
            matched = expected && workspace_id && g_str_equal(expected, workspace_id);
        } else if (g_str_equal(key, "workspace_number") || g_str_equal(key, "workspace-number")) {
            gint64 expected = 0;
            if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64)) {
                expected = g_variant_get_int64(value);
                matched = TRUE;
            } else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT32)) {
                expected = g_variant_get_int32(value);
                matched = TRUE;
            }
            matched = matched && expected == workspace_number;
        } else if (g_str_equal(key, "app-id") || g_str_equal(key, "app_id") ||
                   g_str_equal(key, "title")) {
            const char* pattern = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)
                                      ? g_variant_get_string(value, NULL)
                                      : NULL;
            const char* subject = g_str_equal(key, "title") ? (title ? title : "") : rule_app_id;
            g_autoptr(GError) error = NULL;
            gboolean pattern_matched = FALSE;
            matched = pattern &&
                      gnoblin_lua_pattern_match(pattern, subject, &pattern_matched, &error) &&
                      pattern_matched;
        }

        g_variant_unref(value);
        if (!matched)
            return FALSE;
    }
    return TRUE;
}

static void native_shadow_layer_defaults(MetaGnoblinWindowShadowLayer* layer) {
    *layer = (MetaGnoblinWindowShadowLayer){
        .x = 0.,
        .y = 4.,
        .blur = 28.,
        .spread = 4.,
        .opacity = .6,
        .color = {0., 0., 0., 1.},
    };
}

static gboolean native_shadow_layer_parse(GVariant* value, MetaGnoblinWindowShadowLayer* layer,
                                          gboolean initialize) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) unboxed = g_variant_get_variant(value);
        return native_shadow_layer_parse(unboxed, layer, initialize);
    }
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT))
        return FALSE;
    if (initialize)
        native_shadow_layer_defaults(layer);
    double number;
    if (native_rule_get_number(value, "x", &number))
        layer->x = number;
    if (native_rule_get_number(value, "y", &number))
        layer->y = number;
    if (native_rule_get_number(value, "blur", &number))
        layer->blur = number;
    if (native_rule_get_number(value, "spread", &number))
        layer->spread = number;
    if (native_rule_get_number(value, "opacity", &number))
        layer->opacity = number;
    double color[4];
    if (native_rule_get_color(value, "color", color))
        memcpy(layer->color, color, sizeof(color));
    return TRUE;
}

static gboolean native_shadow_layers_parse(GVariant* value, MetaGnoblinWindowShadowLayer layers[4],
                                           guint* n_layers, gboolean merge_single_layer) {
    *n_layers = 0;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) unboxed = g_variant_get_variant(value);
        return native_shadow_layers_parse(unboxed, layers, n_layers, merge_single_layer);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN)) {
        if (g_variant_get_boolean(value)) {
            native_shadow_layer_defaults(&layers[0]);
            *n_layers = 1;
        }
        return TRUE;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        if (!native_shadow_layer_parse(value, &layers[0], !merge_single_layer))
            return FALSE;
        *n_layers = 1;
        return TRUE;
    }
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE("av")) || g_variant_n_children(value) == 0 ||
        g_variant_n_children(value) > 4)
        return FALSE;
    for (gsize i = 0; i < g_variant_n_children(value); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
        if (!native_shadow_layer_parse(boxed, &layers[i], TRUE))
            return FALSE;
        (*n_layers)++;
    }
    return TRUE;
}

static void native_shadow_transition_set_easing(GVariant* record, const char* key,
                                                MetaGnoblinWindowShadowTransition* transition) {
    g_autoptr(GVariant) value = g_variant_lookup_value(record, key, NULL);
    if (!value)
        return;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
        transition->easing = g_variant_get_string(value, NULL);
        transition->has_bezier = FALSE;
        return;
    }
    const char* type = NULL;
    if (!g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(value, "type", "&s", &type) || !g_str_equal(type, "cubic-bezier"))
        return;
    const char* names[] = {"x1", "y1", "x2", "y2"};
    double points[4];
    for (guint i = 0; i < G_N_ELEMENTS(points); i++)
        if (!native_rule_get_number(value, names[i], &points[i]) || points[i] < -2. ||
            points[i] > 2.)
            return;
    if (points[0] > 1. || points[0] < 0. || points[2] > 1. || points[2] < 0.)
        return;
    memcpy(transition->bezier, points, sizeof(points));
    transition->has_bezier = TRUE;
}

static GVariant* native_shadow_animation_merge(GVariant* previous, GVariant* corners) {
    g_autoptr(GVariant) next = g_variant_lookup_value(corners, "shadow_animation", NULL);
    if (!next)
        next = g_variant_lookup_value(corners, "shadow-animation", NULL);
    if (!next)
        return previous ? g_variant_ref(previous) : NULL;
    g_autoptr(GVariant) unboxed = NULL;
    GVariant* next_value = next;
    if (g_variant_is_of_type(next_value, G_VARIANT_TYPE_VARIANT)) {
        unboxed = g_variant_get_variant(next_value);
        next_value = unboxed;
    }
    if (!g_variant_is_of_type(next_value, G_VARIANT_TYPE_VARDICT))
        return previous ? g_variant_ref(previous) : NULL;

    GVariantDict merged;
    g_variant_dict_init(&merged, previous && g_variant_is_of_type(previous, G_VARIANT_TYPE_VARDICT)
                                     ? previous
                                     : NULL);
    GVariantIter iter;
    const char* key;
    GVariant* value;
    g_variant_iter_init(&iter, next_value);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        g_variant_dict_insert_value(&merged, key, value);
        g_variant_unref(value);
    }
    return g_variant_ref_sink(g_variant_dict_end(&merged));
}

static void native_shadow_animation_resolve(GVariant* document, GVariant* animation,
                                            MetaGnoblinWindowShadowTransition* transition) {
    transition->duration_ms = 0;
    transition->easing = "ease-out-cubic";
    transition->has_bezier = FALSE;
    const char* selected_name = NULL;
    if (animation && g_variant_is_of_type(animation, G_VARIANT_TYPE_VARDICT)) {
        g_variant_lookup(animation, "animation", "&s", &selected_name);
    }

    g_autoptr(GVariant) registrations =
        document ? g_variant_lookup_value(document, "animations", NULL) : NULL;
    gboolean selected_found = !selected_name || g_str_equal(selected_name, "none") ||
                              g_str_equal(selected_name, "gnoblin-shadow-change");
    for (gsize i = 0; registrations && i < g_variant_n_children(registrations); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(registrations, i);
        g_autoptr(GVariant) registration = g_variant_is_of_type(boxed, G_VARIANT_TYPE_VARIANT)
                                               ? g_variant_get_variant(boxed)
                                               : g_variant_ref(boxed);
        const char* name = NULL;
        const char* event = NULL;
        gboolean enabled = TRUE;
        if (!g_variant_is_of_type(registration, G_VARIANT_TYPE_VARDICT) ||
            !g_variant_lookup(registration, "event", "&s", &event) ||
            !g_str_equal(event, "shadow-change"))
            continue;
        g_variant_lookup(registration, "name", "&s", &name);
        g_variant_lookup(registration, "enable", "b", &enabled);
        if (!enabled || (selected_name && g_strcmp0(selected_name, name) != 0))
            continue;
        selected_found = TRUE;
        double duration = 0;
        native_rule_get_number(registration, "duration", &duration);
        if (duration >= 0 && duration <= 2000)
            transition->duration_ms = (guint)duration;
        native_shadow_transition_set_easing(registration, "ease", transition);
        native_shadow_transition_set_easing(registration, "easing", transition);
        break;
    }
    if (!selected_found) {
        transition->duration_ms = 0;
        transition->easing = "ease-out-cubic";
    }
    if (animation) {
        double duration = transition->duration_ms;
        if (native_rule_get_number(animation, "duration", &duration) && duration >= 0 &&
            duration <= 2000)
            transition->duration_ms = (guint)duration;
        native_shadow_transition_set_easing(animation, "ease", transition);
        native_shadow_transition_set_easing(animation, "easing", transition);
    }
}

void gnoblin_control_window_shader_file_free(gpointer data) {
    NativeWindowShaderFile* file = data;
    if (!file)
        return;
    if (file->reload_timeout_id)
        g_source_remove(file->reload_timeout_id);
    g_clear_object(&file->monitor);
    g_free(file->path);
    g_free(file->source);
    g_free(file);
}

static gboolean native_window_shader_file_reload(NativeWindowShaderFile* file, GError** error) {
    g_autofree char* contents = NULL;
    gsize length = 0;
    if (!g_file_get_contents(file->path, &contents, &length, error))
        return FALSE;
    if (length > 64 * 1024) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA, "shader exceeds 64 KiB");
        return FALSE;
    }
    if (!g_utf8_validate(contents, (gssize)length, NULL)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "shader is not valid UTF-8");
        return FALSE;
    }
    if (!strstr(contents, "gnoblin_effect")) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "shader must define gnoblin_effect");
        return FALSE;
    }
    g_free(file->source);
    file->source = g_steal_pointer(&contents);
    return TRUE;
}

static gboolean native_window_shader_file_reload_timeout(gpointer user_data) {
    NativeWindowShaderFile* file = user_data;
    GnoblinNativeControl* control = file->control;
    file->reload_timeout_id = 0;
    if (!control || control->stopping)
        return G_SOURCE_REMOVE;
    g_autoptr(GError) error = NULL;
    if (native_window_shader_file_reload(file, &error))
        native_apply_all_window_rules(control);
    else
        g_warning("gnoblin-shader: keeping previous effect for %s: %s", file->path,
                  error ? error->message : "could not read shader");
    return G_SOURCE_REMOVE;
}

static void native_window_shader_file_changed(GFileMonitor* monitor, GFile* changed_file,
                                              GFile* other_file, GFileMonitorEvent event_type,
                                              gpointer user_data) {
    NativeWindowShaderFile* file = user_data;
    (void)monitor;
    (void)event_type;
    g_autofree char* changed_path = changed_file ? g_file_get_path(changed_file) : NULL;
    g_autofree char* other_path = other_file ? g_file_get_path(other_file) : NULL;
    if (g_strcmp0(changed_path, file->path) != 0 && g_strcmp0(other_path, file->path) != 0)
        return;
    if (!file->control || file->control->stopping)
        return;
    if (file->reload_timeout_id)
        g_source_remove(file->reload_timeout_id);
    file->reload_timeout_id = g_timeout_add(100, native_window_shader_file_reload_timeout, file);
}

static char* native_window_shader_resolve_path(const char* shader) {
    if (!shader || !*shader)
        return NULL;
    if (g_str_has_prefix(shader, "~/")) {
        g_autofree char* expanded = g_build_filename(g_get_home_dir(), shader + 2, NULL);
        return g_canonicalize_filename(expanded, NULL);
    }
    if (g_path_is_absolute(shader))
        return g_canonicalize_filename(shader, NULL);
    g_autofree char* config = NULL;
    const char* configured_path = g_getenv("GNOBLIN_CONFIG");
    if (configured_path && *configured_path)
        config = g_strdup(configured_path);
    else
        config = g_build_filename(g_get_user_config_dir(), "gnoblin", "init.lua", NULL);
    g_autofree char* directory = g_path_get_dirname(config);
    return g_canonicalize_filename(shader, directory);
}

static NativeWindowShaderFile* native_window_shader_file_get(GnoblinNativeControl* control,
                                                             const char* path) {
    NativeWindowShaderFile* file = g_hash_table_lookup(control->window_shader_files, path);
    if (file)
        return file;
    if (g_hash_table_size(control->window_shader_files) >= 128) {
        g_warning("gnoblin-shader: refusing more than 128 shader files in one session");
        return NULL;
    }
    file = g_new0(NativeWindowShaderFile, 1);
    file->control = control;
    file->path = g_strdup(path);
    g_hash_table_insert(control->window_shader_files, g_strdup(path), file);
    g_autoptr(GFile) shader_file = g_file_new_for_path(path);
    g_autoptr(GFile) parent = g_file_get_parent(shader_file);
    if (parent) {
        g_autoptr(GError) monitor_error = NULL;
        file->monitor =
            g_file_monitor_directory(parent, G_FILE_MONITOR_WATCH_MOVES, NULL, &monitor_error);
        if (file->monitor)
            g_signal_connect(file->monitor, "changed",
                             G_CALLBACK(native_window_shader_file_changed), file);
        else
            g_warning("gnoblin-shader: cannot watch %s: %s", path,
                      monitor_error ? monitor_error->message : "monitor unavailable");
    }
    g_autoptr(GError) error = NULL;
    if (!native_window_shader_file_reload(file, &error))
        g_warning("gnoblin-shader: keeping previous effect for %s: %s", path,
                  error ? error->message : "could not read shader");
    return file;
}

/* Resolve the effective workspace target: the last matching rule that sets it wins. */
static GVariant* native_rule_workspace_target(GnoblinNativeControl* control, MetaWindow* window) {
    g_autoptr(GVariant) document = gnoblin_control_native_config_document(control);
    g_autoptr(GVariant) rules =
        document ? g_variant_lookup_value(document, "window-rules", NULL) : NULL;
    GVariant* target = NULL;
    for (gsize i = 0; rules && i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) boxed_rule = g_variant_get_child_value(rules, i);
        g_autoptr(GVariant) rule = g_variant_is_of_type(boxed_rule, G_VARIANT_TYPE_VARIANT)
                                       ? g_variant_get_variant(boxed_rule)
                                       : g_variant_ref(boxed_rule);
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !native_window_rule_matches(match, window, control))
            continue;
        g_autoptr(GVariant) workspace = g_variant_lookup_value(rule, "workspace", NULL);
        if (workspace && g_variant_is_of_type(workspace, G_VARIANT_TYPE_VARDICT)) {
            g_clear_pointer(&target, g_variant_unref);
            target = g_variant_ref(workspace);
        }
    }
    return target;
}

/* Called on window creation and on every property change until the window is ready.
 * A Wayland client sets its app ID and title after the toplevel exists, so rules that
 * match on them only work once the window has gone through its first commit. The guard
 * is set on that first ready call, whether or not the target resolves, so a later
 * reload, title change or focus change never moves the window. */
void gnoblin_control_place_window_by_rules(GnoblinNativeControl* control, MetaWindow* window) {
    if (!control || !control->display || !window ||
        g_object_get_data(G_OBJECT(window), "gnoblin-rule-workspace-placed"))
        return;
    if (!meta_window_is_ready(window))
        return;
    g_object_set_data(G_OBJECT(window), "gnoblin-rule-workspace-placed", GINT_TO_POINTER(1));

    if (meta_window_get_window_type(window) != META_WINDOW_NORMAL ||
        meta_window_is_override_redirect(window) || meta_window_get_transient_for(window) ||
        g_object_get_data(G_OBJECT(window), "gnoblin-layer-namespace"))
        return;

    g_autoptr(GVariant) target = native_rule_workspace_target(control, window);
    if (!target)
        return;

    MetaWorkspaceManager* manager = meta_display_get_workspace_manager(control->display);
    MetaWorkspace* workspace = NULL;
    g_autoptr(GVariant) id = g_variant_lookup_value(target, "id", G_VARIANT_TYPE_STRING);
    g_autoptr(GVariant) number = g_variant_lookup_value(target, "number", NULL);
    const char* title = meta_window_get_title(window) ?: "(untitled)";
    if (id) {
        const char* wanted = g_variant_get_string(id, NULL);
        for (GList* l = meta_workspace_manager_get_workspaces(manager); l; l = l->next) {
            const char* workspace_id = g_object_get_data(G_OBJECT(l->data), "gnoblin-native-id");
            if (workspace_id && g_str_equal(workspace_id, wanted)) {
                workspace = l->data;
                break;
            }
        }
        if (!workspace)
            g_warning("gnoblin-window-rule: workspace id '%s' not present for %s", wanted, title);
    } else if (number) {
        gint64 position = g_variant_is_of_type(number, G_VARIANT_TYPE_INT64)
                              ? g_variant_get_int64(number)
                              : g_variant_is_of_type(number, G_VARIANT_TYPE_INT32)
                                    ? g_variant_get_int32(number)
                                    : 0;
        if (position >= 1 && position <= meta_workspace_manager_get_n_workspaces(manager))
            workspace = meta_workspace_manager_get_workspace_by_index(manager, position - 1);
        if (!workspace)
            g_warning("gnoblin-window-rule: workspace number %" G_GINT64_FORMAT
                      " not available for %s",
                      position, title);
    }
    if (workspace && workspace != meta_window_get_workspace(window))
        meta_window_change_workspace(window, workspace);
}

/* Rule blur is stored on the window as radius and ignore flag so a size change only recomputes
 * the region. The client's own ext-background-effect region still wins inside the compositor. */
static void native_window_rule_blur_update(MetaWindow* window, MetaSurfaceActor* surface) {
    gpointer stored_radius = g_object_get_data(G_OBJECT(window), "gnoblin-rule-blur");
    float radius = GPOINTER_TO_INT(stored_radius);
    if (radius <= 0.f) {
        meta_surface_actor_set_rule_background_blur(surface, FALSE, 0.f, NULL);
        return;
    }

    MtkRectangle buffer;
    meta_window_get_buffer_rect(window, &buffer);
    MtkRectangle area = {0, 0, buffer.width, buffer.height};
    if (g_object_get_data(G_OBJECT(window), "gnoblin-rule-blur-ignore-shadows")) {
        MtkRectangle frame;
        meta_window_get_frame_rect(window, &frame);
        MtkRectangle local = {frame.x - buffer.x, frame.y - buffer.y, frame.width, frame.height};
        MtkRectangle clipped;
        if (mtk_rectangle_intersect(&local, &area, &clipped))
            area = clipped;
        else
            area = (MtkRectangle){0, 0, 0, 0};
    }

    g_autoptr(MtkRegion) region = area.width > 0 && area.height > 0
                                      ? mtk_region_create_rectangle(&area)
                                      : NULL;
    meta_surface_actor_set_rule_background_blur(surface, region != NULL, radius, region);
}

void gnoblin_control_refresh_window_rule_blur(MetaWindow* window) {
    if (!window || !g_object_get_data(G_OBJECT(window), "gnoblin-rule-blur"))
        return;
    MetaWindowActor* actor = meta_window_actor_from_window(window);
    MetaSurfaceActor* surface = actor ? meta_window_actor_get_surface(actor) : NULL;
    if (!surface)
        surface = meta_wayland_layer_shell_get_actor(window);
    if (surface)
        native_window_rule_blur_update(window, surface);
}

static void native_apply_window_rules(GnoblinNativeControl* control, MetaWindow* window) {
    if (!control || !window)
        return;

    MetaWindowActor* actor = meta_window_actor_from_window(window);
    MetaSurfaceActor* surface = actor ? meta_window_actor_get_surface(actor) : NULL;
    if (!surface)
        surface = meta_wayland_layer_shell_get_actor(window);
    if (!surface)
        return;

    double radius = 0;
    double smoothing = 0;
    double border_width = 0;
    double border_color[4] = {128. / 255., 128. / 255., 128. / 255., 1.};
    double padding[4] = {0, 0, 0, 0};
    gboolean keep_maximized = TRUE;
    gboolean keep_fullscreen = FALSE;
    gboolean keep_tiled = FALSE;
    gboolean skip_libadwaita = TRUE;
    gboolean skip_libhandy = FALSE;
    gboolean remove_csd = FALSE;
    gboolean keep_shadow = FALSE;
    gboolean shadow_is_single_layer = FALSE;
    MetaGnoblinWindowShadowLayer shadow_layers[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS] = {0};
    guint n_shadow_layers = 0;
    MetaGnoblinWindowShadowTransition shadow_transition = {
        .duration_ms = 0,
        .easing = "ease-out-cubic",
    };
    g_autoptr(GVariant) shadow_animation_config = NULL;
    g_autofree char* mode = g_strdup("auto");
    int frame_policy = 0;
    int frame_extents[4] = {32, 1, 1, 1};
    int frame_crop[4] = {0, 0, 0, 0};
    GVariantDict frame_style;
    g_variant_dict_init(&frame_style, NULL);
    g_variant_dict_insert(&frame_style, "renderer", "s", "native");
    g_autofree char* shader_path = NULL;
    g_autoptr(GVariant) shader_uniforms = NULL;
    gboolean shader_selected = FALSE;
    double rule_opacity = 1.;
    gboolean opacity_selected = FALSE;
    int rule_blur = 0;
    gboolean rule_blur_ignore_shadows = FALSE;
    g_autoptr(GVariant) document = gnoblin_control_native_config_document(control);
    g_autoptr(GVariant) rules =
        document ? g_variant_lookup_value(document, "window-rules", NULL) : NULL;

    for (gsize i = 0; rules && i < g_variant_n_children(rules); i++) {
        g_autoptr(GVariant) boxed_rule = g_variant_get_child_value(rules, i);
        g_autoptr(GVariant) rule = g_variant_is_of_type(boxed_rule, G_VARIANT_TYPE_VARIANT)
                                       ? g_variant_get_variant(boxed_rule)
                                       : g_variant_ref(boxed_rule);
        g_autoptr(GVariant) match = g_variant_lookup_value(rule, "match", NULL);
        if (!match || !native_window_rule_matches(match, window, control))
            continue;

        g_autoptr(GVariant) shader = g_variant_lookup_value(rule, "shader", NULL);
        if (shader && g_variant_is_of_type(shader, G_VARIANT_TYPE_STRING)) {
            shader_selected = TRUE;
            g_free(shader_path);
            const char* configured_shader = g_variant_get_string(shader, NULL);
            shader_path =
                *configured_shader ? native_window_shader_resolve_path(configured_shader) : NULL;
        }

        double configured_opacity;
        if (native_rule_get_number(rule, "opacity", &configured_opacity)) {
            rule_opacity = CLAMP(configured_opacity, 0., 1.);
            opacity_selected = TRUE;
        }

        double configured_blur;
        if (native_rule_get_number(rule, "blur", &configured_blur))
            rule_blur = (int)lround(CLAMP(configured_blur, 0., 100.));
        native_rule_get_boolean(rule, "blur_ignore_shadows", "blur-ignore-shadows",
                                &rule_blur_ignore_shadows);

        g_autoptr(GVariant) frame = g_variant_lookup_value(rule, "frame", NULL);
        if (frame) {
            g_autoptr(GVariant) unboxed_frame = NULL;
            GVariant* frame_options = frame;
            if (g_variant_is_of_type(frame_options, G_VARIANT_TYPE_VARIANT)) {
                unboxed_frame = g_variant_get_variant(frame_options);
                frame_options = unboxed_frame;
            }
            if (g_variant_is_of_type(frame_options, G_VARIANT_TYPE_VARDICT)) {
                g_autoptr(GVariant) frame_mode =
                    native_rule_lookup_alias(frame_options, "mode", NULL);
                if (frame_mode && g_variant_is_of_type(frame_mode, G_VARIANT_TYPE_STRING))
                    frame_policy =
                        native_frame_policy_from_name(g_variant_get_string(frame_mode, NULL));
                native_frame_read_extents(frame_options, "extents", frame_extents);
                native_frame_read_extents(frame_options, "crop", frame_crop);
                native_frame_style_merge_string(&frame_style, frame_options, "renderer", NULL,
                                                "renderer");
                native_frame_style_merge_string(&frame_style, frame_options, "style", NULL,
                                                "style");
                native_frame_style_merge_string(&frame_style, frame_options, "background", NULL,
                                                "background");
                native_frame_style_merge_string(&frame_style, frame_options, "foreground", NULL,
                                                "foreground");
                native_frame_style_merge_string(&frame_style, frame_options, "inactive_background",
                                                "inactive-background", "inactive-background");
                native_frame_style_merge_buttons(&frame_style, frame_options);
            }
        }
        g_autoptr(GVariant) configured_uniforms =
            g_variant_lookup_value(rule, "shader-uniforms", NULL);
        if (configured_uniforms) {
            g_clear_pointer(&shader_uniforms, g_variant_unref);
            shader_uniforms = g_variant_ref(configured_uniforms);
        }

        g_autoptr(GVariant) corners = g_variant_lookup_value(rule, "corners", NULL);
        if (!corners || !g_variant_is_of_type(corners, G_VARIANT_TYPE_VARDICT))
            continue;

        native_rule_get_number(corners, "radius", &radius);
        native_rule_get_number(corners, "smoothing", &smoothing);
        if (!native_rule_get_number(corners, "border_width", &border_width))
            native_rule_get_number(corners, "border-width", &border_width);
        border_width = CLAMP(border_width, -40., 40.);
        if (!native_rule_get_color(corners, "border_color", border_color))
            native_rule_get_color(corners, "border-color", border_color);
        native_rule_get_padding(corners, padding);
        native_rule_get_boolean(corners, "keep_maximized", "keep-maximized", &keep_maximized);
        native_rule_get_boolean(corners, "keep_fullscreen", "keep-fullscreen", &keep_fullscreen);
        native_rule_get_boolean(corners, "keep_tiled", "keep-tiled", &keep_tiled);
        native_rule_get_boolean(corners, "skip_libadwaita", "skip-libadwaita", &skip_libadwaita);
        native_rule_get_boolean(corners, "skip_libhandy", "skip-libhandy", &skip_libhandy);
        native_rule_get_boolean(corners, "remove_csd", "remove-csd", &remove_csd);
        native_rule_get_boolean(corners, "keep_shadow", "keep-shadow", &keep_shadow);
        g_autoptr(GVariant) shadow = g_variant_lookup_value(corners, "shadow", NULL);
        if (shadow) {
            g_autoptr(GVariant) shadow_unboxed = NULL;
            GVariant* shadow_value = shadow;
            if (g_variant_is_of_type(shadow_value, G_VARIANT_TYPE_VARIANT)) {
                shadow_unboxed = g_variant_get_variant(shadow_value);
                shadow_value = shadow_unboxed;
            }
            const gboolean is_single_layer =
                g_variant_is_of_type(shadow_value, G_VARIANT_TYPE_VARDICT);
            const gboolean merge_single_layer =
                shadow_is_single_layer && is_single_layer && n_shadow_layers > 0;
            MetaGnoblinWindowShadowLayer parsed[META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS] = {0};
            MetaGnoblinWindowShadowLayer* target = merge_single_layer ? shadow_layers : parsed;
            guint parsed_count = 0;
            if (native_shadow_layers_parse(shadow_value, target, &parsed_count,
                                           merge_single_layer)) {
                if (!merge_single_layer)
                    memcpy(shadow_layers, parsed, sizeof(shadow_layers));
                n_shadow_layers = parsed_count;
                shadow_is_single_layer = is_single_layer;
            }
        }
        GVariant* merged_animation =
            native_shadow_animation_merge(shadow_animation_config, corners);
        g_clear_pointer(&shadow_animation_config, g_variant_unref);
        shadow_animation_config = merged_animation;
        g_autoptr(GVariant) mode_value = g_variant_lookup_value(corners, "mode", NULL);
        if (mode_value && g_variant_is_of_type(mode_value, G_VARIANT_TYPE_STRING)) {
            g_free(mode);
            mode = g_strdup(g_variant_get_string(mode_value, NULL));
        }
    }

    /* The surface actor carries the client content, while window animations fade the window
     * actor, so an opacity set here survives them. Restore full opacity only for a window
     * this rule engine faded; never touch the opacity of a window no rule ever matched. */
    if (opacity_selected) {
        clutter_actor_set_opacity(CLUTTER_ACTOR(surface), (guint8)lround(rule_opacity * 255.));
        g_object_set_data(G_OBJECT(window), "gnoblin-rule-opacity", GINT_TO_POINTER(1));
    } else if (g_object_get_data(G_OBJECT(window), "gnoblin-rule-opacity")) {
        clutter_actor_set_opacity(CLUTTER_ACTOR(surface), 255);
        g_object_set_data(G_OBJECT(window), "gnoblin-rule-opacity", NULL);
    }

    /* Only touch the surface blur override for a window this engine blurred before. */
    if (rule_blur > 0) {
        g_object_set_data(G_OBJECT(window), "gnoblin-rule-blur", GINT_TO_POINTER(rule_blur));
        g_object_set_data(G_OBJECT(window), "gnoblin-rule-blur-ignore-shadows",
                          rule_blur_ignore_shadows ? GINT_TO_POINTER(1) : NULL);
        native_window_rule_blur_update(window, surface);
    } else if (g_object_get_data(G_OBJECT(window), "gnoblin-rule-blur")) {
        meta_surface_actor_set_rule_background_blur(surface, FALSE, 0.f, NULL);
        g_object_set_data(G_OBJECT(window), "gnoblin-rule-blur", NULL);
        g_object_set_data(G_OBJECT(window), "gnoblin-rule-blur-ignore-shadows", NULL);
    }

    if (!shader_selected || !shader_path) {
        meta_gnoblin_window_effects_clear_shader(CLUTTER_ACTOR(surface));
    } else {
        NativeWindowShaderFile* shader_file = native_window_shader_file_get(control, shader_path);
        if (shader_file && shader_file->source) {
            MetaGnoblinWindowShaderUniform uniforms[64] = {0};
            guint n_uniforms = 0;
            gboolean uniforms_valid = TRUE;
            if (shader_uniforms && g_variant_is_of_type(shader_uniforms, G_VARIANT_TYPE_VARDICT)) {
                GVariantIter iter;
                const char* name = NULL;
                GVariant* value = NULL;
                g_variant_iter_init(&iter, shader_uniforms);
                while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
                    g_autoptr(GVariant) uniform_value = value;
                    double number = 0;
                    if (g_variant_is_of_type(uniform_value, G_VARIANT_TYPE_DOUBLE))
                        number = g_variant_get_double(uniform_value);
                    else if (g_variant_is_of_type(uniform_value, G_VARIANT_TYPE_INT64))
                        number = (double)g_variant_get_int64(uniform_value);
                    else if (g_variant_is_of_type(uniform_value, G_VARIANT_TYPE_INT32))
                        number = (double)g_variant_get_int32(uniform_value);
                    else
                        uniforms_valid = FALSE;
                    uniforms_valid &= isfinite(number);
                    if (n_uniforms < G_N_ELEMENTS(uniforms)) {
                        uniforms[n_uniforms].name = name;
                        uniforms[n_uniforms].value = number;
                        n_uniforms++;
                    } else {
                        uniforms_valid = FALSE;
                    }
                }
            } else if (shader_uniforms) {
                uniforms_valid = FALSE;
            }
            if (!uniforms_valid) {
                g_warning("gnoblin-shader: keeping previous effect for %s: invalid uniform table",
                          shader_path);
            } else {
                const double surface_width = clutter_actor_get_width(CLUTTER_ACTOR(surface));
                const double surface_height = clutter_actor_get_height(CLUTTER_ACTOR(surface));
                g_autoptr(GError) shader_error = NULL;
                if (!meta_gnoblin_window_effects_set_shader(
                        CLUTTER_ACTOR(surface), shader_file->source, uniforms, n_uniforms,
                        surface_width, surface_height, &shader_error))
                    g_warning("gnoblin-shader: keeping previous effect for %s: %s", shader_path,
                              shader_error ? shader_error->message : "could not apply shader");
            }
        }
    }

    if (!actor)
        return;

    native_shadow_animation_resolve(document, shadow_animation_config, &shadow_transition);

    const MetaMaximizeFlags maximize_flags = meta_window_get_maximize_flags(window);
    const gboolean partially_maximized = !!(maximize_flags & META_MAXIMIZE_HORIZONTAL) !=
                                         !!(maximize_flags & META_MAXIMIZE_VERTICAL);
    const gboolean tiled = meta_window_is_tiled_side_by_side(window) || partially_maximized;
    const MetaWindowType window_type = meta_window_get_window_type(window);
    const gboolean normal =
        (window_type == META_WINDOW_NORMAL || window_type == META_WINDOW_DIALOG ||
         window_type == META_WINDOW_MODAL_DIALOG) &&
        !meta_window_is_override_redirect(window);
    const gboolean allowed = normal && !g_str_equal(mode, "off") &&
                             (!meta_window_is_maximized(window) || keep_maximized) &&
                             (!meta_window_is_fullscreen(window) || keep_fullscreen) &&
                             (!tiled || keep_tiled);
    gboolean clip_enabled = allowed && radius > 0;
    const gboolean border_enabled = allowed && fabs(border_width) > 0.001;
    gboolean has_padding = FALSE;
    for (guint i = 0; i < G_N_ELEMENTS(padding); i++)
        has_padding |= fabs(padding[i]) > 0.001;
    double csd_insets[4] = {0, 0, 0, 0};
    gboolean csd_detected =
        clip_enabled && remove_csd && !has_padding && !window->minimized &&
        meta_gnoblin_window_effects_detect_csd(CLUTTER_ACTOR(actor), CLUTTER_ACTOR(surface),
                                               csd_insets);
    if (clip_enabled && g_str_equal(mode, "auto") && !csd_detected &&
        native_corner_toolkit_should_skip(control, window, skip_libadwaita, skip_libhandy))
        clip_enabled = FALSE;
    const double effect_radius = clip_enabled ? radius : border_enabled ? 0.5 : 0;
    const gboolean effect_enabled = clip_enabled || border_enabled;
    const double exponent = 2 + CLAMP(smoothing, 0, 1) * 4;
    /* A negotiated server frame is drawn rounded around square client content.
     * Auto mode preserves the app's own corners, which would leave the content
     * corners square inside the frame's rounded outline. Force the clip and
     * extend it by the frame's border extents so it matches the outer shape. */
    double clip_padding[4] = {padding[0], padding[1], padding[2], padding[3]};
    gboolean framed_by_server = FALSE;
    MtkRectangle visible_geometry = {0};
    MtkRectangle outer_geometry = {0};
    if (clip_enabled && meta_gnoblin_window_frame_get_visible_geometry(window, &visible_geometry) &&
        meta_gnoblin_window_frame_get_effect_geometry(window, &outer_geometry)) {
        const double extents[4] = {
            visible_geometry.y - outer_geometry.y,
            (outer_geometry.x + outer_geometry.width) - (visible_geometry.x + visible_geometry.width),
            (outer_geometry.y + outer_geometry.height) - (visible_geometry.y + visible_geometry.height),
            visible_geometry.x - outer_geometry.x,
        };
        for (guint i = 0; i < 4; i++) {
            if (extents[i] > 0.) {
                clip_padding[i] -= extents[i];
                framed_by_server = TRUE;
            }
        }
    }
    if (META_IS_WINDOW_ACTOR_WAYLAND(actor))
        meta_window_actor_wayland_set_rounded_clip(
            actor, effect_radius, exponent,
            g_str_equal(mode, "auto") && !csd_detected && !framed_by_server, clip_padding);
    else if (META_IS_WINDOW_ACTOR_X11(actor))
        meta_window_actor_x11_set_rounded_clip(META_WINDOW_ACTOR_X11(actor), effect_radius,
                                               exponent, g_str_equal(mode, "auto") && !csd_detected,
                                               padding);
    /* The frame follows the content clip. A maximised, fullscreen or tiled
     * window with rounding disabled must not keep rounded frame corners. */
    g_variant_dict_insert(&frame_style, "radius", "d", clip_enabled ? radius : 0.);
    g_variant_dict_insert(&frame_style, "exponent", "d", exponent);
    g_variant_dict_insert(&frame_style, "border-width", "d",
                          effect_enabled ? border_width : 0.);
    g_variant_dict_insert(&frame_style, "border-color", "(dddd)", border_color[0],
                          border_color[1], border_color[2], border_color[3]);
    g_autoptr(GVariant) frame_style_options = g_variant_ref_sink(g_variant_dict_end(&frame_style));
    meta_gnoblin_window_frame_style(window, frame_style_options);
    g_autoptr(GError) frame_error = NULL;
    g_autoptr(GVariant) frame_policy_value = g_variant_ref_sink(g_variant_new(
        "(iiiiiiiii)", frame_policy, frame_crop[0], frame_crop[1], frame_crop[2], frame_crop[3],
        frame_extents[0], frame_extents[1], frame_extents[2], frame_extents[3]));
    if (!meta_gnoblin_window_frame_set(window, frame_policy_value, &frame_error) && frame_error &&
        frame_error->code != G_IO_ERROR_INVALID_ARGUMENT)
        g_warning("gnoblin-frame: could not apply frame rule for %s: %s",
                  meta_window_get_title(window) ?: "(untitled)", frame_error->message);
    meta_gnoblin_window_effects_set_rounded_border(CLUTTER_ACTOR(actor),
                                                   effect_enabled ? border_width : 0, border_color);
    const gboolean shadow_state_allowed =
        normal && !g_str_equal(mode, "off") && (!meta_window_is_maximized(window) || keep_shadow) &&
        (!meta_window_is_fullscreen(window) || keep_shadow) && (!tiled || keep_shadow);
    const double actor_width = clutter_actor_get_width(CLUTTER_ACTOR(actor));
    const double actor_height = clutter_actor_get_height(CLUTTER_ACTOR(actor));
    double shadow_geometry[4] = {0., 0., actor_width, actor_height};
    guint shadow_child_index = 0;
    if (META_IS_WINDOW_ACTOR_WAYLAND(actor)) {
        meta_window_actor_wayland_get_surface_container_bounds(actor, shadow_geometry);
        shadow_child_index = meta_window_actor_wayland_get_shadow_child_index(actor);
        MtkRectangle effect_geometry = {0};
        if (meta_gnoblin_window_frame_get_effect_geometry(window, &effect_geometry)) {
            shadow_geometry[0] = effect_geometry.x;
            shadow_geometry[1] = effect_geometry.y;
            shadow_geometry[2] = effect_geometry.x + effect_geometry.width;
            shadow_geometry[3] = effect_geometry.y + effect_geometry.height;
        }
    } else {
        if (surface && clutter_actor_has_allocation(CLUTTER_ACTOR(surface))) {
            ClutterActorBox surface_box;
            clutter_actor_get_allocation_box(CLUTTER_ACTOR(surface), &surface_box);
            shadow_geometry[0] = surface_box.x1;
            shadow_geometry[1] = surface_box.y1;
            shadow_geometry[2] = surface_box.x2;
            shadow_geometry[3] = surface_box.y2;
        }
    }
    /* Client geometry is already in window-local coordinates. The window
     * actor may still be unallocated when this rule first runs, so clamping
     * it to the current actor size would collapse a valid shadow to 0x0. */
    double shadow_bounds[4] = {
        shadow_geometry[0] + padding[3],
        shadow_geometry[1] + padding[0],
        shadow_geometry[2] - padding[1],
        shadow_geometry[3] - padding[2],
    };
    meta_gnoblin_window_effects_set_window_shadow(
        CLUTTER_ACTOR(actor), shadow_state_allowed && n_shadow_layers > 0, shadow_bounds, radius,
        exponent, shadow_layers, n_shadow_layers, &shadow_transition, shadow_child_index);
    if (META_IS_WINDOW_ACTOR_WAYLAND(actor))
        meta_window_actor_wayland_set_csd_reconstruction(actor, clip_enabled && csd_detected,
                                                         csd_insets);
    else if (META_IS_WINDOW_ACTOR_X11(actor))
        meta_window_actor_x11_set_csd_reconstruction(META_WINDOW_ACTOR_X11(actor),
                                                     clip_enabled && csd_detected, csd_insets);
}

static void native_apply_all_window_rules(GnoblinNativeControl* control) {
    if (!control || !control->display || !control->windows)
        return;
    GHashTableIter iter;
    gpointer window;
    g_hash_table_iter_init(&iter, control->windows);
    while (g_hash_table_iter_next(&iter, &window, NULL))
        native_apply_window_rules(control, window);
}

void gnoblin_control_apply_window_rules(GnoblinNativeControl* control, MetaWindow* window) {
    native_apply_window_rules(control, window);
}

void gnoblin_control_apply_all_window_rules(GnoblinNativeControl* control) {
    native_apply_all_window_rules(control);
}
