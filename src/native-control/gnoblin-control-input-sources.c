/* Input sources: the XKB layouts and IBus engines that Lua can list and select, the per-window
 * source memory, IBus bus tracking, and the input source snapshot and events published to Lua.
 * Pointer and keyboard device handling (apply_native_input) stays in gnoblin-native-control.c. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "config.h"

#include "core/gnoblin-control-internal.h"

#include <string.h>
#include <xkbcommon/xkbcommon.h>
#include <xkbcommon/xkbregistry.h>

#include "core/window-private.h"

#define IBUS_BUS_NAME "org.freedesktop.IBus"
#define IBUS_OBJECT_PATH "/org/freedesktop/IBus"
#define IBUS_INTERFACE "org.freedesktop.IBus"

typedef struct {
    GnoblinNativeControl* control;
    MetaKeymapDescription* description;
    GPtrArray* source_ids;
    char* selected_type;
    char* selected_id;
    char* target_window_id;
    gint64 request_id;
    char* method;
    guint group;
    guint64 ibus_owner_generation;
    guint64 ibus_engine_generation;
    gboolean internal_restore;
} PendingInputSource;

typedef struct {
    GnoblinNativeControl* control;
    guint64 ibus_owner_generation;
    guint64 ibus_engine_generation;
    guint64 input_source_selection_generation;
} PendingIBusQuery;

static void native_input_source_free(gpointer data) {
    NativeInputSource* source = data;
    g_free(source->type);
    g_free(source->id);
    g_free(source->layout);
    g_free(source->variant);
    g_free(source->short_name);
    g_free(source->name);
    g_free(source);
}

static NativeInputSource* input_source_new(const char* id) {
    const char* plus = strchr(id, '+');
    g_autofree char* layout = plus ? g_strndup(id, plus - id) : g_strdup(id);
    const char* variant = plus ? plus + 1 : "";
    if (!*layout || (plus && !*variant) || strchr(variant, '+'))
        return NULL;

    struct rxkb_context* registry = rxkb_context_new(RXKB_CONTEXT_LOAD_EXOTIC_RULES);
    if (!registry)
        return NULL;
    if (!rxkb_context_parse(registry, "evdev")) {
        rxkb_context_unref(registry);
        return NULL;
    }
    gboolean found = FALSE;
    for (struct rxkb_layout* candidate = rxkb_layout_first(registry); candidate;
         candidate = rxkb_layout_next(candidate)) {
        if (g_strcmp0(rxkb_layout_get_name(candidate), layout) == 0 &&
            g_strcmp0(rxkb_layout_get_variant(candidate) ? rxkb_layout_get_variant(candidate) : "",
                      variant) == 0) {
            found = TRUE;
            break;
        }
    }
    rxkb_context_unref(registry);
    if (!found)
        return NULL;

    g_autoptr(MetaKeymapDescription) description =
        meta_keymap_description_new_from_rules(NULL, layout, variant, NULL, NULL, NULL);
    if (!description)
        return NULL;
    g_auto(GStrv) display_names = NULL;
    g_auto(GStrv) short_names = NULL;
    g_autoptr(GError) error = NULL;
    struct xkb_keymap* keymap = meta_keymap_description_create_xkb_keymap(
        description, &display_names, &short_names, &error);
    if (!keymap)
        return NULL;
    xkb_keymap_unref(keymap);

    NativeInputSource* source = g_new0(NativeInputSource, 1);
    source->type = g_strdup("xkb");
    source->id = g_strdup(id);
    source->layout = g_steal_pointer(&layout);
    source->variant = g_strdup(variant);
    source->name = g_strdup(display_names && display_names[0] ? display_names[0] : id);
    source->short_name =
        g_strdup(short_names && short_names[0] && *short_names[0] ? short_names[0] : id);
    return source;
}

static NativeInputSource* ibus_input_source_new(const char* id) {
    if (!id || !*id || strlen(id) > 128 || !g_utf8_validate(id, -1, NULL))
        return NULL;
    NativeInputSource* source = g_new0(NativeInputSource, 1);
    source->type = g_strdup("ibus");
    source->id = g_strdup(id);
    /* Engine IDs are stable names from IBus. The optional engine metadata is
     * not needed to select an engine and is intentionally not guessed here. */
    source->name = g_strdup(id);
    source->short_name = g_strdup(id);
    return source;
}

static GVariant* input_source_record_variant(NativeInputSource* source, guint64 revision,
                                             gboolean current) {
    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "type", g_variant_new_string(source->type));
    g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string(source->id));
    g_variant_builder_add(&record, "{sv}", "short_name", g_variant_new_string(source->short_name));
    g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(source->name));
    g_variant_builder_add(&record, "{sv}", "current", g_variant_new_boolean(current));
    g_variant_builder_add(&record, "{sv}", "revision", g_variant_new_int64(revision));
    /* Floating on purpose: callers hand the record to a builder or to g_variant_new_variant(),
     * which take it over, or release it with g_autoptr. */
    return g_variant_builder_end(&record);
}

static NativeInputSource* input_source_by_id(GnoblinNativeControl* control, const char* id) {
    for (guint i = 0; control->input_sources && i < control->input_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->input_sources, i);
        if (g_str_equal(source->id, id))
            return source;
    }
    return NULL;
}

static NativeInputSource* ibus_input_source_by_id(GnoblinNativeControl* control, const char* id) {
    for (guint i = 0; control->ibus_sources && i < control->ibus_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->ibus_sources, i);
        if (g_str_equal(source->id, id))
            return source;
    }
    return NULL;
}

NativeInputSource* gnoblin_control_current_input_source(GnoblinNativeControl* control) {
    if ((!control->active_input_source_type ||
         g_str_equal(control->active_input_source_type, "ibus")) &&
        control->current_ibus_source_id) {
        for (guint i = 0; control->ibus_sources && i < control->ibus_sources->len; i++) {
            NativeInputSource* source = g_ptr_array_index(control->ibus_sources, i);
            if (g_str_equal(source->id, control->current_ibus_source_id))
                return source;
        }
        return NULL;
    }
    if (control->active_input_source_type && g_str_equal(control->active_input_source_type, "ibus"))
        return NULL;
    if (!control->backend || !control->input_keymap_description ||
        meta_backend_get_keymap_description(control->backend) !=
            control->input_keymap_description ||
        !control->active_input_source_ids)
        return NULL;
    guint group = meta_backend_get_keymap_layout_group(control->backend);
    if (group >= control->active_input_source_ids->len)
        return NULL;
    return input_source_by_id(control, g_ptr_array_index(control->active_input_source_ids, group));
}

GVariant* gnoblin_control_input_source_snapshot(GnoblinNativeControl* control) {
    GVariantBuilder sources;
    GVariantBuilder snapshot;
    g_variant_builder_init(&sources, G_VARIANT_TYPE("av"));
    NativeInputSource* current = gnoblin_control_current_input_source(control);
    for (guint i = 0; control->input_sources && i < control->input_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->input_sources, i);
        g_variant_builder_add_value(&sources,
                                    g_variant_new_variant(input_source_record_variant(
                                        source, control->state_revision, source == current)));
    }
    for (guint i = 0; control->ibus_sources && i < control->ibus_sources->len; i++) {
        NativeInputSource* source = g_ptr_array_index(control->ibus_sources, i);
        g_variant_builder_add_value(&sources,
                                    g_variant_new_variant(input_source_record_variant(
                                        source, control->state_revision, source == current)));
    }

    g_variant_builder_init(&snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&snapshot, "{sv}", "sources", g_variant_builder_end(&sources));
    if (current)
        g_variant_builder_add(&snapshot, "{sv}", "current",
                              input_source_record_variant(current, control->state_revision, TRUE));
    g_variant_builder_add(&snapshot, "{sv}", "revision",
                          g_variant_new_int64(control->state_revision));
    return g_variant_ref_sink(g_variant_builder_end(&snapshot));
}

static void add_configured_input_source_id(GPtrArray* ids, const char* id) {
    if (!id || !*id)
        return;
    for (guint i = 0; i < ids->len; i++) {
        if (g_str_equal(g_ptr_array_index(ids, i), id))
            return;
    }
    g_ptr_array_add(ids, g_strdup(id));
}

static GPtrArray* read_configured_input_source_ids(GVariant* document, const char* source_type) {
    GPtrArray* ids = g_ptr_array_new_with_free_func(g_free);
    g_autoptr(GVariant) config =
        document ? g_variant_lookup_value(document, "input-sources", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) configured =
        config ? g_variant_lookup_value(config, "sources", G_VARIANT_TYPE("av")) : NULL;
    if (configured) {
        for (gsize i = 0; i < g_variant_n_children(configured); i++) {
            g_autoptr(GVariant) wrapped = g_variant_get_child_value(configured, i);
            g_autoptr(GVariant) record = g_variant_is_of_type(wrapped, G_VARIANT_TYPE_VARIANT)
                                             ? g_variant_get_variant(wrapped)
                                             : g_variant_ref(wrapped);
            const char* type = NULL;
            const char* id = NULL;
            if (!g_variant_lookup(record, "type", "&s", &type) ||
                !g_variant_lookup(record, "id", "&s", &id) || !g_str_equal(type, source_type))
                continue;
            add_configured_input_source_id(ids, id);
        }
        return ids;
    }
    return ids;
}

static gboolean input_source_id_lists_equal(GPtrArray* first, GPtrArray* second) {
    if (!first || !second || first->len != second->len)
        return FALSE;
    for (guint i = 0; i < first->len; i++) {
        if (!g_str_equal(g_ptr_array_index(first, i), g_ptr_array_index(second, i)))
            return FALSE;
    }
    return TRUE;
}

void gnoblin_control_window_input_source_free(gpointer data) {
    NativeWindowInputSource* source = data;
    if (!source)
        return;
    g_free(source->type);
    g_free(source->id);
    g_free(source);
}

static gboolean input_source_is_configured(GnoblinNativeControl* control, const char* type,
                                           const char* id) {
    return g_str_equal(type, "ibus") ? ibus_input_source_by_id(control, id) != NULL
                                     : input_source_by_id(control, id) != NULL;
}

static gboolean input_window_is_managed(GnoblinNativeControl* control, const char* window_id) {
    if (!window_id || !control->windows)
        return FALSE;
    GHashTableIter iter;
    gpointer window;
    g_hash_table_iter_init(&iter, control->windows);
    while (g_hash_table_iter_next(&iter, &window, NULL)) {
        g_autofree char* candidate_id = gnoblin_control_native_window_id(META_WINDOW(window));
        if (g_str_equal(candidate_id, window_id))
            return TRUE;
    }
    return FALSE;
}

static void remember_window_input_source(GnoblinNativeControl* control, const char* window_id,
                                         const char* type, const char* id) {
    if (!control->per_window_input_sources || !window_id || !*window_id || !type || !id ||
        !input_window_is_managed(control, window_id) ||
        !input_source_is_configured(control, type, id))
        return;
    NativeWindowInputSource* source = g_new0(NativeWindowInputSource, 1);
    source->type = g_strdup(type);
    source->id = g_strdup(id);
    g_hash_table_replace(control->window_input_sources, g_strdup(window_id), source);
}

static NativeWindowInputSource* remembered_window_input_source(GnoblinNativeControl* control,
                                                               const char* window_id) {
    return window_id && control->window_input_sources
               ? g_hash_table_lookup(control->window_input_sources, window_id)
               : NULL;
}

static void prune_window_input_sources(GnoblinNativeControl* control) {
    if (!control->window_input_sources)
        return;
    GHashTableIter iter;
    gpointer key;
    gpointer value;
    g_hash_table_iter_init(&iter, control->window_input_sources);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        NativeWindowInputSource* source = value;
        if (!control->per_window_input_sources ||
            !input_source_is_configured(control, source->type, source->id))
            g_hash_table_iter_remove(&iter);
    }
}

gboolean gnoblin_control_input_sources_refresh(GnoblinNativeControl* control, GVariant* document) {
    g_autoptr(GVariant) current_document = NULL;
    if (!document) {
        current_document = gnoblin_control_native_config_document(control);
        document = current_document;
    }
    GPtrArray* xkb_ids = read_configured_input_source_ids(document, "xkb");
    GPtrArray* ibus_ids = read_configured_input_source_ids(document, "ibus");
    g_autoptr(GVariant) config =
        document ? g_variant_lookup_value(document, "input-sources", G_VARIANT_TYPE_VARDICT) : NULL;
    gboolean per_window = FALSE;
    if (config)
        g_variant_lookup(config, "per-window", "b", &per_window);
    gboolean changed =
        !input_source_id_lists_equal(control->configured_input_source_ids, xkb_ids) ||
        !input_source_id_lists_equal(control->configured_ibus_source_ids, ibus_ids) ||
        control->per_window_input_sources != per_window;
    control->per_window_input_sources = per_window;
    if (!changed) {
        prune_window_input_sources(control);
        g_ptr_array_unref(xkb_ids);
        g_ptr_array_unref(ibus_ids);
        return FALSE;
    }
    g_clear_pointer(&control->configured_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->configured_ibus_source_ids, g_ptr_array_unref);
    control->configured_input_source_ids = xkb_ids;
    control->configured_ibus_source_ids = ibus_ids;

    GPtrArray* sources = g_ptr_array_new_with_free_func(native_input_source_free);
    for (guint i = 0; i < xkb_ids->len; i++) {
        const char* id = g_ptr_array_index(xkb_ids, i);
        NativeInputSource* source = input_source_new(id);
        if (source)
            g_ptr_array_add(sources, source);
    }
    g_clear_pointer(&control->input_sources, g_ptr_array_unref);
    control->input_sources = sources;
    sources = g_ptr_array_new_with_free_func(native_input_source_free);
    for (guint i = 0; i < ibus_ids->len; i++) {
        const char* id = g_ptr_array_index(ibus_ids, i);
        NativeInputSource* source = ibus_input_source_new(id);
        if (source)
            g_ptr_array_add(sources, source);
    }
    g_clear_pointer(&control->ibus_sources, g_ptr_array_unref);
    control->ibus_sources = sources;
    prune_window_input_sources(control);
    return TRUE;
}

static void pending_input_source_free(PendingInputSource* pending) {
    if (!pending)
        return;
    g_clear_pointer(&pending->description, meta_keymap_description_unref);
    g_clear_pointer(&pending->source_ids, g_ptr_array_unref);
    g_free(pending->selected_type);
    g_free(pending->selected_id);
    g_free(pending->target_window_id);
    g_free(pending->method);
    g_free(pending);
}

void gnoblin_control_input_sources_release(GnoblinNativeControl* control) {
    g_clear_pointer(&control->input_sources, g_ptr_array_unref);
    g_clear_pointer(&control->ibus_sources, g_ptr_array_unref);
    g_clear_pointer(&control->configured_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->configured_ibus_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->active_input_source_ids, g_ptr_array_unref);
    g_clear_pointer(&control->input_keymap_description, meta_keymap_description_unref);
    g_clear_object(&control->appearance_settings);
    g_clear_pointer(&control->last_published_input_source, g_free);
    g_clear_pointer(&control->current_ibus_source_id, g_free);
    g_clear_pointer(&control->active_input_source_type, g_free);
    g_clear_pointer(&control->focused_input_window_id, g_free);
}

static void dispatch_input_source_operation(GnoblinNativeControl* control, gint64 request_id,
                                            const char* method, const char* source_type,
                                            gboolean ok, const char* selected_id,
                                            const char* error_code, const char* message) {
    g_autoptr(JsonNode) result = NULL;
    if (ok && selected_id) {
        NativeInputSource* source = g_str_equal(source_type, "ibus")
                                        ? ibus_input_source_by_id(control, selected_id)
                                        : input_source_by_id(control, selected_id);
        if (source) {
            g_autoptr(GVariant) record =
                input_source_record_variant(source, control->state_revision, TRUE);
            result = gnoblin_control_json_from_variant(record);
        }
    }
    gnoblin_control_dispatch_operation_completion_full(control, request_id, method, ok, result, error_code, message,
                                       TRUE);
}

static void remember_completed_window_input_source(GnoblinNativeControl* control,
                                                   PendingInputSource* pending,
                                                   gboolean confirmed) {
    if (confirmed)
        remember_window_input_source(control, pending->target_window_id, pending->selected_type,
                                     pending->selected_id);
}

static gboolean should_restore_after_input_source_completion(GnoblinNativeControl* control,
                                                             PendingInputSource* pending) {
    return !pending->internal_restore ||
           g_strcmp0(pending->target_window_id, control->focused_input_window_id) != 0;
}

static char* ibus_engine_name_from_value(GVariant* value) {
    if (!value)
        return NULL;
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) unwrapped = g_variant_get_variant(value);
        return ibus_engine_name_from_value(unwrapped);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        const char* name = NULL;
        if (g_variant_lookup(value, "name", "&s", &name) && name && *name)
            return g_strdup(name);
        return NULL;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_TUPLE) && g_variant_n_children(value) >= 3) {
        g_autoptr(GVariant) type_value = g_variant_get_child_value(value, 0);
        if (g_variant_is_of_type(type_value, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(type_value, NULL), "IBusEngineDesc")) {
            g_autoptr(GVariant) name_value = g_variant_get_child_value(value, 2);
            if (g_variant_is_of_type(name_value, G_VARIANT_TYPE_STRING)) {
                const char* name = g_variant_get_string(name_value, NULL);
                return *name ? g_strdup(name) : NULL;
            }
        }
    }
    if (g_variant_is_container(value)) {
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) child = g_variant_get_child_value(value, i);
            char* name = ibus_engine_name_from_value(child);
            if (name)
                return name;
        }
    }
    return NULL;
}

static char* ibus_bus_address(void) {
    const char* address = g_getenv("IBUS_ADDRESS");
    if (address && *address)
        return g_strdup(address);

    const char* display = g_getenv("WAYLAND_DISPLAY");
    if (!display || !*display)
        display = g_getenv("DISPLAY");
    if (!display || !*display)
        return NULL;

    g_autofree char* display_name = g_path_get_basename(display);
    if (display_name[0] == ':')
        memmove(display_name, display_name + 1, strlen(display_name));
    g_autofree char* machine_id = NULL;
    if (!g_file_get_contents("/etc/machine-id", &machine_id, NULL, NULL) || !machine_id) {
        g_clear_pointer(&machine_id, g_free);
        if (!g_file_get_contents("/var/lib/dbus/machine-id", &machine_id, NULL, NULL))
            return NULL;
    }
    g_strstrip(machine_id);
    if (!*machine_id)
        return NULL;
    g_autofree char* filename = g_strdup_printf("%s-unix-%s", machine_id, display_name);
    const char* address_dirs[] = {g_get_user_cache_dir(), g_get_user_config_dir()};
    for (guint i = 0; i < G_N_ELEMENTS(address_dirs); i++) {
        g_autofree char* path = g_build_filename(address_dirs[i], "ibus", "bus", filename, NULL);
        g_autofree char* contents = NULL;
        if (!g_file_get_contents(path, &contents, NULL, NULL))
            continue;

        g_auto(GStrv) lines = g_strsplit(contents, "\n", -1);
        for (char** line = lines; *line; line++) {
            if (!g_str_has_prefix(*line, "IBUS_ADDRESS="))
                continue;
            const char* value = *line + strlen("IBUS_ADDRESS=");
            return *value ? g_strdup(value) : NULL;
        }
    }
    g_debug("gnoblin-native-control: IBus address file was not found under the XDG cache or config "
            "directories");
    return NULL;
}

static GDBusConnection* connect_ibus_bus(void) {
    g_autofree char* address = ibus_bus_address();
    if (!address)
        return NULL;

    g_autoptr(GError) error = NULL;
    GDBusConnection* connection =
        g_dbus_connection_new_for_address_sync(address,
                                               G_DBUS_CONNECTION_FLAGS_AUTHENTICATION_CLIENT |
                                                   G_DBUS_CONNECTION_FLAGS_MESSAGE_BUS_CONNECTION,
                                               NULL, NULL, &error);
    if (!connection)
        g_debug("gnoblin-native-control: IBus is unavailable: %s", error->message);
    return connection;
}

static gboolean ibus_retry_connection(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->ibus_retry_source_id = 0;
    if (!control->stopping)
        gnoblin_control_input_sources_start_ibus(control);
    return G_SOURCE_REMOVE;
}

static void schedule_ibus_connection_retry(GnoblinNativeControl* control) {
    if (control->stopping || control->ibus_bus || control->ibus_retry_source_id)
        return;
    control->ibus_retry_source_id = g_timeout_add_seconds(1, ibus_retry_connection, control);
}

static void ibus_bus_closed(GDBusConnection* connection, gboolean remote_peer_vanished,
                            GError* error, gpointer user_data) {
    (void)remote_peer_vanished;
    (void)error;
    GnoblinNativeControl* control = user_data;
    if (control->stopping || control->ibus_bus != connection)
        return;
    control->ibus_owner_generation++;
    control->ibus_engine_generation++;
    control->input_source_selection_generation++;
    if (control->ibus_signal_subscription_id) {
        g_dbus_connection_signal_unsubscribe(connection, control->ibus_signal_subscription_id);
        control->ibus_signal_subscription_id = 0;
    }
    if (control->ibus_owner_subscription_id) {
        g_dbus_connection_signal_unsubscribe(connection, control->ibus_owner_subscription_id);
        control->ibus_owner_subscription_id = 0;
    }
    g_signal_handlers_disconnect_by_data(connection, control);
    g_clear_object(&control->ibus_bus);
    g_clear_pointer(&control->current_ibus_source_id, g_free);
    if (g_strcmp0(control->active_input_source_type, "ibus") == 0) {
        g_free(control->active_input_source_type);
        control->active_input_source_type = NULL;
    }
    gnoblin_control_publish_input_source_changes(control, control->state_revision);
    schedule_ibus_connection_retry(control);
}

static void ibus_global_engine_query_done(GObject* source_object, GAsyncResult* result,
                                          gpointer user_data) {
    PendingIBusQuery* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source_object), result, &error);
    control->pending_ibus_queries--;
    gboolean current = !control->stopping &&
                       control->ibus_bus == G_DBUS_CONNECTION(source_object) &&
                       control->ibus_owner_generation == pending->ibus_owner_generation &&
                       control->ibus_engine_generation == pending->ibus_engine_generation;
    if (current) {
        g_autofree char* engine = reply ? ibus_engine_name_from_value(reply) : NULL;
        gboolean selection_unchanged = control->input_source_selection_generation ==
                                       pending->input_source_selection_generation;
        gboolean configured_engine = engine && ibus_input_source_by_id(control, engine);
        g_free(control->current_ibus_source_id);
        control->current_ibus_source_id = configured_engine ? g_strdup(engine) : NULL;
        if (selection_unchanged) {
            if (configured_engine) {
                if (g_strcmp0(control->active_input_source_type, "xkb") != 0) {
                    g_free(control->active_input_source_type);
                    control->active_input_source_type = g_strdup("ibus");
                }
            } else if (g_strcmp0(control->active_input_source_type, "ibus") == 0) {
                g_free(control->active_input_source_type);
                control->active_input_source_type = NULL;
            }
            control->input_source_selection_generation++;
        }
        gnoblin_control_publish_input_source_changes(control, control->state_revision);
        gnoblin_control_input_source_restore_focused(control);
    }
    g_free(pending);
    gnoblin_control_maybe_free_stopped(control);
}

static void query_ibus_global_engine(GnoblinNativeControl* control) {
    if (!control || control->stopping || !control->ibus_bus)
        return;
    PendingIBusQuery* pending = g_new0(PendingIBusQuery, 1);
    pending->control = control;
    pending->ibus_owner_generation = control->ibus_owner_generation;
    pending->ibus_engine_generation = control->ibus_engine_generation;
    pending->input_source_selection_generation = control->input_source_selection_generation;
    control->pending_ibus_queries++;
    g_dbus_connection_call(control->ibus_bus, IBUS_BUS_NAME, IBUS_OBJECT_PATH, IBUS_INTERFACE,
                           "GetGlobalEngine", NULL, G_VARIANT_TYPE("(v)"), G_DBUS_CALL_FLAGS_NONE,
                           1000, NULL, ibus_global_engine_query_done, pending);
}

static void ibus_global_engine_changed(GDBusConnection* connection, const char* sender_name,
                                       const char* object_path, const char* interface_name,
                                       const char* signal_name, GVariant* parameters,
                                       gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    const char* engine = NULL;
    if (control->stopping || control->ibus_bus != connection)
        return;
    control->ibus_engine_generation++;
    g_variant_get(parameters, "(&s)", &engine);
    g_free(control->current_ibus_source_id);
    gboolean configured_engine = *engine && ibus_input_source_by_id(control, engine);
    control->current_ibus_source_id = configured_engine ? g_strdup(engine) : NULL;
    if (configured_engine) {
        g_free(control->active_input_source_type);
        control->active_input_source_type = g_strdup("ibus");
    } else if (g_strcmp0(control->active_input_source_type, "ibus") == 0) {
        g_free(control->active_input_source_type);
        control->active_input_source_type = NULL;
    }
    control->input_source_selection_generation++;
    gnoblin_control_publish_input_source_changes(control, control->state_revision);
}

static void ibus_name_owner_changed(GDBusConnection* connection, const char* sender_name,
                                    const char* object_path, const char* interface_name,
                                    const char* signal_name, GVariant* parameters,
                                    gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    const char* name = NULL;
    const char* old_owner = NULL;
    const char* new_owner = NULL;
    g_variant_get(parameters, "(&s&s&s)", &name, &old_owner, &new_owner);
    if (control->stopping || control->ibus_bus != connection || !g_str_equal(name, IBUS_BUS_NAME) ||
        g_str_equal(old_owner, new_owner))
        return;
    control->ibus_owner_generation++;
    control->ibus_engine_generation++;
    control->input_source_selection_generation++;
    g_clear_pointer(&control->current_ibus_source_id, g_free);
    if (g_strcmp0(control->active_input_source_type, "ibus") == 0) {
        g_free(control->active_input_source_type);
        control->active_input_source_type = NULL;
    }
    gnoblin_control_publish_input_source_changes(control, control->state_revision);
    if (*new_owner)
        query_ibus_global_engine(control);
}

void gnoblin_control_input_sources_start_ibus(GnoblinNativeControl* control) {
    if (!control || control->stopping || control->ibus_bus)
        return;
    control->ibus_bus = connect_ibus_bus();
    if (!control->ibus_bus) {
        schedule_ibus_connection_retry(control);
        return;
    }
    control->ibus_owner_generation++;
    control->ibus_engine_generation++;
    g_signal_connect(control->ibus_bus, "closed", G_CALLBACK(ibus_bus_closed), control);
    control->ibus_signal_subscription_id = g_dbus_connection_signal_subscribe(
        control->ibus_bus, IBUS_BUS_NAME, IBUS_INTERFACE, "GlobalEngineChanged", IBUS_OBJECT_PATH,
        NULL, G_DBUS_SIGNAL_FLAGS_NONE, ibus_global_engine_changed, control, NULL);
    control->ibus_owner_subscription_id = g_dbus_connection_signal_subscribe(
        control->ibus_bus, "org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
        "/org/freedesktop/DBus", IBUS_BUS_NAME, G_DBUS_SIGNAL_FLAGS_NONE, ibus_name_owner_changed,
        control, NULL);
    g_autoptr(GVariant) owner = g_dbus_connection_call_sync(
        control->ibus_bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
        "GetNameOwner", g_variant_new("(s)", IBUS_BUS_NAME), G_VARIANT_TYPE("(s)"),
        G_DBUS_CALL_FLAGS_NONE, 250, NULL, NULL);
    if (owner)
        query_ibus_global_engine(control);
}

static void input_source_keymap_set_done(GObject* source_object, GAsyncResult* result,
                                         gpointer user_data) {
    PendingInputSource* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    gboolean ok = meta_backend_set_keymap_finish(META_BACKEND(source_object), result, &error);
    if (control->stopping) {
        control->pending_input_source_ops--;
        pending_input_source_free(pending);
        gnoblin_control_maybe_free_stopped(control);
        return;
    }
    control->pending_input_source_ops--;
    NativeInputSource* selected = input_source_by_id(control, pending->selected_id);
    gboolean confirmed =
        ok && selected &&
        meta_backend_get_keymap_description(control->backend) == pending->description &&
        meta_backend_get_keymap_layout_group(control->backend) == pending->group;
    if (!confirmed)
        g_message("gnoblin-native-control: input source %s was not confirmed: keymap set %s%s%s, "
                  "selected source %s, backend description %s, layout group %u (wanted %u)",
                  pending->selected_id ? pending->selected_id : "(none)", ok ? "ok" : "failed",
                  error ? ": " : "", error ? error->message : "", selected ? "found" : "missing",
                  meta_backend_get_keymap_description(control->backend) == pending->description
                      ? "matches"
                      : "differs",
                  (guint)meta_backend_get_keymap_layout_group(control->backend), (guint)pending->group);
    if (confirmed) {
        g_clear_pointer(&control->input_keymap_description, meta_keymap_description_unref);
        control->input_keymap_description = meta_keymap_description_ref(pending->description);
        g_clear_pointer(&control->active_input_source_ids, g_ptr_array_unref);
        control->active_input_source_ids = g_steal_pointer(&pending->source_ids);
        /* An IBus engine signal received while this request was in flight is
         * newer evidence of the globally active source than the XKB request. */
        if (control->ibus_engine_generation == pending->ibus_engine_generation) {
            g_free(control->active_input_source_type);
            control->active_input_source_type = g_strdup("xkb");
            control->input_source_selection_generation++;
        }
        gnoblin_control_schedule_windows(control);
    }
    if (!confirmed && ok)
        error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_FAILED,
                                    "Mutter did not confirm the requested keymap group");
    gnoblin_control_publish_input_source_changes(control, control->state_revision);
    remember_completed_window_input_source(control, pending, confirmed);
    if (!pending->internal_restore)
        dispatch_input_source_operation(control, pending->request_id, pending->method,
                                        pending->selected_type, confirmed,
                                        confirmed ? pending->selected_id : NULL,
                                        confirmed ? NULL : gnoblin_control_operation_error_code(error),
                                        confirmed ? NULL
                                        : error   ? error->message
                                                  : "keymap request failed");
    /* Focus may have changed while Mutter was completing the request. Reconcile
     * only after reporting the selection result, so its result reflects the
     * source that was actually confirmed for the requesting window. */
    if (should_restore_after_input_source_completion(control, pending))
        gnoblin_control_input_source_restore_focused(control);
    pending_input_source_free(pending);
}

gboolean gnoblin_control_select_xkb_source(GnoblinNativeControl* control, const char* id, gint64 request_id,
                                  const char* method, gboolean internal_restore, GError** error) {
    NativeInputSource* selected = input_source_by_id(control, id);
    if (!selected) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "XKB input source is not configured or available: %s", id);
        return FALSE;
    }
    if (control->pending_input_source_ops > 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input source selection is still pending");
        return FALSE;
    }
    guint selected_index = 0;
    while (selected_index < control->input_sources->len &&
           g_ptr_array_index(control->input_sources, selected_index) != selected)
        selected_index++;
    guint chunk_start = (selected_index / 4) * 4;
    guint chunk_length = MIN(4, control->input_sources->len - chunk_start);
    GPtrArray* ids = g_ptr_array_new_with_free_func(g_free);
    g_auto(GStrv) layouts = g_new0(char*, chunk_length + 1);
    g_auto(GStrv) variants = g_new0(char*, chunk_length + 1);
    g_auto(GStrv) display_names = g_new0(char*, chunk_length + 1);
    g_auto(GStrv) short_names = g_new0(char*, chunk_length + 1);
    for (guint i = 0; i < chunk_length; i++) {
        NativeInputSource* source = g_ptr_array_index(control->input_sources, chunk_start + i);
        layouts[i] = g_strdup(source->layout);
        variants[i] = g_strdup(source->variant);
        display_names[i] = g_strdup(source->name);
        short_names[i] = g_strdup(source->short_name);
        g_ptr_array_add(ids, g_strdup(source->id));
    }
    g_autofree char* layout_list = g_strjoinv(",", layouts);
    g_autofree char* variant_list = g_strjoinv(",", variants);
    g_autofree char* options = NULL;
    g_autoptr(GVariant) document = gnoblin_control_native_config_document(control);
    g_autoptr(GVariant) input =
        document ? g_variant_lookup_value(document, "input", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) keyboard =
        input ? g_variant_lookup_value(input, "keyboard", G_VARIANT_TYPE_VARDICT) : NULL;
    g_autoptr(GVariant) xkb_options =
        keyboard ? g_variant_lookup_value(keyboard, "xkb-options", G_VARIANT_TYPE("as")) : NULL;
    if (xkb_options) {
        GPtrArray* option_parts = g_ptr_array_new_with_free_func(g_free);
        for (gsize i = 0; i < g_variant_n_children(xkb_options); i++) {
            g_autoptr(GVariant) option = g_variant_get_child_value(xkb_options, i);
            g_ptr_array_add(option_parts, g_strdup(g_variant_get_string(option, NULL)));
        }
        g_ptr_array_add(option_parts, NULL);
        options = g_strjoinv(",", (char**)option_parts->pdata);
        g_ptr_array_unref(option_parts);
    }
    MetaKeymapDescription* description = meta_keymap_description_new_from_rules(
        NULL, layout_list, variant_list, options, display_names, short_names);
    if (!description) {
        g_ptr_array_unref(ids);
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "cannot construct the requested XKB keymap");
        return FALSE;
    }
    g_autoptr(GError) compile_error = NULL;
    struct xkb_keymap* keymap =
        meta_keymap_description_create_xkb_keymap(description, NULL, NULL, &compile_error);
    if (!keymap) {
        meta_keymap_description_unref(description);
        g_ptr_array_unref(ids);
        g_propagate_error(error, g_steal_pointer(&compile_error));
        return FALSE;
    }
    xkb_keymap_unref(keymap);
    PendingInputSource* pending = g_new0(PendingInputSource, 1);
    pending->control = control;
    pending->description = description;
    pending->source_ids = ids;
    pending->selected_type = g_strdup("xkb");
    pending->selected_id = g_strdup(id);
    pending->target_window_id = g_strdup(control->focused_input_window_id);
    pending->internal_restore = internal_restore;
    pending->ibus_engine_generation = control->ibus_engine_generation;
    pending->request_id = request_id;
    pending->method = g_strdup(method);
    pending->group = selected_index - chunk_start;
    control->pending_input_source_ops++;
    meta_backend_set_keymap_async(control->backend, description, pending->group, NULL,
                                  input_source_keymap_set_done, pending);
    return TRUE;
}

static void complete_ibus_input_source_selection(GnoblinNativeControl* control,
                                                 PendingInputSource* pending, gboolean confirmed,
                                                 const GError* error) {
    control->pending_input_source_ops--;
    gboolean selected_source_still_configured =
        ibus_input_source_by_id(control, pending->selected_id) != NULL;
    confirmed = confirmed && selected_source_still_configured;
    if (confirmed) {
        g_free(control->active_input_source_type);
        control->active_input_source_type = g_strdup("ibus");
        control->input_source_selection_generation++;
        gnoblin_control_publish_input_source_changes(control, control->state_revision);
        remember_completed_window_input_source(control, pending, TRUE);
    }

    if (!control->stopping && !pending->internal_restore)
        dispatch_input_source_operation(control, pending->request_id, pending->method, "ibus",
                                        confirmed, confirmed ? pending->selected_id : NULL,
                                        confirmed ? NULL : gnoblin_control_operation_error_code(error),
                                        confirmed ? NULL
                                        : error   ? error->message
                                                  : "IBus selection was not confirmed");
    if (!control->stopping && should_restore_after_input_source_completion(control, pending))
        gnoblin_control_input_source_restore_focused(control);
    pending_input_source_free(pending);
    gnoblin_control_maybe_free_stopped(control);
}

static void ibus_input_source_get_engine_done(GObject* source_object, GAsyncResult* result,
                                              gpointer user_data) {
    PendingInputSource* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source_object), result, &error);
    gboolean current = !control->stopping &&
                       control->ibus_bus == G_DBUS_CONNECTION(source_object) &&
                       control->ibus_owner_generation == pending->ibus_owner_generation;
    g_autofree char* engine = reply ? ibus_engine_name_from_value(reply) : NULL;
    gboolean signal_matches_query =
        control->ibus_engine_generation == pending->ibus_engine_generation ||
        g_strcmp0(control->current_ibus_source_id, pending->selected_id) == 0;
    gboolean confirmed = current && reply && engine && g_str_equal(engine, pending->selected_id) &&
                         signal_matches_query &&
                         ibus_input_source_by_id(control, pending->selected_id);

    if (!confirmed && !control->stopping && !current) {
        g_clear_error(&error);
        error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                                    "IBus owner changed before source selection was confirmed");
    } else if (!confirmed && !control->stopping && reply &&
               !ibus_input_source_by_id(control, pending->selected_id)) {
        error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                    "IBus source was removed before selection was confirmed");
    } else if (!confirmed && !control->stopping && reply) {
        error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_FAILED,
                                    "IBus did not confirm the requested global engine");
    }

    if (confirmed) {
        g_free(control->current_ibus_source_id);
        control->current_ibus_source_id = g_strdup(engine);
    }
    complete_ibus_input_source_selection(control, pending, confirmed, error);
}

static void ibus_input_source_set_done(GObject* source_object, GAsyncResult* result,
                                       gpointer user_data) {
    PendingInputSource* pending = user_data;
    GnoblinNativeControl* control = pending->control;
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) reply =
        g_dbus_connection_call_finish(G_DBUS_CONNECTION(source_object), result, &error);
    gboolean current = !control->stopping &&
                       control->ibus_bus == G_DBUS_CONNECTION(source_object) &&
                       control->ibus_owner_generation == pending->ibus_owner_generation;
    if (!current) {
        if (!control->stopping) {
            g_clear_error(&error);
            error = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                                        "IBus owner changed before source selection completed");
        }
        complete_ibus_input_source_selection(control, pending, FALSE, error);
        return;
    }
    if (!reply) {
        complete_ibus_input_source_selection(control, pending, FALSE, error);
        return;
    }

    /* SetGlobalEngine acknowledges the request but does not return the active
     * engine. Query the same owner before treating the selection as confirmed. */
    pending->ibus_engine_generation = control->ibus_engine_generation;
    g_dbus_connection_call(control->ibus_bus, IBUS_BUS_NAME, IBUS_OBJECT_PATH, IBUS_INTERFACE,
                           "GetGlobalEngine", NULL, G_VARIANT_TYPE("(v)"), G_DBUS_CALL_FLAGS_NONE,
                           1000, NULL, ibus_input_source_get_engine_done, pending);
}

static gboolean select_ibus_source(GnoblinNativeControl* control, const char* id, gint64 request_id,
                                   const char* method, gboolean internal_restore, GError** error) {
    if (!ibus_input_source_by_id(control, id)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "IBus input source is not configured: %s", id);
        return FALSE;
    }
    if (control->pending_input_source_ops > 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_BUSY,
                            "another input source selection is still pending");
        return FALSE;
    }
    if (!control->ibus_bus) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_CONNECTED,
                            "the IBus service is unavailable");
        return FALSE;
    }
    PendingInputSource* pending = g_new0(PendingInputSource, 1);
    pending->control = control;
    pending->selected_type = g_strdup("ibus");
    pending->selected_id = g_strdup(id);
    pending->target_window_id = g_strdup(control->focused_input_window_id);
    pending->request_id = request_id;
    pending->method = g_strdup(method);
    pending->ibus_owner_generation = control->ibus_owner_generation;
    pending->internal_restore = internal_restore;
    control->pending_input_source_ops++;
    g_dbus_connection_call(control->ibus_bus, IBUS_BUS_NAME, IBUS_OBJECT_PATH, IBUS_INTERFACE,
                           "SetGlobalEngine", g_variant_new("(s)", id), G_VARIANT_TYPE_UNIT,
                           G_DBUS_CALL_FLAGS_NONE, 5000, NULL, ibus_input_source_set_done, pending);
    return TRUE;
}

gboolean gnoblin_native_control_select_input_source(MetaDisplay* display, GVariant* arguments,
                                                    gint64 request_id, const char* method,
                                                    GError** error) {
    GnoblinNativeControl* control =
        display ? g_object_get_data(G_OBJECT(display), NATIVE_CONTROL_OBJECT_DATA_KEY) : NULL;
    if (!control || control->stopping) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_INITIALIZED,
                            "native input control is not available");
        return FALSE;
    }

    const char* type = NULL;
    const char* source_id = NULL;
    if (!arguments || !g_variant_lookup(arguments, "type", "&s", &type) ||
        !g_variant_lookup(arguments, "id", "&s", &source_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input source selection requires type and id");
        return FALSE;
    }
    if (g_str_equal(type, "ibus")) {
        return select_ibus_source(control, source_id, request_id, method, FALSE, error);
    }
    if (!g_str_equal(type, "xkb")) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input source type must be 'xkb' or 'ibus'");
        return FALSE;
    }

    return gnoblin_control_select_xkb_source(control, source_id, request_id, method, FALSE, error);
}

void gnoblin_control_input_source_restore_focused(GnoblinNativeControl* control) {
    if (!control || control->stopping || !control->per_window_input_sources ||
        control->pending_input_source_ops > 0)
        return;
    const char* window_id = control->focused_input_window_id;
    if (!window_id || !input_window_is_managed(control, window_id))
        return;

    NativeWindowInputSource* remembered = remembered_window_input_source(control, window_id);
    NativeInputSource* current = gnoblin_control_current_input_source(control);
    if (!remembered) {
        /* A first-seen window starts with the source already active. */
        if (current)
            remember_window_input_source(control, window_id, current->type, current->id);
        return;
    }
    if (!input_source_is_configured(control, remembered->type, remembered->id)) {
        g_hash_table_remove(control->window_input_sources, window_id);
        return;
    }
    if (current && g_str_equal(current->type, remembered->type) &&
        g_str_equal(current->id, remembered->id))
        return;

    g_autoptr(GError) error = NULL;
    gboolean started =
        g_str_equal(remembered->type, "ibus")
            ? select_ibus_source(control, remembered->id, 0, "input.select_source", TRUE, &error)
            : gnoblin_control_select_xkb_source(control, remembered->id, 0, "input.select_source", TRUE, &error);
    if (!started && error && !g_error_matches(error, G_IO_ERROR, G_IO_ERROR_BUSY))
        g_debug("gnoblin-native-control: cannot restore input source for window %s: %s", window_id,
                error->message);
}

void gnoblin_control_input_source_focus_changed(GnoblinNativeControl* control) {
    if (!control || control->stopping)
        return;
    MetaWindow* focused = meta_display_get_focus_window(control->display);
    g_autofree char* next_id = focused ? gnoblin_control_native_window_id(focused) : NULL;
    if (g_strcmp0(control->focused_input_window_id, next_id) == 0)
        return;

    /* Capture the departing window before updating identity or starting an
     * asynchronous switch. A pending user selection will overwrite this with
     * its confirmed source in its completion callback. */
    if (control->per_window_input_sources && control->focused_input_window_id) {
        NativeInputSource* current = gnoblin_control_current_input_source(control);
        if (current)
            remember_window_input_source(control, control->focused_input_window_id, current->type,
                                         current->id);
    }
    g_free(control->focused_input_window_id);
    control->focused_input_window_id = g_steal_pointer(&next_id);
    if (control->per_window_input_sources)
        gnoblin_control_input_source_restore_focused(control);
}

static void dispatch_lua_input_sources_changed(GnoblinNativeControl* control, guint64 revision) {
    g_autoptr(GVariant) snapshot = gnoblin_control_input_source_snapshot(control);
    gnoblin_control_publish_runtime_snapshot(control, "input-sources", snapshot, revision);
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.input.sources-changed");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    g_autoptr(JsonNode) json = gnoblin_control_json_from_variant(snapshot);
    json_object_set_member(
        object, "sources",
        json_node_copy(json_object_get_member(json_node_get_object(json), "sources")));
    gnoblin_control_publish_socket_event(control, root);
    g_autoptr(GVariant) payload = gnoblin_control_variant_from_json(root);
    if (payload)
        gnoblin_control_dispatch_event(control, "gnoblin.input.sources-changed", payload);
}

static void dispatch_lua_input_source_changed(GnoblinNativeControl* control, guint64 revision,
                                              NativeInputSource* source) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "name", "gnoblin.input.source-changed");
    json_object_set_int_member(object, "revision", revision);
    json_object_set_int_member(object, "sequence", ++control->event_sequence);
    json_object_set_int_member(object, "time", g_get_monotonic_time());
    json_object_set_boolean_member(object, "available", source != NULL);
    if (source) {
        g_autoptr(GVariant) record = input_source_record_variant(source, revision, TRUE);
        json_object_set_member(object, "source", gnoblin_control_json_from_variant(record));
    }
    gnoblin_control_publish_socket_event(control, root);
    g_autoptr(GVariant) payload = gnoblin_control_variant_from_json(root);
    if (payload)
        gnoblin_control_dispatch_event(control, "gnoblin.input.source-changed", payload);
}

void gnoblin_control_publish_input_source_changes(GnoblinNativeControl* control, guint64 revision) {
    gboolean sources_changed = gnoblin_control_input_sources_refresh(control, NULL);
    g_autoptr(GVariant) snapshot = gnoblin_control_input_source_snapshot(control);
    gnoblin_control_publish_runtime_snapshot(control, "input-sources", snapshot, revision);
    if (sources_changed && control->input_source_state_initialized)
        dispatch_lua_input_sources_changed(control, revision);
    NativeInputSource* current = gnoblin_control_current_input_source(control);
    g_autofree char* current_key =
        current ? g_strdup_printf("%s:%s", current->type, current->id) : NULL;
    if (control->pending_input_source_ops == 0) {
        if (control->input_source_state_initialized &&
            g_strcmp0(control->last_published_input_source, current_key) != 0)
            dispatch_lua_input_source_changed(control, revision, current);
        g_free(control->last_published_input_source);
        control->last_published_input_source = g_steal_pointer(&current_key);
    }
    control->input_source_state_initialized = TRUE;
}
