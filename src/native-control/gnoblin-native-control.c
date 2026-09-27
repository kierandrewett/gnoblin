/* The first native owner of compositor-v1: Lua-validated compositor methods. */
#include "config.h"

#include "core/gnoblin-native-control.h"

#include <gio/gio.h>
#include <gio/gunixsocketaddress.h>
#include <glib/gstdio.h>
#include <json-glib/json-glib.h>
#include <errno.h>
#include <math.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "meta/display.h"
#include "meta/meta-backend.h"
#include "meta/meta-orientation-manager.h"
#include "meta/prefs.h"
#include "meta/util.h"
#include "meta/window.h"

#define MAX_REQUEST_BYTES (64 * 1024)
#define MAX_PENDING_BYTES (1024 * 1024)

struct _GnoblinNativeControl {
    GSocketService* service;
    GHashTable* clients;
    GHashTable* windows;
    GPtrArray* shortcuts;
    MetaDisplay* display;
    guint publish_id;
    char* path;
    dev_t device;
    ino_t inode;
};

typedef struct {
    GSocketConnection* connection;
    GnoblinNativeControl* control;
    GString* request;
    GQueue* outgoing;
    gsize pending_bytes;
    gboolean reading;
    gboolean writing;
    gboolean closing;
    gboolean track_windows;
    gboolean close_after_response;
} Client;

typedef struct {
    Client* client;
    char* response;
} PendingWrite;

typedef struct {
    char* name;
    char** argv;
} NativeAutostart;

typedef struct {
    char* name;
    char* binding;
    char** argv;
    guint action;
    gboolean overlay;
    gboolean release;
} NativeShortcut;

typedef struct {
    char* name;
    const char* category;
    GSubprocess* process;
} RunningCommand;

static void native_autostart_free(gpointer data) {
    NativeAutostart* entry = data;
    g_free(entry->name);
    g_strfreev(entry->argv);
    g_free(entry);
}

static void native_shortcut_free(gpointer data) {
    NativeShortcut* shortcut = data;
    g_free(shortcut->name);
    g_free(shortcut->binding);
    g_strfreev(shortcut->argv);
    g_free(shortcut);
}

static char** native_command_argv(GVariant* command) {
    if (!command || !g_variant_is_of_type(command, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(command) == 0)
        return NULL;
    char** argv = g_new0(char*, g_variant_n_children(command) + 1);
    for (gsize item = 0; item < g_variant_n_children(command); item++) {
        g_autoptr(GVariant) wrapped = g_variant_get_child_value(command, item);
        g_autoptr(GVariant) argument = g_variant_get_variant(wrapped);
        if (!g_variant_is_of_type(argument, G_VARIANT_TYPE_STRING)) {
            g_strfreev(argv);
            return NULL;
        }
        argv[item] = g_variant_dup_string(argument, NULL);
    }
    if (!*argv[0]) {
        g_strfreev(argv);
        return NULL;
    }
    return argv;
}

static void native_command_finished(GObject* source, GAsyncResult* result, gpointer user_data) {
    RunningCommand* run = user_data;
    g_autoptr(GError) error = NULL;
    if (!g_subprocess_wait_finish(G_SUBPROCESS(source), result, &error))
        g_warning("gnoblin-%s: %s: %s", run->category, run->name, error->message);
    else if (!g_subprocess_get_successful(run->process))
        g_warning("gnoblin-%s: %s exited unsuccessfully", run->category, run->name);
    g_object_unref(run->process);
    g_free(run->name);
    g_free(run);
}

static void launch_native_command(const char* category, const char* name, char** argv) {
    g_autoptr(GError) error = NULL;
    GSubprocess* process =
        g_subprocess_newv((const char* const*)argv, G_SUBPROCESS_FLAGS_NONE, &error);
    if (!process) {
        g_warning("gnoblin-%s: could not start %s: %s", category, name, error->message);
        return;
    }
    RunningCommand* run = g_new0(RunningCommand, 1);
    run->name = g_strdup(name);
    run->category = category;
    run->process = process;
    g_subprocess_wait_async(process, NULL, native_command_finished, run);
    g_message("gnoblin-%s: started %s", category, name);
}

static gboolean start_native_autostart(GVariant* document, GError** error) {
    g_autoptr(GVariant) declarations =
        document ? g_variant_lookup_value(document, "autostart", NULL) : NULL;
    if (!declarations)
        return TRUE;
    if (!g_variant_is_of_type(declarations, G_VARIANT_TYPE("av"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "autostart must be an array of entries");
        return FALSE;
    }

    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) entries = g_ptr_array_new_with_free_func(native_autostart_free);
    for (gsize index = 0; index < g_variant_n_children(declarations); index++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(declarations, index);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        g_autoptr(GVariant) when_value = NULL;
        g_autoptr(GVariant) command = NULL;
        const char* name = NULL;
        const char* when = "on_login";
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT))
            goto invalid_entry;
        GVariantIter fields;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&fields, entry);
        while (g_variant_iter_next(&fields, "{&sv}", &key, &value)) {
            gboolean supported =
                g_str_equal(key, "name") || g_str_equal(key, "command") || g_str_equal(key, "when");
            g_variant_unref(value);
            if (!supported)
                goto invalid_entry;
        }
        if (!g_variant_lookup(entry, "name", "&s", &name) || !*name ||
            g_utf8_strlen(name, -1) > 80 || g_hash_table_contains(names, name))
            goto invalid_entry;
        when_value = g_variant_lookup_value(entry, "when", NULL);
        if (when_value && (!g_variant_is_of_type(when_value, G_VARIANT_TYPE_STRING) ||
                           !g_str_equal(g_variant_get_string(when_value, NULL), when)))
            goto invalid_entry;
        command = g_variant_lookup_value(entry, "command", NULL);
        NativeAutostart* parsed = g_new0(NativeAutostart, 1);
        parsed->name = g_strdup(name);
        parsed->argv = native_command_argv(command);
        if (!parsed->argv) {
            native_autostart_free(parsed);
            goto invalid_entry;
        }
        g_hash_table_add(names, g_strdup(name));
        g_ptr_array_add(entries, parsed);
        continue;

    invalid_entry:
        g_set_error(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "autostart entry %zu needs a unique name, command array, and optional when='on_login'",
            index + 1);
        return FALSE;
    }

    for (guint index = 0; index < entries->len; index++) {
        NativeAutostart* entry = g_ptr_array_index(entries, index);
        launch_native_command("autostart", entry->name, entry->argv);
    }
    return TRUE;
}

static void native_shortcut_activated(MetaDisplay* display, guint action, gpointer device,
                                      guint timestamp, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (!shortcut->overlay && shortcut->action == action && !shortcut->release)
            launch_native_command("shortcut", shortcut->name, shortcut->argv);
    }
}

static void native_shortcut_deactivated(MetaDisplay* display, guint action, gpointer device,
                                        guint timestamp, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (!shortcut->overlay && shortcut->action == action && shortcut->release)
            launch_native_command("shortcut", shortcut->name, shortcut->argv);
    }
}

static void native_overlay_key(MetaDisplay* display, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    for (guint i = 0; i < control->shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
        if (shortcut->overlay)
            launch_native_command("shortcut", shortcut->name, shortcut->argv);
    }
}

static gboolean native_action_target(GVariant* action, const char** group, char** native_name) {
    const char* name = NULL;
    const char* schema = NULL;
    if (g_variant_is_of_type(action, G_VARIANT_TYPE_STRING)) {
        const char* label = g_variant_get_string(action, NULL);
        const char* separator = strchr(label, '.');
        if (!separator)
            return FALSE;
        g_autofree char* namespace = g_strndup(label, separator - label);
        if (g_str_equal(namespace, "gnome:shell"))
            *group = "shell";
        else if (g_str_equal(namespace, "wm") || g_str_equal(namespace, "mutter") ||
                 g_str_equal(namespace, "wayland"))
            *group = g_intern_string(namespace);
        else
            return FALSE;
        name = separator + 1;
        if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", name, 0, 0))
            return FALSE;
        *native_name = g_strdup(name);
        g_strdelimit(*native_name, "_", '-');
        return TRUE;
    }
    if (!g_variant_is_of_type(action, G_VARIANT_TYPE_VARDICT) ||
        g_variant_n_children(action) != 2 || !g_variant_lookup(action, "schema", "&s", &schema) ||
        !g_variant_lookup(action, "key", "&s", &name) ||
        !g_regex_match_simple("^[a-z0-9]+(?:-[a-z0-9]+)*$", name, 0, 0))
        return FALSE;
    if (g_str_equal(schema, "org.gnome.shell.keybindings"))
        *group = "shell";
    else if (g_str_equal(schema, "org.gnome.desktop.wm.keybindings"))
        *group = "wm";
    else if (g_str_equal(schema, "org.gnome.mutter.keybindings"))
        *group = "mutter";
    else if (g_str_equal(schema, "org.gnome.mutter.wayland.keybindings"))
        *group = "wayland";
    else
        return FALSE;
    *native_name = g_strdup(name);
    return TRUE;
}

static GVariant* native_binding_array(GVariant* bindings) {
    if (!bindings || !g_variant_is_of_type(bindings, G_VARIANT_TYPE("av")))
        return NULL;
    g_auto(GStrv) strings = g_new0(char*, g_variant_n_children(bindings) + 1);
    for (gsize i = 0; i < g_variant_n_children(bindings); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(bindings, i);
        g_autoptr(GVariant) binding = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(binding, G_VARIANT_TYPE_STRING) ||
            !*g_variant_get_string(binding, NULL) ||
            strlen(g_variant_get_string(binding, NULL)) > 160)
            return NULL;
        strings[i] = g_variant_dup_string(binding, NULL);
    }
    return g_variant_ref_sink(g_variant_new_strv((const char* const*)strings, -1));
}

static gboolean merge_native_action(GVariantDict* groups, GVariant* entry, GError** error) {
    g_autoptr(GVariant) action = g_variant_lookup_value(entry, "action", NULL);
    g_autoptr(GVariant) command = g_variant_lookup_value(entry, "command", NULL);
    g_autoptr(GVariant) trigger = g_variant_lookup_value(entry, "trigger", NULL);
    g_autoptr(GVariant) binding = g_variant_lookup_value(entry, "binding", NULL);
    g_autoptr(GVariant) capture = g_variant_lookup_value(entry, "capture-input", NULL);
    g_autoptr(GVariant) native_bindings = NULL;
    g_autoptr(GVariant) existing_group = NULL;
    g_autoptr(GVariant) existing_action = NULL;
    const char* group = NULL;
    g_autofree char* native_name = NULL;
    if (!native_action_target(action, &group, &native_name)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "named shortcut action must identify a supported keybinding");
        return FALSE;
    }
    if (g_str_equal(group, "shell")) {
        g_warning("gnoblin-shortcuts: skipping Shell action %s; it needs the Gnoblin Shell session",
                  native_name);
        return TRUE;
    }
    if (command || trigger ||
        (capture && (!g_variant_is_of_type(capture, G_VARIANT_TYPE_BOOLEAN) ||
                     g_variant_get_boolean(capture)))) {
        g_set_error_literal(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "named built-in actions cannot have command, trigger, or input capture");
        return FALSE;
    }
    native_bindings = native_binding_array(binding);
    if (!native_bindings) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "named built-in action needs an array of accelerators");
        return FALSE;
    }
    const char* schema_name = g_str_equal(group, "wm") ? "org.gnome.desktop.wm.keybindings"
                              : g_str_equal(group, "mutter")
                                  ? "org.gnome.mutter.keybindings"
                                  : "org.gnome.mutter.wayland.keybindings";
    GSettingsSchemaSource* source = g_settings_schema_source_get_default();
    g_autoptr(GSettingsSchema) schema =
        source ? g_settings_schema_source_lookup(source, schema_name, TRUE) : NULL;
    if (!schema || !g_settings_schema_has_key(schema, native_name)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "unknown native shortcut action: %s.%s", group, native_name);
        return FALSE;
    }
    g_autoptr(GSettingsSchemaKey) schema_key = g_settings_schema_get_key(schema, native_name);
    if (!g_variant_type_equal(g_settings_schema_key_get_value_type(schema_key),
                              G_VARIANT_TYPE_STRING_ARRAY)) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "native shortcut action does not accept bindings: %s.%s", group, native_name);
        return FALSE;
    }
    existing_group = g_variant_dict_lookup_value(groups, group, G_VARIANT_TYPE_VARDICT);
    existing_action =
        existing_group ? g_variant_lookup_value(existing_group, native_name, NULL) : NULL;
    if (existing_action) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                    "native action configured more than once: %s.%s", group, native_name);
        return FALSE;
    }
    GVariantDict actions;
    g_variant_dict_init(&actions, existing_group);
    g_variant_dict_insert_value(&actions, native_name, native_bindings);
    g_autoptr(GVariant) merged_actions = g_variant_ref_sink(g_variant_dict_end(&actions));
    g_variant_dict_insert_value(groups, group, merged_actions);
    return TRUE;
}

static GVariant* merge_native_actions(GVariant* base, GVariant* document, GError** error) {
    g_autoptr(GVariant) shortcuts =
        document ? g_variant_lookup_value(document, "shortcuts", NULL) : NULL;
    if (!shortcuts)
        return g_variant_ref(base);
    if (!g_variant_is_of_type(shortcuts, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(shortcuts) > 256) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcuts must be an array of at most 256 entries");
        return NULL;
    }
    GVariantDict groups;
    g_variant_dict_init(&groups, base);
    for (gsize i = 0; i < g_variant_n_children(shortcuts); i++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(shortcuts, i);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut entry %zu must be a table", i + 1);
            g_variant_dict_clear(&groups);
            return NULL;
        }
        g_autoptr(GVariant) action = g_variant_lookup_value(entry, "action", NULL);
        if (action && !merge_native_action(&groups, entry, error)) {
            g_variant_dict_clear(&groups);
            return NULL;
        }
    }
    return g_variant_ref_sink(g_variant_dict_end(&groups));
}

static gboolean apply_native_keybindings(GVariant* document, GError** error) {
    static const struct {
        const char* group;
        const char* schema;
    } schemas[] = {
        {"wm", "org.gnome.desktop.wm.keybindings"},
        {"mutter", "org.gnome.mutter.keybindings"},
        {"wayland", "org.gnome.mutter.wayland.keybindings"},
    };
    g_autoptr(GVariant) configured =
        document ? g_variant_lookup_value(document, "keybindings", NULL) : NULL;
    if (configured && !g_variant_is_of_type(configured, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "keybindings must be a table");
        return FALSE;
    }
    GVariantBuilder groups;
    g_autoptr(GVariant) normalized = NULL;
    g_autoptr(GVariant) merged = NULL;
    g_variant_builder_init(&groups, G_VARIANT_TYPE_VARDICT);
    if (configured) {
        GVariantIter iter;
        const char* group;
        GVariant* entries;
        GSettingsSchemaSource* source = g_settings_schema_source_get_default();
        g_variant_iter_init(&iter, configured);
        while (g_variant_iter_next(&iter, "{&sv}", &group, &entries)) {
            g_autoptr(GVariant) group_entries = entries;
            const char* schema_name = NULL;
            for (guint i = 0; i < G_N_ELEMENTS(schemas); i++)
                if (g_str_equal(group, schemas[i].group))
                    schema_name = schemas[i].schema;
            if (!schema_name) {
                if (g_str_equal(group, "shell")) {
                    g_warning("gnoblin-keybindings: skipping keybindings.shell; it needs the "
                              "Gnoblin Shell session");
                    continue;
                }
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "keybindings.%s is not a native keybinding group", group);
                goto invalid_keybindings;
            }
            if (!g_variant_is_of_type(group_entries, G_VARIANT_TYPE_VARDICT)) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "keybindings.%s must be a table", group);
                goto invalid_keybindings;
            }
            g_autoptr(GSettingsSchema) schema =
                source ? g_settings_schema_source_lookup(source, schema_name, TRUE) : NULL;
            if (!schema) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "keybinding schema %s is not installed", schema_name);
                goto invalid_keybindings;
            }
            GVariantBuilder actions;
            GVariantIter action_iter;
            const char* name;
            GVariant* value;
            g_variant_builder_init(&actions, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&action_iter, group_entries);
            while (g_variant_iter_next(&action_iter, "{&sv}", &name, &value)) {
                g_autoptr(GVariant) bindings = value;
                if (!g_regex_match_simple("^[a-z0-9]+(?:_[a-z0-9]+)*$", name, 0, 0)) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "invalid keybinding name: %s.%s", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_autofree char* native_name = g_strdup(name);
                g_strdelimit(native_name, "_", '-');
                if (!g_settings_schema_has_key(schema, native_name)) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "unknown keybinding: %s.%s", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_autoptr(GSettingsSchemaKey) schema_key =
                    g_settings_schema_get_key(schema, native_name);
                g_autoptr(GVariant) native_bindings = native_binding_array(bindings);
                if (!g_variant_type_equal(g_settings_schema_key_get_value_type(schema_key),
                                          G_VARIANT_TYPE_STRING_ARRAY) ||
                    !native_bindings) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "keybindings.%s.%s needs an array of accelerators", group, name);
                    g_variant_builder_clear(&actions);
                    goto invalid_keybindings;
                }
                g_variant_builder_add(&actions, "{sv}", native_name,
                                      g_variant_ref(native_bindings));
            }
            g_autoptr(GVariant) native_actions =
                g_variant_ref_sink(g_variant_builder_end(&actions));
            g_variant_builder_add(&groups, "{sv}", group, g_variant_ref(native_actions));
        }
    }
    normalized = g_variant_ref_sink(g_variant_builder_end(&groups));
    merged = merge_native_actions(normalized, document, error);
    if (!merged)
        return FALSE;
    meta_prefs_apply_gnoblin_keybindings(merged);
    return TRUE;

invalid_keybindings:
    g_variant_builder_clear(&groups);
    return FALSE;
}

static gboolean start_native_shortcuts(GnoblinNativeControl* control, GVariant* document,
                                       GError** error) {
    g_autoptr(GVariant) declarations =
        document ? g_variant_lookup_value(document, "shortcuts", NULL) : NULL;
    if (!declarations)
        return TRUE;
    if (!g_variant_is_of_type(declarations, G_VARIANT_TYPE("av")) ||
        g_variant_n_children(declarations) > 256) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "shortcuts must be an array of at most 256 entries");
        return FALSE;
    }
    g_autoptr(GHashTable) names = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) shortcuts = g_ptr_array_new_with_free_func(native_shortcut_free);
    gboolean has_overlay = FALSE;
    for (gsize index = 0; index < g_variant_n_children(declarations); index++) {
        g_autoptr(GVariant) boxed = g_variant_get_child_value(declarations, index);
        g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
        g_autoptr(GVariant) command = NULL;
        g_autoptr(GVariant) binding_value = NULL;
        g_autoptr(GVariant) trigger_value = NULL;
        g_autoptr(GVariant) capture_value = NULL;
        g_autoptr(GVariant) action_value = NULL;
        const char* name = NULL;
        const char* binding = NULL;
        const char* trigger = "press";
        if (!g_variant_is_of_type(entry, G_VARIANT_TYPE_VARDICT))
            goto invalid_shortcut;
        GVariantIter fields;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&fields, entry);
        while (g_variant_iter_next(&fields, "{&sv}", &key, &value)) {
            gboolean supported = g_str_equal(key, "name") || g_str_equal(key, "binding") ||
                                 g_str_equal(key, "command") || g_str_equal(key, "action") ||
                                 g_str_equal(key, "trigger") || g_str_equal(key, "capture-input");
            g_variant_unref(value);
            if (!supported)
                goto invalid_shortcut;
        }
        if (!g_variant_lookup(entry, "name", "&s", &name) || !*name || strlen(name) > 80 ||
            g_hash_table_contains(names, name))
            goto invalid_shortcut;
        for (const char* character = name; *character; character++)
            if (!g_ascii_isalnum(*character) && *character != '_' && *character != '-')
                goto invalid_shortcut;
        action_value = g_variant_lookup_value(entry, "action", NULL);
        if (action_value) {
            g_hash_table_add(names, g_strdup(name));
            continue;
        }
        binding_value = g_variant_lookup_value(entry, "binding", NULL);
        if (!binding_value || !g_variant_is_of_type(binding_value, G_VARIANT_TYPE_STRING))
            goto invalid_shortcut;
        binding = g_variant_get_string(binding_value, NULL);
        if (!*binding || strlen(binding) > 160)
            goto invalid_shortcut;
        trigger_value = g_variant_lookup_value(entry, "trigger", NULL);
        if (trigger_value) {
            if (!g_variant_is_of_type(trigger_value, G_VARIANT_TYPE_STRING))
                goto invalid_shortcut;
            trigger = g_variant_get_string(trigger_value, NULL);
            if (!g_str_equal(trigger, "press") && !g_str_equal(trigger, "release"))
                goto invalid_shortcut;
        }
        capture_value = g_variant_lookup_value(entry, "capture-input", NULL);
        if (capture_value) {
            if (!g_variant_is_of_type(capture_value, G_VARIANT_TYPE_BOOLEAN))
                goto invalid_shortcut;
            if (g_variant_get_boolean(capture_value)) {
                g_warning("gnoblin-shortcuts: skipping '%s'; input capture needs the Gnoblin Shell "
                          "session",
                          name);
                g_hash_table_add(names, g_strdup(name));
                continue;
            }
        }
        command = g_variant_lookup_value(entry, "command", NULL);
        NativeShortcut* shortcut = g_new0(NativeShortcut, 1);
        shortcut->name = g_strdup(name);
        shortcut->binding = g_strdup(binding);
        shortcut->argv = native_command_argv(command);
        if (!shortcut->argv) {
            native_shortcut_free(shortcut);
            goto invalid_shortcut;
        }
        shortcut->overlay = g_str_equal(binding, "Super");
        shortcut->release = g_str_equal(trigger, "release");
        if (shortcut->overlay && (!shortcut->release || has_overlay)) {
            native_shortcut_free(shortcut);
            goto invalid_shortcut;
        }
        has_overlay |= shortcut->overlay;
        g_hash_table_add(names, g_strdup(name));
        g_ptr_array_add(shortcuts, shortcut);
        continue;

    invalid_shortcut:
        g_set_error(
            error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
            "shortcut entry %zu needs a unique name, binding, command array, and valid trigger",
            index + 1);
        return FALSE;
    }
    for (guint i = 0; i < shortcuts->len; i++) {
        NativeShortcut* shortcut = g_ptr_array_index(shortcuts, i);
        if (shortcut->overlay)
            continue;
        MetaKeyBindingFlags flags = META_KEY_BINDING_IGNORE_AUTOREPEAT;
        if (shortcut->release)
            flags |= META_KEY_BINDING_TRIGGER_RELEASE;
        shortcut->action =
            meta_display_grab_accelerator(control->display, shortcut->binding, flags);
        if (shortcut->action == META_KEYBINDING_ACTION_NONE) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "shortcut '%s' could not claim %s", shortcut->name, shortcut->binding);
            for (guint previous = 0; previous < i; previous++) {
                NativeShortcut* claimed = g_ptr_array_index(shortcuts, previous);
                if (!claimed->overlay)
                    meta_display_ungrab_accelerator(control->display, claimed->action);
            }
            return FALSE;
        }
    }
    control->shortcuts = g_steal_pointer(&shortcuts);
    g_signal_connect(control->display, "accelerator-activated",
                     G_CALLBACK(native_shortcut_activated), control);
    g_signal_connect(control->display, "accelerator-deactivated",
                     G_CALLBACK(native_shortcut_deactivated), control);
    if (has_overlay)
        g_signal_connect(control->display, "overlay-key", G_CALLBACK(native_overlay_key), control);
    return TRUE;
}

typedef enum {
    INPUT_BOOLEAN,
    INPUT_DOUBLE,
    INPUT_MILLISECONDS,
    INPUT_STRING,
    INPUT_STRINGS,
    INPUT_CHOICE,
    INPUT_ACCEL_CURVE,
} InputKind;

typedef struct {
    const char* group;
    const char* name;
    InputKind kind;
    const char* choices;
    double minimum;
    double maximum;
} InputField;

static const InputField input_fields[] = {
    {"mouse", "speed", INPUT_DOUBLE, NULL, -1, 1},
    {"mouse", "left-handed", INPUT_BOOLEAN},
    {"mouse", "natural-scroll", INPUT_BOOLEAN},
    {"mouse", "accel-profile", INPUT_CHOICE, "default flat adaptive custom"},
    {"mouse", "accel-curve", INPUT_ACCEL_CURVE},
    {"touchpad", "speed", INPUT_DOUBLE, NULL, -1, 1},
    {"touchpad", "scroll-speed", INPUT_DOUBLE, NULL, 0, 2},
    {"touchpad", "left-handed", INPUT_CHOICE, "right left mouse"},
    {"touchpad", "natural-scroll", INPUT_BOOLEAN},
    {"touchpad", "accel-profile", INPUT_CHOICE, "default flat adaptive custom"},
    {"touchpad", "accel-curve", INPUT_ACCEL_CURVE},
    {"touchpad", "tap-to-click", INPUT_BOOLEAN},
    {"touchpad", "tap-button-map", INPUT_CHOICE, "default lrm lmr"},
    {"touchpad", "tap-and-drag", INPUT_BOOLEAN},
    {"touchpad", "tap-and-drag-lock", INPUT_BOOLEAN},
    {"touchpad", "disable-while-typing", INPUT_BOOLEAN},
    {"touchpad", "edge-scrolling-enabled", INPUT_BOOLEAN},
    {"touchpad", "two-finger-scrolling-enabled", INPUT_BOOLEAN},
    {"touchpad", "click-method", INPUT_CHOICE, "default none areas fingers"},
    {"keyboard", "repeat", INPUT_BOOLEAN},
    {"keyboard", "delay", INPUT_MILLISECONDS},
    {"keyboard", "repeat-interval", INPUT_MILLISECONDS},
    {"keyboard", "remember-numlock-state", INPUT_BOOLEAN},
    {"keyboard", "numlock-state", INPUT_BOOLEAN},
    {"keyboard", "xkb-options", INPUT_STRINGS},
    {"tablets", "mapping", INPUT_CHOICE, "absolute relative"},
    {"tablets", "left-handed", INPUT_BOOLEAN},
    {"tablets", "keep-aspect", INPUT_BOOLEAN},
    {"styluses", "button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "secondary-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "tertiary-button-action", INPUT_CHOICE,
     "default middle right back forward switch-monitor keybinding"},
    {"styluses", "button-keybinding", INPUT_STRING},
    {"styluses", "secondary-button-keybinding", INPUT_STRING},
    {"styluses", "tertiary-button-keybinding", INPUT_STRING},
};

static const InputField* find_input_field(const char* group, const char* name) {
    for (guint i = 0; i < G_N_ELEMENTS(input_fields); i++)
        if (g_str_equal(input_fields[i].group, group) && g_str_equal(input_fields[i].name, name))
            return &input_fields[i];
    return NULL;
}

static GVariant* normalize_input_value(const InputField* field, GVariant* value) {
    if (field->kind == INPUT_ACCEL_CURVE && g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        GVariantBuilder curve;
        GVariantIter iter;
        const char* name;
        GVariant* member;
        g_autoptr(GVariant) step_value = NULL;
        g_autoptr(GVariant) points_value = NULL;
        double step;
        GArray* points = g_array_new(FALSE, FALSE, sizeof(double));
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &name, &member)) {
            g_autoptr(GVariant) current = member;
            if (g_str_equal(name, "step"))
                step_value = g_steal_pointer(&current);
            else if (g_str_equal(name, "points"))
                points_value = g_steal_pointer(&current);
            else {
                g_array_unref(points);
                return NULL;
            }
        }
        if (!step_value || !points_value) {
            g_array_unref(points);
            return NULL;
        }
        if (g_variant_is_of_type(step_value, G_VARIANT_TYPE_DOUBLE))
            step = g_variant_get_double(step_value);
        else if (g_variant_is_of_type(step_value, G_VARIANT_TYPE_INT64))
            step = (double)g_variant_get_int64(step_value);
        else {
            g_array_unref(points);
            return NULL;
        }
        if (!isfinite(step) || step <= 0) {
            g_array_unref(points);
            return NULL;
        }
        if (g_variant_is_of_type(points_value, G_VARIANT_TYPE("ad"))) {
            gsize n_points;
            const double* values =
                g_variant_get_fixed_array(points_value, &n_points, sizeof(double));
            for (gsize i = 0; i < n_points; i++) {
                if (!isfinite(values[i]) || values[i] < 0) {
                    g_array_unref(points);
                    return NULL;
                }
                g_array_append_val(points, values[i]);
            }
        } else if (g_variant_is_of_type(points_value, G_VARIANT_TYPE("av"))) {
            for (gsize i = 0; i < g_variant_n_children(points_value); i++) {
                g_autoptr(GVariant) boxed = g_variant_get_child_value(points_value, i);
                g_autoptr(GVariant) point = g_variant_get_variant(boxed);
                double number;
                if (g_variant_is_of_type(point, G_VARIANT_TYPE_DOUBLE))
                    number = g_variant_get_double(point);
                else if (g_variant_is_of_type(point, G_VARIANT_TYPE_INT64))
                    number = (double)g_variant_get_int64(point);
                else {
                    g_array_unref(points);
                    return NULL;
                }
                if (!isfinite(number) || number < 0) {
                    g_array_unref(points);
                    return NULL;
                }
                g_array_append_val(points, number);
            }
        } else {
            g_array_unref(points);
            return NULL;
        }
        if (points->len < 2) {
            g_array_unref(points);
            return NULL;
        }
        g_variant_builder_init(&curve, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&curve, "{sv}", "step", g_variant_new_double(step));
        g_variant_builder_add(&curve, "{sv}", "points",
                              g_variant_new_fixed_array(G_VARIANT_TYPE_DOUBLE, points->data,
                                                        points->len, sizeof(double)));
        g_array_unref(points);
        return g_variant_ref_sink(g_variant_builder_end(&curve));
    }
    if (field->kind == INPUT_BOOLEAN && g_variant_is_of_type(value, G_VARIANT_TYPE_BOOLEAN))
        return g_variant_ref(value);
    if (field->kind == INPUT_STRING && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING))
        return g_variant_ref(value);
    if (field->kind == INPUT_CHOICE && g_variant_is_of_type(value, G_VARIANT_TYPE_STRING)) {
        g_auto(GStrv) choices = g_strsplit(field->choices, " ", -1);
        if (g_strv_contains((const char* const*)choices, g_variant_get_string(value, NULL)))
            return g_variant_ref(value);
    }
    if (field->kind == INPUT_DOUBLE) {
        double number;
        if (g_variant_is_of_type(value, G_VARIANT_TYPE_DOUBLE))
            number = g_variant_get_double(value);
        else if (g_variant_is_of_type(value, G_VARIANT_TYPE_INT64))
            number = (double)g_variant_get_int64(value);
        else
            return NULL;
        if (isfinite(number) && number >= field->minimum && number <= field->maximum)
            return g_variant_ref_sink(g_variant_new_double(number));
    }
    if (field->kind == INPUT_MILLISECONDS && g_variant_is_of_type(value, G_VARIANT_TYPE_INT64)) {
        gint64 milliseconds = g_variant_get_int64(value);
        if (milliseconds >= 1 && milliseconds <= 10000)
            return g_variant_ref_sink(g_variant_new_uint32((guint32)milliseconds));
    }
    if (field->kind == INPUT_STRINGS && g_variant_is_of_type(value, G_VARIANT_TYPE("av"))) {
        g_auto(GStrv) strings = g_new0(char*, g_variant_n_children(value) + 1);
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(value, i);
            g_autoptr(GVariant) item = g_variant_get_variant(boxed);
            if (!g_variant_is_of_type(item, G_VARIANT_TYPE_STRING))
                return NULL;
            strings[i] = g_variant_dup_string(item, NULL);
        }
        return g_variant_ref_sink(g_variant_new_strv((const char* const*)strings, -1));
    }
    return NULL;
}

static GVariant* normalize_input_fields(const char* group, GVariant* fields, GError** error) {
    if (!g_variant_is_of_type(fields, G_VARIANT_TYPE_VARDICT))
        goto invalid_group;
    GVariantBuilder converted;
    GVariantIter iter;
    const char* name;
    GVariant* value;
    g_variant_builder_init(&converted, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&iter, fields);
    while (g_variant_iter_next(&iter, "{&sv}", &name, &value)) {
        g_autoptr(GVariant) current = value;
        const InputField* field = find_input_field(group, name);
        if (!field) {
            g_variant_builder_clear(&converted);
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "unknown input setting: input.%s.%s", group, name);
            return NULL;
        }
        if (g_variant_is_of_type(current, G_VARIANT_TYPE_STRING) &&
            g_str_equal(g_variant_get_string(current, NULL), "inherit"))
            continue;
        g_autoptr(GVariant) normalized = normalize_input_value(field, current);
        if (!normalized) {
            g_variant_builder_clear(&converted);
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "input.%s.%s: invalid value", group, name);
            return NULL;
        }
        g_variant_builder_add(&converted, "{sv}", name, g_variant_ref(normalized));
    }
    return g_variant_ref_sink(g_variant_builder_end(&converted));

invalid_group:
    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "input.%s must be a table", group);
    return NULL;
}

static gboolean valid_input_device(const char* group, const char* device) {
    return g_regex_match_simple(g_str_equal(group, "tablets")
                                    ? "^[0-9a-fA-F]{4}:[0-9a-fA-F]{4}$"
                                    : "^(?:[0-9a-fA-F]+|default-[0-9a-fA-F]{4}:[0-9a-fA-F]{4})$",
                                device, G_REGEX_OPTIMIZE, 0);
}

static gboolean apply_native_input(GnoblinNativeControl* control, MetaContext* context,
                                   GVariant* document, GError** error) {
    g_autoptr(GVariant) input = document ? g_variant_lookup_value(document, "input", NULL) : NULL;
    if (!input)
        return TRUE;
    if (!g_variant_is_of_type(input, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "input must be a table");
        return FALSE;
    }
    GVariantBuilder converted;
    GVariantIter groups;
    const char* group;
    GVariant* value;
    gboolean orientation_set = FALSE;
    gboolean orientation_locked = FALSE;
    g_autoptr(GVariant) normalized_input = NULL;
    g_autoptr(GVariant) keyboard = NULL;
    g_variant_builder_init(&converted, G_VARIANT_TYPE_VARDICT);
    g_variant_iter_init(&groups, input);
    while (g_variant_iter_next(&groups, "{&sv}", &group, &value)) {
        g_autoptr(GVariant) settings = value;
        if (g_str_equal(group, "orientation-lock")) {
            if (g_variant_is_of_type(settings, G_VARIANT_TYPE_BOOLEAN)) {
                orientation_set = TRUE;
                orientation_locked = g_variant_get_boolean(settings);
            } else if (!g_variant_is_of_type(settings, G_VARIANT_TYPE_STRING) ||
                       !g_str_equal(g_variant_get_string(settings, NULL), "inherit"))
                goto invalid_group;
            continue;
        }
        if (g_str_equal(group, "mouse") || g_str_equal(group, "touchpad") ||
            g_str_equal(group, "keyboard")) {
            g_autoptr(GVariant) normalized = normalize_input_fields(group, settings, error);
            if (!normalized)
                goto invalid_input;
            g_variant_builder_add(&converted, "{sv}", group, g_variant_ref(normalized));
            continue;
        }
        if (g_str_equal(group, "tablets") || g_str_equal(group, "styluses")) {
            if (!g_variant_is_of_type(settings, G_VARIANT_TYPE_VARDICT))
                goto invalid_group;
            GVariantBuilder devices;
            GVariantIter iter;
            const char* device;
            GVariant* fields;
            g_variant_builder_init(&devices, G_VARIANT_TYPE_VARDICT);
            g_variant_iter_init(&iter, settings);
            while (g_variant_iter_next(&iter, "{&sv}", &device, &fields)) {
                g_autoptr(GVariant) device_fields = fields;
                if (!valid_input_device(group, device)) {
                    g_variant_builder_clear(&devices);
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "input.%s: invalid device identifier %s", group, device);
                    goto invalid_input;
                }
                g_autoptr(GVariant) normalized =
                    normalize_input_fields(group, device_fields, error);
                if (!normalized) {
                    g_variant_builder_clear(&devices);
                    goto invalid_input;
                }
                g_variant_builder_add(&devices, "{sv}", device, g_variant_ref(normalized));
            }
            g_autoptr(GVariant) normalized = g_variant_ref_sink(g_variant_builder_end(&devices));
            g_variant_builder_add(&converted, "{sv}", group, g_variant_ref(normalized));
            continue;
        }
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "unknown input group: %s",
                    group);
        goto invalid_input;

    invalid_group:
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT, "input.%s: invalid value",
                    group);
        goto invalid_input;
    }
    normalized_input = g_variant_ref_sink(g_variant_builder_end(&converted));
    meta_display_apply_gnoblin_input_config(control->display, normalized_input);
    keyboard = g_variant_lookup_value(normalized_input, "keyboard", G_VARIANT_TYPE_VARDICT);
    meta_prefs_apply_gnoblin_keyboard_preferences(keyboard);
    MetaOrientationManager* orientation =
        meta_backend_get_orientation_manager(meta_context_get_backend(context));
    if (orientation_set)
        meta_orientation_manager_set_orientation_locked(orientation, orientation_locked);
    else
        meta_orientation_manager_clear_orientation_lock_override(orientation);
    return TRUE;

invalid_input:
    g_variant_builder_clear(&converted);
    return FALSE;
}

static void client_free(Client* client) {
    if (client->control)
        g_hash_table_remove(client->control->clients, client);
    g_io_stream_close(G_IO_STREAM(client->connection), NULL, NULL);
    g_clear_object(&client->connection);
    g_string_free(client->request, TRUE);
    g_queue_free_full(client->outgoing, g_free);
    g_free(client);
}

static void client_maybe_free(Client* client) {
    if (client->closing && !client->reading && !client->writing)
        client_free(client);
}

static void client_close(Client* client) {
    if (!client->closing) {
        client->closing = TRUE;
        if (client->control)
            g_hash_table_remove(client->control->clients, client);
        client->control = NULL;
        g_io_stream_close(G_IO_STREAM(client->connection), NULL, NULL);
    }
    client_maybe_free(client);
}

static JsonNode* json_from_variant(GVariant* value) {
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARIANT)) {
        g_autoptr(GVariant) child = g_variant_get_variant(value);
        return json_from_variant(child);
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_VARDICT)) {
        JsonObject* object = json_object_new();
        GVariantIter iter;
        const char* key;
        GVariant* child;
        g_variant_iter_init(&iter, value);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &child)) {
            json_object_set_member(object, key, json_from_variant(child));
            g_variant_unref(child);
        }
        JsonNode* node = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(node, object);
        return node;
    }
    if (g_variant_is_of_type(value, G_VARIANT_TYPE_ARRAY)) {
        JsonArray* array = json_array_new();
        for (gsize i = 0; i < g_variant_n_children(value); i++) {
            g_autoptr(GVariant) child = g_variant_get_child_value(value, i);
            json_array_add_element(array, json_from_variant(child));
        }
        JsonNode* node = json_node_new(JSON_NODE_ARRAY);
        json_node_take_array(node, array);
        return node;
    }
    JsonNode* node = json_node_new(JSON_NODE_VALUE);
    switch (g_variant_classify(value)) {
    case G_VARIANT_CLASS_BOOLEAN:
        json_node_set_boolean(node, g_variant_get_boolean(value));
        break;
    case G_VARIANT_CLASS_DOUBLE:
        json_node_set_double(node, g_variant_get_double(value));
        break;
    case G_VARIANT_CLASS_INT32:
        json_node_set_int(node, g_variant_get_int32(value));
        break;
    case G_VARIANT_CLASS_INT64:
        json_node_set_int(node, g_variant_get_int64(value));
        break;
    case G_VARIANT_CLASS_UINT32:
        json_node_set_int(node, g_variant_get_uint32(value));
        break;
    case G_VARIANT_CLASS_STRING:
        json_node_set_string(node, g_variant_get_string(value, NULL));
        break;
    default:
        json_node_free(node);
        return json_node_new(JSON_NODE_NULL);
    }
    return node;
}

static GVariant* variant_from_json(JsonNode* node) {
    if (JSON_NODE_HOLDS_OBJECT(node)) {
        GVariantBuilder builder;
        JsonObject* object = json_node_get_object(node);
        GList* members = json_object_get_members(object);
        g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
        for (GList* item = members; item; item = item->next) {
            const char* key = item->data;
            GVariant* value = variant_from_json(json_object_get_member(object, key));
            if (value)
                g_variant_builder_add(&builder, "{sv}", key, value);
        }
        g_list_free(members);
        return g_variant_ref_sink(g_variant_builder_end(&builder));
    }
    if (JSON_NODE_HOLDS_ARRAY(node)) {
        GVariantBuilder builder;
        JsonArray* array = json_node_get_array(node);
        g_variant_builder_init(&builder, G_VARIANT_TYPE("av"));
        for (guint i = 0; i < json_array_get_length(array); i++) {
            GVariant* value = variant_from_json(json_array_get_element(array, i));
            if (value)
                g_variant_builder_add_value(&builder, g_variant_new_variant(value));
        }
        return g_variant_ref_sink(g_variant_builder_end(&builder));
    }
    if (!JSON_NODE_HOLDS_VALUE(node))
        return NULL;
    GType type = json_node_get_value_type(node);
    if (type == G_TYPE_BOOLEAN)
        return g_variant_new_boolean(json_node_get_boolean(node));
    if (type == G_TYPE_DOUBLE || type == G_TYPE_FLOAT)
        return g_variant_new_double(json_node_get_double(node));
    if (type == G_TYPE_INT64 || type == G_TYPE_INT || type == G_TYPE_LONG)
        return g_variant_new_int64(json_node_get_int(node));
    if (type == G_TYPE_STRING)
        return g_variant_new_string(json_node_get_string(node));
    return NULL;
}

static char* encode_response(const char* id, JsonNode* result, const char* message) {
    g_autoptr(JsonNode) root = json_node_new(JSON_NODE_OBJECT);
    JsonObject* object = json_object_new();
    json_node_take_object(root, object);
    json_object_set_string_member(object, "event", message ? "error" : "reply");
    json_object_set_string_member(object, "id", id);
    if (message)
        json_object_set_string_member(object, "message", message);
    else
        json_object_set_member(object, "result", json_node_copy(result));
    g_autofree char* encoded = json_to_string(root, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

static char* window_snapshot(GnoblinNativeControl* control) {
    g_autoptr(GError) error = NULL;
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) result =
        meta_gnoblin_dispatch_native_api(control->display, "window.list", arguments, &error);
    if (!result)
        return encode_response("", NULL, error ? error->message : "window listing unavailable");
    g_autoptr(JsonNode) json = json_from_variant(result);
    JsonObject* object = json_node_get_object(json);
    json_object_set_string_member(object, "event", "windows");
    g_autofree char* encoded = json_to_string(json, FALSE);
    return g_strconcat(encoded, "\n", NULL);
}

static char* handle_request(Client* client, const char* data, gsize length) {
    g_autoptr(JsonParser) parser = json_parser_new();
    g_autoptr(GError) error = NULL;
    const char* id = "";
    if (!json_parser_load_from_data(parser, data, length, &error) ||
        !JSON_NODE_HOLDS_OBJECT(json_parser_get_root(parser))) {
        client->close_after_response = TRUE;
        return encode_response(id, NULL, "invalid JSON request");
    }

    JsonObject* request = json_node_get_object(json_parser_get_root(parser));
    const char* op = json_object_get_string_member_with_default(request, "op", "");
    if (g_str_equal(op, "windows")) {
        client->track_windows = TRUE;
        return window_snapshot(client->control);
    }
    if (json_object_has_member(request, "id") &&
        JSON_NODE_HOLDS_VALUE(json_object_get_member(request, "id")) &&
        json_node_get_value_type(json_object_get_member(request, "id")) == G_TYPE_STRING)
        id = json_object_get_string_member(request, "id");
    if (!*id || strlen(id) > 64)
        return encode_response("", NULL, "invalid request ID");
    const char* method = json_object_get_string_member_with_default(request, "method", "");
    if (!g_str_equal(op, "api"))
        return encode_response(id, NULL, "native compositor supports only API requests");
    if (g_str_equal(method, "shell.ping")) {
        JsonObject* pong = json_object_new();
        json_object_set_string_member(pong, "pong", "pong");
        g_autoptr(JsonNode) result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, pong);
        return encode_response(id, result, NULL);
    }
    JsonNode* arguments_node = json_object_get_member(request, "arguments");
    if (arguments_node && !JSON_NODE_HOLDS_OBJECT(arguments_node))
        return encode_response(id, NULL, "arguments must be an object");
    GVariantBuilder empty;
    g_variant_builder_init(&empty, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) arguments = arguments_node
                                        ? variant_from_json(arguments_node)
                                        : g_variant_ref_sink(g_variant_builder_end(&empty));
    g_autoptr(GVariant) operation = meta_gnoblin_call_config_api(method, arguments, &error);
    if (!operation)
        return encode_response(id, NULL, error ? error->message : "unsupported API method");
    const char* native_method = NULL;
    if (!g_variant_lookup(operation, "method", "&s", &native_method))
        return encode_response(id, NULL, "invalid Lua operation descriptor");
    g_autoptr(GVariant) native_arguments =
        g_variant_lookup_value(operation, "arguments", G_VARIANT_TYPE_VARDICT);
    if (!native_arguments)
        return encode_response(id, NULL, "invalid Lua operation arguments");
    g_autoptr(GVariant) result = meta_gnoblin_dispatch_native_api(
        client->control->display, native_method, native_arguments, &error);
    if (!result)
        return encode_response(id, NULL,
                               error ? error->message : "method still needs Gnoblin Shell");
    g_autoptr(JsonNode) json = json_from_variant(result);
    return encode_response(id, json, NULL);
}

static void process_buffer(Client* client);
static void write_next(Client* client);

static void write_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    PendingWrite* pending = user_data;
    g_autoptr(GError) error = NULL;
    gboolean written =
        g_output_stream_write_all_finish(G_OUTPUT_STREAM(source), result, NULL, &error);
    Client* client = pending->client;
    client->writing = FALSE;
    client->pending_bytes -= strlen(pending->response);
    g_free(pending->response);
    g_free(pending);
    if (!written || client->closing) {
        client_close(client);
        return;
    }
    if (client->close_after_response && g_queue_is_empty(client->outgoing)) {
        client_close(client);
        return;
    }
    write_next(client);
    if (!client->writing)
        process_buffer(client);
}

static void write_next(Client* client) {
    if (client->writing || client->closing || g_queue_is_empty(client->outgoing))
        return;
    PendingWrite* pending = g_new0(PendingWrite, 1);
    pending->client = client;
    pending->response = g_queue_pop_head(client->outgoing);
    client->writing = TRUE;
    g_output_stream_write_all_async(g_io_stream_get_output_stream(G_IO_STREAM(client->connection)),
                                    pending->response, strlen(pending->response),
                                    G_PRIORITY_DEFAULT, NULL, write_done, pending);
}

static void send_response(Client* client, char* response) {
    if (client->closing) {
        g_free(response);
        return;
    }
    gsize length = strlen(response);
    if (client->pending_bytes + length > MAX_PENDING_BYTES) {
        g_free(response);
        client_close(client);
        return;
    }
    client->pending_bytes += length;
    g_queue_push_tail(client->outgoing, response);
    write_next(client);
}

static gboolean publish_windows(gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    control->publish_id = 0;
    g_autofree char* snapshot = window_snapshot(control);
    GList* clients = g_hash_table_get_keys(control->clients);
    for (GList* item = clients; item; item = item->next) {
        Client* client = item->data;
        if (client->track_windows)
            send_response(client, g_strdup(snapshot));
    }
    g_list_free(clients);
    return G_SOURCE_REMOVE;
}

static void schedule_windows(GnoblinNativeControl* control) {
    if (!control->publish_id)
        control->publish_id = g_idle_add(publish_windows, control);
}

static void window_changed(MetaWindow* window, gpointer user_data) {
    schedule_windows(user_data);
}

static void window_notified(GObject* window, GParamSpec* property, gpointer user_data) {
    schedule_windows(user_data);
}

static void window_unmanaged(MetaWindow* window, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    g_signal_handlers_disconnect_by_data(window, control);
    g_hash_table_remove(control->windows, window);
    schedule_windows(control);
}

static void track_window(GnoblinNativeControl* control, MetaWindow* window) {
    if (g_hash_table_contains(control->windows, window))
        return;
    g_hash_table_add(control->windows, g_object_ref(window));
    g_signal_connect(window, "notify", G_CALLBACK(window_notified), control);
    g_signal_connect(window, "position-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "size-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "workspace-changed", G_CALLBACK(window_changed), control);
    g_signal_connect(window, "unmanaged", G_CALLBACK(window_unmanaged), control);
    schedule_windows(control);
}

static void window_created(MetaDisplay* display, MetaWindow* window, gpointer user_data) {
    track_window(user_data, window);
}

static void display_notified(GObject* display, GParamSpec* property, gpointer user_data) {
    schedule_windows(user_data);
}

static void display_restacked(MetaDisplay* display, gpointer user_data) {
    schedule_windows(user_data);
}

static void read_request(Client* client);

static void process_buffer(Client* client) {
    if (client->closing || client->writing)
        return;
    const char* newline = memchr(client->request->str, '\n', client->request->len);
    if (newline) {
        gsize line_length = newline - client->request->str;
        if (line_length > MAX_REQUEST_BYTES) {
            client->close_after_response = TRUE;
            send_response(client, encode_response("", NULL, "request is too large"));
            return;
        }
        char* response = handle_request(client, client->request->str, line_length);
        g_string_erase(client->request, 0, line_length + 1);
        send_response(client, response);
        return;
    }
    if (client->request->len > MAX_REQUEST_BYTES) {
        client->close_after_response = TRUE;
        send_response(client, encode_response("", NULL, "request is too large"));
        return;
    }
    if (!client->reading)
        read_request(client);
}

static void read_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    Client* client = user_data;
    g_autoptr(GError) error = NULL;
    g_autoptr(GBytes) bytes =
        g_input_stream_read_bytes_finish(G_INPUT_STREAM(source), result, &error);
    client->reading = FALSE;
    if (client->closing || !client->control || !bytes || g_bytes_get_size(bytes) == 0) {
        client_close(client);
        return;
    }
    gsize length;
    const char* data = g_bytes_get_data(bytes, &length);
    g_string_append_len(client->request, data, length);
    process_buffer(client);
}

static void read_request(Client* client) {
    client->reading = TRUE;
    g_input_stream_read_bytes_async(g_io_stream_get_input_stream(G_IO_STREAM(client->connection)),
                                    4096, G_PRIORITY_DEFAULT, NULL, read_done, client);
}

static gboolean client_connected(GSocketService* service, GSocketConnection* connection,
                                 GObject* source_object, gpointer user_data) {
    GnoblinNativeControl* control = user_data;
    Client* client = g_new0(Client, 1);
    client->control = control;
    client->connection = g_object_ref(connection);
    client->request = g_string_new(NULL);
    client->outgoing = g_queue_new();
    g_hash_table_add(control->clients, client);
    send_response(client, g_strdup("{\"event\":\"hello\",\"version\":1,\"features\":[]}\n"));
    return TRUE;
}

GnoblinNativeControl* gnoblin_native_control_start(MetaContext* context, GVariant* document,
                                                   GError** error) {
    const char* runtime = g_getenv("XDG_RUNTIME_DIR");
    if (!runtime || !*runtime) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "XDG_RUNTIME_DIR is required for native control");
        return NULL;
    }
    g_autofree char* directory = g_build_filename(runtime, "gnoblin", NULL);
    if (g_mkdir_with_parents(directory, 0700) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno), "cannot create %s: %s",
                    directory, g_strerror(errno));
        return NULL;
    }
    struct stat directory_stat;
    if (lstat(directory, &directory_stat) != 0 || !S_ISDIR(directory_stat.st_mode) ||
        directory_stat.st_uid != getuid() || (directory_stat.st_mode & 077) != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                    "%s must be a private directory owned by this user", directory);
        return NULL;
    }
    GnoblinNativeControl* control = g_new0(GnoblinNativeControl, 1);
    control->clients = g_hash_table_new(g_direct_hash, g_direct_equal);
    control->windows = g_hash_table_new_full(g_direct_hash, g_direct_equal, g_object_unref, NULL);
    g_autoptr(GSocketAddress) bind_address = NULL;
    g_autoptr(GList) windows = NULL;
    control->display = meta_context_get_display(context);
    control->path = g_strdup(g_getenv("GNOBLIN_COMPOSITOR_SOCKET"));
    if (!control->path)
        control->path = g_build_filename(directory, "compositor-v1.sock", NULL);
    g_autofree char* socket_directory = g_path_get_dirname(control->path);
    if (!meta_gnoblin_initialize_native_workspaces(control->display, error))
        goto fail;
    struct stat socket_directory_stat;
    if (lstat(socket_directory, &socket_directory_stat) != 0 ||
        !S_ISDIR(socket_directory_stat.st_mode) || socket_directory_stat.st_uid != getuid() ||
        (socket_directory_stat.st_mode & 077) != 0) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                    "%s must be a private directory owned by this user", socket_directory);
        goto fail;
    }

    struct stat existing;
    if (lstat(control->path, &existing) == 0) {
        if (!S_ISSOCK(existing.st_mode) || existing.st_uid != getuid()) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS, "refusing to replace %s",
                        control->path);
            goto fail;
        }
        g_autoptr(GSocketClient) probe = g_socket_client_new();
        g_socket_client_set_timeout(probe, 1);
        g_autoptr(GSocketAddress) address = g_unix_socket_address_new(control->path);
        g_autoptr(GError) probe_error = NULL;
        g_autoptr(GSocketConnection) active =
            g_socket_client_connect(probe, G_SOCKET_CONNECTABLE(address), NULL, &probe_error);
        if (active || !g_error_matches(probe_error, G_IO_ERROR, G_IO_ERROR_CONNECTION_REFUSED)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                        "Gnoblin compositor socket is already active: %s", control->path);
            goto fail;
        }
        if (g_unlink(control->path) != 0) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "cannot replace stale socket %s: %s", control->path, g_strerror(errno));
            goto fail;
        }
    }
    control->service = g_socket_service_new();
    bind_address = g_unix_socket_address_new(control->path);
    if (!g_socket_listener_add_address(G_SOCKET_LISTENER(control->service), bind_address,
                                       G_SOCKET_TYPE_STREAM, G_SOCKET_PROTOCOL_DEFAULT, NULL, NULL,
                                       error))
        goto fail;
    if (lstat(control->path, &existing) == 0 && S_ISSOCK(existing.st_mode)) {
        control->device = existing.st_dev;
        control->inode = existing.st_ino;
    }
    if (!control->inode || g_chmod(control->path, 0600) != 0) {
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "cannot protect native control socket %s: %s", control->path,
                    g_strerror(errno));
        goto fail;
    }
    g_signal_connect(control->service, "incoming", G_CALLBACK(client_connected), control);
    g_socket_service_start(control->service);
    g_signal_connect(control->display, "window-created", G_CALLBACK(window_created), control);
    g_signal_connect(control->display, "notify::focus-window", G_CALLBACK(display_notified),
                     control);
    g_signal_connect(control->display, "restacked", G_CALLBACK(display_restacked), control);
    windows = meta_display_list_all_windows(control->display);
    for (GList* item = windows; item; item = item->next)
        track_window(control, item->data);
    if (!apply_native_input(control, context, document, error))
        goto fail;
    if (!apply_native_keybindings(document, error))
        goto fail;
    if (!start_native_shortcuts(control, document, error))
        goto fail;
    if (!start_native_autostart(document, error))
        goto fail;
    return control;

fail:
    gnoblin_native_control_stop(control);
    return NULL;
}

void gnoblin_native_control_stop(GnoblinNativeControl* control) {
    if (!control)
        return;
    if (control->service) {
        g_socket_service_stop(control->service);
        g_clear_object(&control->service);
    }
    if (control->publish_id)
        g_source_remove(control->publish_id);
    if (control->display)
        g_signal_handlers_disconnect_by_data(control->display, control);
    if (control->shortcuts) {
        for (guint i = 0; i < control->shortcuts->len; i++) {
            NativeShortcut* shortcut = g_ptr_array_index(control->shortcuts, i);
            if (!shortcut->overlay)
                meta_display_ungrab_accelerator(control->display, shortcut->action);
        }
        g_ptr_array_unref(control->shortcuts);
    }
    if (control->windows) {
        GHashTableIter window_iter;
        gpointer window;
        g_hash_table_iter_init(&window_iter, control->windows);
        while (g_hash_table_iter_next(&window_iter, &window, NULL))
            g_signal_handlers_disconnect_by_data(window, control);
        g_hash_table_unref(control->windows);
    }
    if (control->clients) {
        GHashTableIter iter;
        gpointer value;
        g_hash_table_iter_init(&iter, control->clients);
        while (g_hash_table_iter_next(&iter, &value, NULL)) {
            Client* client = value;
            client->control = NULL;
            client_close(client);
        }
        g_hash_table_unref(control->clients);
    }
    struct stat current;
    if (control->path && control->inode && lstat(control->path, &current) == 0 &&
        current.st_dev == control->device && current.st_ino == control->inode)
        g_unlink(control->path);
    g_free(control->path);
    g_free(control);
}
