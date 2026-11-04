/*
 * Gnoblin native control dispatch: runtime events, monitor, layer, window and
 * workspace operations that the native control socket and the Lua runtime
 * call into the compositor for.
 *
 * This code lived in src/core/util.c as a stack of patches. It uses only
 * Mutter's public and private headers, so it is a Gnoblin overlay now.
 *
 * Copyright (C) 2026 Gnoblin contributors
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU General Public License as
 * published by the Free Software Foundation; either version 2 of the
 * License, or (at your option) any later version.
 */

#include "config.h"

#include "core/gnoblin-native-dispatch.h"

#include "core/display-private.h"
#include "core/meta-workspace-manager-private.h"
#include "core/util-private.h"
#include "compositor/compositor-private.h"
#include "core/gnoblin-native-control.h"
#include "core/gnoblin-animation.h"
#include "wayland/gnoblin-lua-pattern.h"

#include <gio/gunixinputstream.h>
#include <glib/gstdio.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>

#include "clutter/clutter-mutter.h"
#include "cogl/cogl.h"
#include "meta/common.h"
#include "meta/main.h"
#include "meta/meta-backend.h"
#include "meta/meta-context.h"
#include "backends/meta-logical-monitor-private.h"
#include "meta/meta-logical-monitor.h"
#include "meta/meta-monitor-manager.h"
#include "meta/meta-monitor.h"
#include "meta/meta-workspace-manager.h"
#include "meta/prefs.h"
#include "meta/window.h"
#include "meta/workspace.h"

#include "wayland/meta-wayland-layer-shell.h"

gboolean meta_gnoblin_window_pattern_match(const char* pattern, const char* value, GError** error) {
    gboolean matched = FALSE;

    if (!gnoblin_lua_pattern_match(pattern, value, &matched, error))
        return FALSE;
    return matched;
}

/**
 * meta_gnoblin_layer_anchor:
 * @window: window to inspect
 *
 * Returns: layer-shell anchor flags, or -1 for other windows
 */
int meta_gnoblin_layer_anchor(MetaWindow* window) {
    return GPOINTER_TO_INT(g_object_get_data(G_OBJECT(window), "gnoblin-layer-anchor")) - 1;
}

/**
 * meta_gnoblin_layer_namespace:
 * @window: window to inspect
 *
 * Returns: (transfer none) (nullable): the layer namespace, or NULL
 */
const char* meta_gnoblin_layer_namespace(MetaWindow* window) {
    return g_object_get_data(G_OBJECT(window), "gnoblin-layer-namespace");
}

gboolean meta_is_gnoblin_session(void) {
    return gnoblin_native_control_is_session(NULL);
}

gboolean meta_display_dispatch_gnoblin_event(MetaDisplay* display, const char* event,
                                             GVariant* payload) {
#ifdef HAVE_WAYLAND
    g_autoptr(GError) error = NULL;
    gboolean claimed = FALSE;

    if (!gnoblin_native_control_is_session(display))
        return FALSE;
    if (!gnoblin_native_control_dispatch_runtime_event(display, event, payload, &claimed, &error))
        g_warning("gnoblin runtime event %s: %s", event,
                  error ? error->message : "could not queue event");
    return claimed;
#else
    return FALSE;
#endif
}

gboolean meta_display_dispatch_gnoblin_shortcut_event(MetaDisplay* display, const char* event,
                                                      GVariant* payload, guint64 context_handle,
                                                      guint64 generation, gint64 expires_at_us) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) event_payload = NULL;
    GVariantDict payload_dict;
    gboolean claimed = FALSE;

    if (!payload || !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT) || !context_handle ||
        !generation || expires_at_us <= g_get_monotonic_time())
        return FALSE;

    g_variant_dict_init(&payload_dict, payload);
    g_variant_dict_insert(&payload_dict, "focus_context_handle", "t", context_handle);
    g_variant_dict_insert(&payload_dict, "focus_context_generation", "t", generation);
    g_variant_dict_insert(&payload_dict, "focus_context_expires_at_us", "x", expires_at_us);
    event_payload = g_variant_ref_sink(g_variant_dict_end(&payload_dict));

    if (!gnoblin_native_control_is_session(display))
        return FALSE;
    if (!gnoblin_native_control_dispatch_runtime_event(display, event, event_payload, &claimed,
                                                       &error))
        g_warning("gnoblin runtime shortcut event %s: %s", event,
                  error ? error->message : "could not queue event");
    return claimed;
}

void meta_display_dispatch_gnoblin_native_event(MetaDisplay* display, const char* event,
                                                GVariant* payload) {
    g_return_if_fail(META_IS_DISPLAY(display));
    g_return_if_fail(event != NULL && *event != '\0');
    g_return_if_fail(payload != NULL && g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT));
    g_signal_emit_by_name(display, "gnoblin-native-event", event, payload);
}

#define GNOBLIN_WORKSPACE_ID "gnoblin-native-id"
#define GNOBLIN_WORKSPACE_NAME "gnoblin-native-name"
#define GNOBLIN_WORKSPACE_PERSISTENT "gnoblin-native-persistent"
#define GNOBLIN_WORKSPACE_SLOT "gnoblin-native-configured-slot"

static guint64 next_native_workspace_id = 1;

typedef struct {
    char* id;
    char* name;
} GnoblinNativeWorkspaceDeclaration;

typedef struct {
    guint index;
    char* id;
    int slot;
} GnoblinNativeWorkspaceAssignment;

static void gnoblin_native_workspace_declaration_free(gpointer data) {
    GnoblinNativeWorkspaceDeclaration* declaration = data;

    g_free(declaration->id);
    g_free(declaration->name);
    g_free(declaration);
}

static void gnoblin_native_workspace_assignment_free(gpointer data) {
    GnoblinNativeWorkspaceAssignment* assignment = data;

    g_free(assignment->id);
    g_free(assignment);
}

static gboolean gnoblin_native_workspace_id_is_valid(const char* id) {
    gsize length = strlen(id);

    if (length == 0 || length > 64 || !g_ascii_isalnum(id[0]))
        return FALSE;

    for (const char* p = id + 1; *p; p++)
        if (!g_ascii_isalnum(*p) && *p != '.' && *p != '_' && *p != '-')
            return FALSE;

    return TRUE;
}

static gboolean gnoblin_native_workspace_id_is_session(const char* id) {
    const char* suffix;

    if (!g_str_has_prefix(id, "@session-"))
        return FALSE;

    suffix = id + strlen("@session-");
    if (!*suffix || *suffix == '0')
        return FALSE;

    for (const char* p = suffix; *p; p++)
        if (!g_ascii_isdigit(*p))
            return FALSE;

    return TRUE;
}

static char* gnoblin_native_workspace_new_session_id(GHashTable* reserved_ids, guint64* next_id) {
    char* id;

    while (TRUE) {
        id = g_strdup_printf("@session-%" G_GUINT64_FORMAT, (*next_id)++);
        if (!g_hash_table_contains(reserved_ids, id))
            break;
        g_free(id);
    }

    g_hash_table_add(reserved_ids, g_strdup(id));
    return id;
}

static const char* gnoblin_native_workspace_id(MetaWorkspace* workspace) {
    const char* id = g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID);
    if (!id) {
        g_autofree char* generated =
            g_strdup_printf("@session-%" G_GUINT64_FORMAT, next_native_workspace_id++);
        g_object_set_data_full(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID,
                               g_steal_pointer(&generated), g_free);
        id = g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID);
    }
    return id;
}

const char* meta_gnoblin_workspace_id(MetaWorkspace* workspace) {
    return gnoblin_native_workspace_id(workspace);
}

gboolean meta_gnoblin_initialize_native_workspaces(MetaDisplay* display, GError** error) {
    g_autoptr(GVariant) document = gnoblin_native_control_get_config_document(display);
    MetaWorkspaceManager* manager = meta_display_get_workspace_manager(display);
    g_autoptr(GVariant) configured =
        document ? g_variant_lookup_value(document, "workspaces", G_VARIANT_TYPE("av")) : NULL;
    g_autoptr(GPtrArray) declarations =
        g_ptr_array_new_with_free_func(gnoblin_native_workspace_declaration_free);
    g_autoptr(GHashTable) declared_ids =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GPtrArray) assignments =
        g_ptr_array_new_with_free_func(gnoblin_native_workspace_assignment_free);
    g_autoptr(GHashTable) reserved_ids =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    g_autoptr(GHashTable) workspace_id_owners = g_hash_table_new(g_str_hash, g_str_equal);
    g_autoptr(GHashTable) changing_workspaces = g_hash_table_new(g_direct_hash, g_direct_equal);
    g_autoptr(GHashTable) assigned_ids = g_hash_table_new(g_str_hash, g_str_equal);
    g_autoptr(GPtrArray) current_workspaces = g_ptr_array_new();
    int current_count;
    int desired_count;
    guint64 next_workspace_id = next_native_workspace_id;

    if (configured) {
        gsize desired_size = g_variant_n_children(configured);

        if (desired_size < 1 || desired_size > G_MAXINT) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "workspaces must contain at least one entry");
            return FALSE;
        }

        for (gsize index = 0; index < desired_size; index++) {
            g_autoptr(GVariant) boxed = g_variant_get_child_value(configured, index);
            g_autoptr(GVariant) entry = g_variant_get_variant(boxed);
            GnoblinNativeWorkspaceDeclaration* declaration =
                g_new0(GnoblinNativeWorkspaceDeclaration, 1);
            const char* id = NULL;
            const char* name = NULL;
            g_autofree char* trimmed_name = NULL;

            if (!g_variant_lookup(entry, "id", "&s", &id) ||
                !g_variant_lookup(entry, "name", "&s", &name) ||
                !gnoblin_native_workspace_id_is_valid(id)) {
                g_free(declaration);
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "configured workspace needs a valid ID and name");
                return FALSE;
            }

            trimmed_name = g_strdup(name);
            if (!*g_strstrip(trimmed_name)) {
                g_free(declaration);
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "configured workspace name must not be empty");
                return FALSE;
            }

            if (!g_hash_table_add(declared_ids, g_strdup(id))) {
                g_free(declaration);
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "duplicate workspace ID: %s", id);
                return FALSE;
            }

            declaration->id = g_strdup(id);
            declaration->name = g_strdup(name);
            g_ptr_array_add(declarations, declaration);
        }
    }

    current_count = meta_workspace_manager_get_n_workspaces(manager);
    desired_count = configured ? declarations->len : current_count;
    for (int index = 0; index < current_count; index++) {
        MetaWorkspace* workspace = meta_workspace_manager_get_workspace_by_index(manager, index);
        const char* id = g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID);

        g_ptr_array_add(current_workspaces, workspace);
        if (id) {
            g_hash_table_add(reserved_ids, g_strdup(id));
            if (index < desired_count) {
                if (g_hash_table_contains(workspace_id_owners, id)) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "duplicate native workspace ID: %s", id);
                    return FALSE;
                }
                g_hash_table_insert(workspace_id_owners, (gpointer)id, GUINT_TO_POINTER(index + 1));
            }
        }
    }

    for (guint index = 0; index < declarations->len; index++) {
        GnoblinNativeWorkspaceDeclaration* declaration = g_ptr_array_index(declarations, index);
        g_hash_table_add(reserved_ids, g_strdup(declaration->id));
    }

    for (int index = 0; index < desired_count; index++) {
        MetaWorkspace* workspace =
            index < current_count ? g_ptr_array_index(current_workspaces, index) : NULL;
        const char* current_id =
            workspace ? g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID) : NULL;
        int configured_slot =
            workspace
                ? GPOINTER_TO_INT(g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_SLOT)) -
                      1
                : -1;
        const char* target_id = NULL;
        int target_slot = -1;
        g_autofree char* generated_id = NULL;

        if (configured_slot >= 0) {
            if (configured_slot < (int)declarations->len) {
                GnoblinNativeWorkspaceDeclaration* declaration =
                    g_ptr_array_index(declarations, configured_slot);
                target_id = declaration->id;
                target_slot = configured_slot;
            } else {
                generated_id =
                    gnoblin_native_workspace_new_session_id(reserved_ids, &next_workspace_id);
                target_id = generated_id;
            }
        } else if ((guint)index < declarations->len &&
                   (!current_id || gnoblin_native_workspace_id_is_session(current_id))) {
            GnoblinNativeWorkspaceDeclaration* declaration = g_ptr_array_index(declarations, index);
            target_id = declaration->id;
            target_slot = index;
        }

        if (target_id && g_strcmp0(target_id, current_id) != 0) {
            GnoblinNativeWorkspaceAssignment* assignment =
                g_new0(GnoblinNativeWorkspaceAssignment, 1);
            assignment->index = index;
            assignment->id = g_strdup(target_id);
            assignment->slot = target_slot;
            g_ptr_array_add(assignments, assignment);
            g_hash_table_add(changing_workspaces, GUINT_TO_POINTER(index + 1));
        }
    }

    for (guint index = 0; index < assignments->len; index++) {
        GnoblinNativeWorkspaceAssignment* assignment = g_ptr_array_index(assignments, index);

        if (!g_hash_table_add(assigned_ids, assignment->id)) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "workspace ID is already in use: %s", assignment->id);
            return FALSE;
        }

        gpointer owner_data = g_hash_table_lookup(workspace_id_owners, assignment->id);
        if (owner_data) {
            guint owner_index = GPOINTER_TO_UINT(owner_data) - 1;

            if (owner_index != assignment->index &&
                !g_hash_table_contains(changing_workspaces, GUINT_TO_POINTER(owner_index + 1))) {
                g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "workspace ID is already in use: %s", assignment->id);
                return FALSE;
            }
        }
    }

    next_native_workspace_id = next_workspace_id;
    g_object_set_data(G_OBJECT(manager), "gnoblin-native-workspace-count", GINT_TO_POINTER(1));
    if (configured)
        meta_workspace_manager_update_num_workspaces(
            manager, meta_display_get_current_time(display), desired_count);

    for (guint index = 0; index < assignments->len; index++) {
        GnoblinNativeWorkspaceAssignment* assignment = g_ptr_array_index(assignments, index);
        MetaWorkspace* workspace =
            meta_workspace_manager_get_workspace_by_index(manager, assignment->index);

        g_object_set_data_full(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID, g_strdup(assignment->id),
                               g_free);
        g_object_set_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_SLOT,
                          assignment->slot >= 0 ? GINT_TO_POINTER(assignment->slot + 1) : NULL);
    }

    int n_workspaces = meta_workspace_manager_get_n_workspaces(manager);
    for (int index = 0; index < n_workspaces; index++) {
        MetaWorkspace* workspace = meta_workspace_manager_get_workspace_by_index(manager, index);

        g_object_set_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_PERSISTENT, NULL);
        gnoblin_native_workspace_id(workspace);
    }

    for (guint index = 0; index < declarations->len; index++) {
        GnoblinNativeWorkspaceDeclaration* declaration = g_ptr_array_index(declarations, index);

        for (int workspace_index = 0; workspace_index < n_workspaces; workspace_index++) {
            MetaWorkspace* workspace =
                meta_workspace_manager_get_workspace_by_index(manager, workspace_index);
            const char* id = g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID);

            if (g_strcmp0(id, declaration->id) == 0) {
                /* Lua config owns these names; do not persist them in Mutter settings. */
                g_object_set_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_PERSISTENT,
                                  GINT_TO_POINTER(1));
                g_object_set_data_full(G_OBJECT(workspace), GNOBLIN_WORKSPACE_NAME,
                                       g_strdup(declaration->name), g_free);
                break;
            }
        }
    }

    return TRUE;
}

static GVariant* gnoblin_native_workspace_record(MetaWorkspaceManager* manager,
                                                 MetaWorkspace* workspace) {
    int index = meta_workspace_index(workspace);
    g_autoptr(GList) windows = meta_workspace_list_windows(workspace);
    const char* name = g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_NAME);
    if (!name)
        name = meta_prefs_get_workspace_name(index);
    GVariantBuilder record;
    g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&record, "{sv}", "id",
                          g_variant_new_string(gnoblin_native_workspace_id(workspace)));
    g_variant_builder_add(&record, "{sv}", "number", g_variant_new_int32(index + 1));
    g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string(name));
    g_variant_builder_add(
        &record, "{sv}", "active",
        g_variant_new_boolean(index == meta_workspace_manager_get_active_workspace_index(manager)));
    g_variant_builder_add(&record, "{sv}", "windows", g_variant_new_int32(g_list_length(windows)));
    g_variant_builder_add(
        &record, "{sv}", "persistent",
        g_variant_new_boolean(
            g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_PERSISTENT) != NULL));
    return g_variant_ref_sink(g_variant_builder_end(&record));
}

static MetaWorkspace* gnoblin_native_workspace_resolve(MetaWorkspaceManager* manager,
                                                       GVariant* selector, GError** error) {
    if (!selector || !g_variant_is_of_type(selector, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "workspace selector must specify an ID or number");
        return NULL;
    }
    g_autoptr(GVariant) id = g_variant_lookup_value(selector, "id", NULL);
    if (id) {
        const char* wanted = g_variant_get_string(id, NULL);
        for (int index = 0; index < meta_workspace_manager_get_n_workspaces(manager); index++) {
            MetaWorkspace* workspace =
                meta_workspace_manager_get_workspace_by_index(manager, index);
            if (g_str_equal(wanted, gnoblin_native_workspace_id(workspace)))
                return workspace;
        }
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "workspace ID not found; list workspaces first");
        return NULL;
    }
    gint64 number;
    if (!g_variant_lookup(selector, "number", "x", &number) || number < 1 ||
        number > meta_workspace_manager_get_n_workspaces(manager)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "workspace not found; list workspaces first");
        return NULL;
    }
    return meta_workspace_manager_get_workspace_by_index(manager, number - 1);
}

static GVariant* gnoblin_native_workspace_api(MetaDisplay* display, const char* method,
                                              GVariant* arguments, GError** error) {
    MetaWorkspaceManager* manager = meta_display_get_workspace_manager(display);
    int count = meta_workspace_manager_get_n_workspaces(manager);
    if (g_str_equal(method, "workspace.list")) {
        GVariantBuilder workspaces;
        GVariantBuilder result;
        g_variant_builder_init(&workspaces, G_VARIANT_TYPE("aa{sv}"));
        for (int index = 0; index < count; index++) {
            MetaWorkspace* workspace =
                meta_workspace_manager_get_workspace_by_index(manager, index);
            g_autoptr(GVariant) record = gnoblin_native_workspace_record(manager, workspace);
            g_variant_builder_add_value(&workspaces, record);
        }
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "workspaces", g_variant_builder_end(&workspaces));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }

    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    if (meta_backend_get_gnoblin_session_lock_active(backend)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "workspace switching is unavailable while the session is locked");
        return NULL;
    }
    if (g_str_equal(method, "workspace.create")) {
        g_autoptr(GVariant) id = g_variant_lookup_value(arguments, "id", NULL);
        if (id) {
            const char* wanted = g_variant_get_string(id, NULL);
            for (int index = 0; index < count; index++) {
                MetaWorkspace* existing =
                    meta_workspace_manager_get_workspace_by_index(manager, index);
                if (g_str_equal(wanted, gnoblin_native_workspace_id(existing))) {
                    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_EXISTS,
                                        "workspace ID already exists");
                    return NULL;
                }
            }
        }
        const char* name = NULL;
        if (!g_variant_lookup(arguments, "name", "&s", &name) || !name || !*name ||
            !g_utf8_validate(name, -1, NULL)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "workspace name must be a nonempty string");
            return NULL;
        }
        gboolean activate = FALSE;
        g_variant_lookup(arguments, "activate", "b", &activate);
        MetaWorkspace* workspace = meta_workspace_manager_append_new_workspace(
            manager, FALSE, meta_display_get_current_time(display));
        if (!workspace) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "could not create workspace");
            return NULL;
        }
        if (id)
            g_object_set_data_full(G_OBJECT(workspace), GNOBLIN_WORKSPACE_ID,
                                   g_strdup(g_variant_get_string(id, NULL)), g_free);
        g_object_set_data_full(G_OBJECT(workspace), GNOBLIN_WORKSPACE_NAME, g_strdup(name), g_free);
        if (activate)
            meta_workspace_activate(workspace, meta_display_get_current_time(display));
        g_signal_emit_by_name(manager, "gnoblin-workspace-state-changed");
        return gnoblin_native_workspace_record(manager, workspace);
    }
    if (count < 1) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "no workspaces are available");
        return NULL;
    }
    if (g_str_equal(method, "workspace.rename") || g_str_equal(method, "workspace.remove") ||
        g_str_equal(method, "workspace.switch")) {
        MetaWorkspace* workspace = gnoblin_native_workspace_resolve(manager, arguments, error);
        if (!workspace)
            return NULL;
        if (g_str_equal(method, "workspace.rename")) {
            const char* name = NULL;
            if (!g_variant_lookup(arguments, "name", "&s", &name) || !name || !*name ||
                !g_utf8_validate(name, -1, NULL)) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "workspace name must be a nonempty string");
                return NULL;
            }
            g_object_set_data_full(G_OBJECT(workspace), GNOBLIN_WORKSPACE_NAME, g_strdup(name),
                                   g_free);
            g_signal_emit_by_name(manager, "gnoblin-workspace-state-changed");
            return gnoblin_native_workspace_record(manager, workspace);
        }
        if (g_str_equal(method, "workspace.remove")) {
            if (g_object_get_data(G_OBJECT(workspace), GNOBLIN_WORKSPACE_PERSISTENT)) {
                g_set_error_literal(
                    error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                    "configured workspace cannot be removed; edit workspaces in config");
                return NULL;
            }
            g_autoptr(GList) windows = meta_workspace_list_windows(workspace);
            if (workspace == meta_workspace_manager_get_active_workspace(manager) || windows ||
                count <= 1) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                    "cannot remove the active, occupied, or only workspace");
                return NULL;
            }
            g_autoptr(GVariant) record = gnoblin_native_workspace_record(manager, workspace);
            meta_workspace_manager_remove_workspace(manager, workspace,
                                                    meta_display_get_current_time(display));
            return g_steal_pointer(&record);
        }
        meta_workspace_activate(workspace, meta_display_get_current_time(display));
        return gnoblin_native_workspace_record(manager, workspace);
    }
    if (g_str_equal(method, "workspace.move_active") ||
        g_str_equal(method, "workspace.move_window")) {
        g_autoptr(GVariant) selector =
            g_variant_lookup_value(arguments, "workspace", G_VARIANT_TYPE_VARDICT);
        MetaWorkspace* workspace = gnoblin_native_workspace_resolve(manager, selector, error);
        if (!workspace)
            return NULL;
        MetaWindow* window = meta_display_get_focus_window(display);
        if (g_str_equal(method, "workspace.move_window")) {
            const char* window_id = NULL;
            if (!g_variant_lookup(arguments, "window", "&s", &window_id))
                window = NULL;
            else if (!g_str_equal(window_id, "active")) {
                window = NULL;
                g_autoptr(GList) windows = meta_display_list_all_windows(display);
                for (GList* item = windows; item; item = item->next) {
                    MetaWindow* candidate = item->data;
                    g_autofree char* id =
                        g_strdup_printf("%u", meta_window_get_stable_sequence(candidate));
                    if (g_str_equal(window_id, id)) {
                        window = candidate;
                        break;
                    }
                }
            }
        }
        if (!window || meta_window_is_skip_taskbar(window) ||
            meta_window_is_override_redirect(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "window no longer available; list windows first");
            return NULL;
        }
        gboolean follow = FALSE;
        g_variant_lookup(arguments, "follow", "b", &follow);
        g_autofree char* window_id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
        meta_window_change_workspace(window, workspace);
        if (follow)
            meta_workspace_activate(workspace, meta_display_get_current_time(display));
        GVariantBuilder result;
        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) workspace_record = gnoblin_native_workspace_record(manager, workspace);
        g_variant_builder_add(&result, "{sv}", "workspace", workspace_record);
        g_variant_builder_add(&result, "{sv}", "window", g_variant_new_string(window_id));
        g_variant_builder_add(&result, "{sv}", "follow", g_variant_new_boolean(follow));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }
    int index = meta_workspace_manager_get_active_workspace_index(manager);
    if (g_str_equal(method, "workspace.next"))
        index = (index + 1) % count;
    else
        index = (index + count - 1) % count;
    MetaWorkspace* workspace = meta_workspace_manager_get_workspace_by_index(manager, index);
    meta_workspace_activate(workspace, meta_display_get_current_time(display));
    return gnoblin_native_workspace_record(manager, workspace);
}

static gboolean gnoblin_native_variant_has_only_keys(GVariant* arguments,
                                                     const char* const* allowed, GError** error);

static MetaLogicalMonitor* gnoblin_native_logical_monitor_for_index(MetaDisplay* display,
                                                                    int monitor_index) {
    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    MetaMonitorManager* manager = meta_backend_get_monitor_manager(backend);
    GList* logical_monitors = meta_monitor_manager_get_logical_monitors(manager);

    for (GList* item = logical_monitors; item; item = item->next) {
        MetaLogicalMonitor* logical_monitor = item->data;

        if (meta_logical_monitor_get_number(logical_monitor) == monitor_index)
            return logical_monitor;
    }

    return NULL;
}

static MetaMonitor* gnoblin_native_monitor_for_index(MetaDisplay* display, int monitor_index) {
    MetaLogicalMonitor* logical_monitor =
        gnoblin_native_logical_monitor_for_index(display, monitor_index);
    MetaMonitor* canonical_monitor = NULL;
    GList* monitors;

    if (!logical_monitor)
        return NULL;

    monitors = meta_logical_monitor_get_monitors(logical_monitor);
    for (GList* item = monitors; item; item = item->next) {
        MetaMonitor* monitor = item->data;
        const char* connector;
        const char* canonical_connector;

        if (!meta_monitor_is_active(monitor))
            continue;

        connector = meta_monitor_get_connector(monitor);
        canonical_connector =
            canonical_monitor ? meta_monitor_get_connector(canonical_monitor) : NULL;
        if (connector && *connector &&
            (!canonical_connector || g_strcmp0(connector, canonical_connector) < 0))
            canonical_monitor = monitor;
    }

    return canonical_monitor;
}

static char* gnoblin_native_monitor_id_for_index(MetaDisplay* display, int monitor_index) {
    MetaMonitor* monitor = gnoblin_native_monitor_for_index(display, monitor_index);
    const char* connector = monitor ? meta_monitor_get_connector(monitor) : NULL;

    return connector && *connector ? g_strdup(connector) : NULL;
}

static const char* gnoblin_native_monitor_transform_name(MtkMonitorTransform transform) {
    switch (transform) {
    case MTK_MONITOR_TRANSFORM_NORMAL:
        return "normal";
    case MTK_MONITOR_TRANSFORM_90:
        return "90";
    case MTK_MONITOR_TRANSFORM_180:
        return "180";
    case MTK_MONITOR_TRANSFORM_270:
        return "270";
    case MTK_MONITOR_TRANSFORM_FLIPPED:
        return "flipped";
    case MTK_MONITOR_TRANSFORM_FLIPPED_90:
        return "flipped-90";
    case MTK_MONITOR_TRANSFORM_FLIPPED_180:
        return "flipped-180";
    case MTK_MONITOR_TRANSFORM_FLIPPED_270:
        return "flipped-270";
    }

    g_assert_not_reached();
}

static int gnoblin_native_monitor_selector_resolve(MetaDisplay* display, GVariant* arguments,
                                                   GError** error) {
    static const char* const monitor_selector_keys[] = {"id", NULL};
    g_autoptr(GVariant) selector = g_variant_lookup_value(arguments, "monitor", NULL);
    const char* monitor_id = NULL;
    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    MetaMonitorManager* manager = meta_backend_get_monitor_manager(backend);
    int monitor_index;

    if (selector && g_variant_is_of_type(selector, G_VARIANT_TYPE_STRING))
        monitor_id = g_variant_get_string(selector, NULL);
    else if (selector && g_variant_is_of_type(selector, G_VARIANT_TYPE_VARDICT) &&
             gnoblin_native_variant_has_only_keys(selector, monitor_selector_keys, error))
        g_variant_lookup(selector, "id", "&s", &monitor_id);
    else {
        if (error && !*error)
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.move_to_monitor requires a monitor ID or {id = string}");
        return -1;
    }

    if (!monitor_id || !*monitor_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.move_to_monitor requires a nonempty monitor ID");
        return -1;
    }

    monitor_index = meta_monitor_manager_get_monitor_for_connector(manager, monitor_id);
    if (monitor_index < 0 || monitor_index >= meta_display_get_n_monitors(display)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "monitor ID is no longer available");
        return -1;
    }

    g_autofree char* canonical_id = gnoblin_native_monitor_id_for_index(display, monitor_index);
    if (!canonical_id || !g_str_equal(canonical_id, monitor_id)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "monitor ID no longer identifies a listed monitor");
        return -1;
    }

    return monitor_index;
}

static GVariant* gnoblin_native_window_list(MetaDisplay* display, GVariant* arguments,
                                            GError** error) {
    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    if (meta_backend_get_gnoblin_session_lock_active(backend)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window listing is unavailable while the session is locked");
        return NULL;
    }

    const char* app_filter = NULL;
    const char* title_filter = NULL;
    gboolean focused_only = FALSE;
    g_autoptr(GVariant) app_value = NULL;
    g_autoptr(GVariant) title_value = NULL;
    g_autoptr(GVariant) focused_value = NULL;
    if (arguments) {
        app_value = g_variant_lookup_value(arguments, "app_id", NULL);
        title_value = g_variant_lookup_value(arguments, "title", NULL);
        focused_value = g_variant_lookup_value(arguments, "focused", NULL);
        if ((app_value && !g_variant_is_of_type(app_value, G_VARIANT_TYPE_STRING)) ||
            (title_value && !g_variant_is_of_type(title_value, G_VARIANT_TYPE_STRING)) ||
            (focused_value && !g_variant_is_of_type(focused_value, G_VARIANT_TYPE_BOOLEAN))) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window filters must be strings or a focused boolean");
            return NULL;
        }
        if (app_value)
            app_filter = g_variant_get_string(app_value, NULL);
        if (title_value)
            title_filter = g_variant_get_string(title_value, NULL);
        if (focused_value)
            focused_only = g_variant_get_boolean(focused_value);
    }
    g_autofree char* folded_filter = title_filter ? g_utf8_casefold(title_filter, -1) : NULL;
    MetaWindow* focused = meta_display_get_focus_window(display);
    g_autoptr(GList) windows = meta_display_list_all_windows(display);
    GVariantBuilder entries;
    GVariantBuilder result;
    g_variant_builder_init(&entries, G_VARIANT_TYPE("aa{sv}"));
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        if (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
            continue;

        const char* title = meta_window_get_title(window);
        const char* gtk_id = meta_window_get_gtk_application_id(window);
        const char* wm_class = meta_window_get_wm_class(window);
        const char* app_id = gtk_id && *gtk_id ? gtk_id : wm_class;
        const int monitor_index = meta_window_get_monitor(window);
        if (app_filter && g_strcmp0(app_filter, app_id) != 0)
            continue;
        if (focused_only && window != focused)
            continue;
        if (folded_filter) {
            g_autofree char* folded_title = g_utf8_casefold(title ? title : "", -1);
            if (!g_strstr_len(folded_title, -1, folded_filter))
                continue;
        }

        g_autofree char* id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
        MetaWindow* parent = meta_window_get_transient_for(window);
        MetaWorkspace* workspace = meta_window_get_workspace(window);
        const char* role = meta_window_get_role(window);
        MetaWindowType type = meta_window_get_window_type(window);
        MtkRectangle frame;
        gboolean minimized = FALSE;
        gboolean demands_attention = FALSE;
        gboolean is_alive = TRUE;
        meta_window_get_frame_rect(window, &frame);
        g_object_get(window, "minimized", &minimized, "demands-attention", &demands_attention,
                     "is-alive", &is_alive, NULL);
        GVariantBuilder geometry;
        GVariantBuilder entry;
        g_variant_builder_init(&geometry, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&geometry, "{sv}", "x", g_variant_new_int32(frame.x));
        g_variant_builder_add(&geometry, "{sv}", "y", g_variant_new_int32(frame.y));
        g_variant_builder_add(&geometry, "{sv}", "width", g_variant_new_int32(frame.width));
        g_variant_builder_add(&geometry, "{sv}", "height", g_variant_new_int32(frame.height));
        g_variant_builder_init(&entry, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&entry, "{sv}", "id", g_variant_new_string(id));
        g_variant_builder_add(&entry, "{sv}", "title", g_variant_new_string(title ? title : ""));
        g_variant_builder_add(&entry, "{sv}", "appId", g_variant_new_string(app_id ? app_id : ""));
        g_variant_builder_add(&entry, "{sv}", "gtkAppId",
                              g_variant_new_string(gtk_id ? gtk_id : ""));
        g_variant_builder_add(&entry, "{sv}", "wmClass",
                              g_variant_new_string(wm_class ? wm_class : ""));
        g_variant_builder_add(&entry, "{sv}", "ruleAppId",
                              g_variant_new_string(app_id ? app_id : ""));
        g_variant_builder_add(&entry, "{sv}", "focused", g_variant_new_boolean(window == focused));
        g_variant_builder_add(&entry, "{sv}", "minimized", g_variant_new_boolean(minimized));
        g_variant_builder_add(&entry, "{sv}", "above",
                              g_variant_new_boolean(meta_window_is_above(window)));
        g_variant_builder_add(
            &entry, "{sv}", "sticky",
            g_variant_new_boolean(meta_window_is_always_on_all_workspaces(window)));
        g_variant_builder_add(&entry, "{sv}", "demandsAttention",
                              g_variant_new_boolean(demands_attention));
        g_variant_builder_add(&entry, "{sv}", "unresponsive", g_variant_new_boolean(!is_alive));
        g_variant_builder_add(&entry, "{sv}", "closable",
                              g_variant_new_boolean(meta_window_can_close(window)));
        g_variant_builder_add(&entry, "{sv}", "minimizable",
                              g_variant_new_boolean(meta_window_can_minimize(window)));
        g_variant_builder_add(&entry, "{sv}", "maximizable",
                              g_variant_new_boolean(meta_window_can_maximize(window)));
        g_variant_builder_add(&entry, "{sv}", "movable",
                              g_variant_new_boolean(meta_window_allows_move(window)));
        g_variant_builder_add(&entry, "{sv}", "resizable",
                              g_variant_new_boolean(meta_window_allows_resize(window)));
        if (role)
            g_variant_builder_add(&entry, "{sv}", "role", g_variant_new_string(role));
        g_variant_builder_add(&entry, "{sv}", "type", g_variant_new_int32(type));
        g_variant_builder_add(&entry, "{sv}", "lastUserTime",
                              g_variant_new_uint32(meta_window_get_user_time(window)));
        if (parent) {
            g_autofree char* parent_id =
                g_strdup_printf("%u", meta_window_get_stable_sequence(parent));
            g_variant_builder_add(&entry, "{sv}", "parent", g_variant_new_string(parent_id));
        }
        if (workspace) {
            int number = meta_workspace_index(workspace) + 1;
            g_variant_builder_add(&entry, "{sv}", "workspace", g_variant_new_int32(number));
            g_variant_builder_add(&entry, "{sv}", "workspaceNumber", g_variant_new_int32(number));
            g_variant_builder_add(&entry, "{sv}", "workspaceId",
                                  g_variant_new_string(gnoblin_native_workspace_id(workspace)));
        }
        g_variant_builder_add(&entry, "{sv}", "monitorIndex", g_variant_new_int32(monitor_index));
        if (monitor_index >= 0 && monitor_index < meta_display_get_n_monitors(display)) {
            g_autofree char* monitor_id =
                gnoblin_native_monitor_id_for_index(display, monitor_index);
            MtkRectangle monitor_geometry;
            GVariantBuilder monitor;
            if (monitor_id)
                g_variant_builder_add(&entry, "{sv}", "monitorId",
                                      g_variant_new_string(monitor_id));
            meta_display_get_monitor_geometry(display, monitor_index, &monitor_geometry);
            g_variant_builder_init(&monitor, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&monitor, "{sv}", "x", g_variant_new_int32(monitor_geometry.x));
            g_variant_builder_add(&monitor, "{sv}", "y", g_variant_new_int32(monitor_geometry.y));
            g_variant_builder_add(&entry, "{sv}", "monitor", g_variant_builder_end(&monitor));
        }
        g_variant_builder_add(&entry, "{sv}", "maximized",
                              g_variant_new_boolean(meta_window_is_maximized(window)));
        g_variant_builder_add(&entry, "{sv}", "fullscreen",
                              g_variant_new_boolean(meta_window_is_fullscreen(window)));
        g_variant_builder_add(&entry, "{sv}", "geometry", g_variant_builder_end(&geometry));
        g_variant_builder_add_value(&entries, g_variant_builder_end(&entry));
    }
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "windows", g_variant_builder_end(&entries));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* gnoblin_native_window_match(MetaDisplay* display, GVariant* arguments,
                                             GError** error) {
    const char* selector = "active";
    g_autoptr(GVariant) selector_value = NULL;
    g_autoptr(GVariant) snapshot = NULL;
    g_autoptr(GVariant) windows = NULL;
    g_autoptr(GVariant) selected = NULL;

    if (arguments && !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.match arguments must be an object");
        return NULL;
    }

    if (arguments) {
        selector_value = g_variant_lookup_value(arguments, "window", NULL);
        if (selector_value && !g_variant_is_of_type(selector_value, G_VARIANT_TYPE_STRING)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window selector must be a string");
            return NULL;
        }
        if (selector_value)
            selector = g_variant_get_string(selector_value, NULL);
    }

    snapshot = gnoblin_native_window_list(display, NULL, error);
    if (!snapshot)
        return NULL;
    windows = g_variant_lookup_value(snapshot, "windows", G_VARIANT_TYPE("aa{sv}"));
    for (gsize i = 0; windows && i < g_variant_n_children(windows); i++) {
        g_autoptr(GVariant) candidate = g_variant_get_child_value(windows, i);
        const char* id = NULL;
        gboolean focused = FALSE;

        g_variant_lookup(candidate, "id", "&s", &id);
        g_variant_lookup(candidate, "focused", "b", &focused);
        if ((g_str_equal(selector, "active") && focused) ||
            (!g_str_equal(selector, "active") && id && g_str_equal(selector, id))) {
            selected = g_steal_pointer(&candidate);
            break;
        }
    }

    if (!selected) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "window not found");
        return NULL;
    }

    const char* id = "";
    const char* title = "";
    const char* desktop_app_id = "";
    const char* gtk_app_id = "";
    const char* wm_class = "";
    const char* rule_app_id = "";
    gboolean focused = FALSE;
    g_variant_lookup(selected, "id", "&s", &id);
    g_variant_lookup(selected, "title", "&s", &title);
    g_variant_lookup(selected, "appId", "&s", &desktop_app_id);
    g_variant_lookup(selected, "gtkAppId", "&s", &gtk_app_id);
    g_variant_lookup(selected, "wmClass", "&s", &wm_class);
    g_variant_lookup(selected, "ruleAppId", "&s", &rule_app_id);
    g_variant_lookup(selected, "focused", "b", &focused);

    GVariantBuilder identity;
    GVariantBuilder match;
    GVariantBuilder result;
    g_variant_builder_init(&identity, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&identity, "{sv}", "desktop_app_id",
                          g_variant_new_string(desktop_app_id));
    g_variant_builder_add(&identity, "{sv}", "gtk_app_id", g_variant_new_string(gtk_app_id));
    g_variant_builder_add(&identity, "{sv}", "wm_class", g_variant_new_string(wm_class));
    g_variant_builder_add(&identity, "{sv}", "rule_app_id", g_variant_new_string(rule_app_id));
    g_variant_builder_init(&match, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&match, "{sv}", "type", g_variant_new_string("window"));
    g_variant_builder_add(&match, "{sv}", "title", g_variant_new_string(title));
    g_variant_builder_add(&match, "{sv}", "focused", g_variant_new_boolean(focused));
    if (*rule_app_id)
        g_variant_builder_add(&match, "{sv}", "app_id", g_variant_new_string(rule_app_id));
    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&result, "{sv}", "identity", g_variant_builder_end(&identity));
    g_variant_builder_add(&result, "{sv}", "match", g_variant_builder_end(&match));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

static GVariant* gnoblin_native_window_action(MetaDisplay* display, GVariant* arguments,
                                              GError** error) {
    static const char* const supported[] = {
        "above",    "unabove",    "stick",      "unstick",      "close",   "minimize", "restore",
        "maximize", "unmaximize", "fullscreen", "unfullscreen", "monitor", NULL};
    const char* action = NULL;
    const char* selector = "active";
    MetaWindow* window = NULL;
    g_autoptr(GList) windows = NULL;
    g_autoptr(GVariant) window_value = NULL;
    g_autofree char* window_id = NULL;
    guint32 stable_sequence;
    GVariantBuilder result;

    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_lookup(arguments, "action", "&s", &action) ||
        !g_strv_contains(supported, action))
        return NULL;

    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    if (meta_backend_get_gnoblin_session_lock_active(backend)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window management is unavailable while the session is locked");
        return NULL;
    }

    window_value = g_variant_lookup_value(arguments, "window", NULL);
    if (window_value) {
        if (!g_variant_is_of_type(window_value, G_VARIANT_TYPE_STRING))
            goto missing_window;
        selector = g_variant_get_string(window_value, NULL);
    }
    if (g_str_equal(selector, "active"))
        window = meta_display_get_focus_window(display);
    else {
        char* end = NULL;
        guint64 sequence = g_ascii_strtoull(selector, &end, 10);
        if (end == selector || *end || sequence > G_MAXUINT32)
            goto missing_window;
        windows = meta_display_list_all_windows(display);
        for (GList* item = windows; item; item = item->next) {
            MetaWindow* candidate = item->data;
            if (meta_window_get_stable_sequence(candidate) == sequence) {
                window = candidate;
                break;
            }
        }
    }

    if (!window || meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
        goto missing_window;

    stable_sequence = meta_window_get_stable_sequence(window);
    if (g_str_equal(action, "monitor")) {
        gint64 monitor;

        if (!g_variant_lookup(arguments, "monitor", "x", &monitor) || monitor < 0 ||
            monitor >= meta_display_get_n_monitors(display)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "monitor not found");
            return NULL;
        }
        meta_window_move_to_monitor(window, (int)monitor);
    } else if (g_str_equal(action, "close")) {
        if (!meta_window_can_close(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be closed");
            return NULL;
        }
        meta_window_delete(window, meta_display_get_current_time(display));
    } else if (g_str_equal(action, "minimize")) {
        if (!meta_window_can_minimize(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be minimized");
            return NULL;
        }
        meta_window_minimize(window);
    } else if (g_str_equal(action, "restore"))
        meta_window_unminimize(window);
    else if (g_str_equal(action, "maximize")) {
        if (!meta_window_can_maximize(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be maximized");
            return NULL;
        }
        meta_window_maximize(window);
    } else if (g_str_equal(action, "unmaximize"))
        meta_window_unmaximize(window);
    else if (g_str_equal(action, "fullscreen"))
        meta_window_make_fullscreen(window);
    else if (g_str_equal(action, "unfullscreen"))
        meta_window_unmake_fullscreen(window);
    else if (g_str_equal(action, "above"))
        meta_window_make_above(window);
    else if (g_str_equal(action, "unabove"))
        meta_window_unmake_above(window);
    else if (g_str_equal(action, "stick"))
        meta_window_stick(window);
    else
        meta_window_unstick(window);

    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    window_id = g_strdup_printf("%u", stable_sequence);
    g_variant_builder_add(&result, "{sv}", "ok", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&result, "{sv}", "pending", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&result, "{sv}", "window", g_variant_new_string(window_id));
    g_variant_builder_add(&result, "{sv}", "action", g_variant_new_string(action));
    return g_variant_ref_sink(g_variant_builder_end(&result));

missing_window:
    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                        "window no longer available; list windows first");
    return NULL;
}

static gboolean gnoblin_native_variant_has_only_keys(GVariant* arguments,
                                                     const char* const* allowed, GError** error) {
    GVariantIter iter;
    const char* key;
    GVariant* value;

    if (!arguments || !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window operation arguments must be a dictionary");
        return FALSE;
    }

    g_variant_iter_init(&iter, arguments);
    while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
        gboolean known = g_strv_contains(allowed, key);
        g_variant_unref(value);
        if (!known) {
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                        "unknown window operation argument '%s'", key);
            return FALSE;
        }
    }
    return TRUE;
}

static MetaWindow* gnoblin_native_resolve_window(MetaDisplay* display, GVariant* arguments,
                                                 GError** error) {
    const char* wanted_id = NULL;
    g_autoptr(GList) windows = NULL;

    if (!g_variant_lookup(arguments, "id", "&s", &wanted_id) || !*wanted_id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window operation requires a stable string ID");
        return NULL;
    }

    windows = meta_display_list_all_windows(display);
    for (GList* item = windows; item; item = item->next) {
        MetaWindow* window = item->data;
        g_autofree char* id = NULL;

        if (meta_window_is_skip_taskbar(window) || meta_window_is_override_redirect(window))
            continue;

        id = g_strdup_printf("%u", meta_window_get_stable_sequence(window));
        if (g_str_equal(id, wanted_id))
            return window;
    }

    g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                        "window ID not found; list windows first");
    return NULL;
}

static GVariant* gnoblin_native_typed_window_operation(MetaDisplay* display, const char* method,
                                                       GVariant* arguments, GError** error) {
    static const char* const id_keys[] = {"id", NULL};
    static const char* const enabled_keys[] = {"id", "enabled", NULL};
    static const char* const move_keys[] = {"id", "x", "y", NULL};
    static const char* const resize_keys[] = {"id", "width", "height", NULL};
    static const char* const workspace_keys[] = {"id", "workspace", "follow", NULL};
    static const char* const workspace_selector_keys[] = {"id", "number", NULL};
    static const char* const monitor_keys[] = {"id", "monitor", NULL};
    static const char* const focus_keys[] = {"id", NULL};
    const char* const* allowed = NULL;
    MetaBackend* backend;
    MetaWindow* window;
    const char* id = NULL;
    gboolean enabled = FALSE;
    int monitor_index = -1;
    guint32 stable_sequence;
    GVariantBuilder result;
    g_autofree char* stable_id = NULL;

    if (g_str_equal(method, "window.focus")) {
        const char* focus_id = NULL;
        if (!gnoblin_native_variant_has_only_keys(arguments, focus_keys, error))
            return NULL;
        if (!g_variant_lookup(arguments, "id", "&s", &focus_id) || !*focus_id) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.focus requires a stable string ID");
            return NULL;
        }
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.focus requires a verified one-use activation context");
        return NULL;
    }
    if (g_str_equal(method, "window.close") || g_str_equal(method, "window.kill") ||
        g_str_equal(method, "window.raise") || g_str_equal(method, "window.lower") ||
        g_str_equal(method, "window.minimize") || g_str_equal(method, "window.toggle_minimize") ||
        g_str_equal(method, "window.unminimize") || g_str_equal(method, "window.restore"))
        allowed = id_keys;
    else if (g_str_equal(method, "window.set_maximized") ||
             g_str_equal(method, "window.set_fullscreen") ||
             g_str_equal(method, "window.set_above") || g_str_equal(method, "window.set_sticky"))
        allowed = enabled_keys;
    else if (g_str_equal(method, "window.move"))
        allowed = move_keys;
    else if (g_str_equal(method, "window.resize"))
        allowed = resize_keys;
    else if (g_str_equal(method, "window.move_to_workspace"))
        allowed = workspace_keys;
    else if (g_str_equal(method, "window.move_to_monitor"))
        allowed = monitor_keys;
    else
        return NULL;

    if (!gnoblin_native_variant_has_only_keys(arguments, allowed, error))
        return NULL;
    if (!g_variant_lookup(arguments, "id", "&s", &id) || !*id) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window operation requires a stable string ID");
        return NULL;
    }
    if (allowed == enabled_keys && !g_variant_lookup(arguments, "enabled", "b", &enabled)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window state operation requires a boolean 'enabled'");
        return NULL;
    }
    if (allowed == move_keys) {
        gint64 x, y;
        if (!g_variant_lookup(arguments, "x", "x", &x) ||
            !g_variant_lookup(arguments, "y", "x", &y) || x < -100000 || x > 100000 ||
            y < -100000 || y > 100000) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.move requires x and y from -100000 to 100000");
            return NULL;
        }
    }
    if (allowed == resize_keys) {
        gint64 width, height;
        if (!g_variant_lookup(arguments, "width", "x", &width) ||
            !g_variant_lookup(arguments, "height", "x", &height) || width < 1 || width > 32768 ||
            height < 1 || height > 32768) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "window.resize requires width and height from 1 to 32768");
            return NULL;
        }
    }
    if (allowed == workspace_keys) {
        g_autoptr(GVariant) selector =
            g_variant_lookup_value(arguments, "workspace", G_VARIANT_TYPE_VARDICT);
        g_autoptr(GVariant) follow_value = g_variant_lookup_value(arguments, "follow", NULL);
        if (!selector ||
            !gnoblin_native_variant_has_only_keys(selector, workspace_selector_keys, error)) {
            if (error && !*error)
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                    "window.move_to_workspace requires a workspace selector");
            return NULL;
        }
        g_autoptr(GVariant) selector_id = g_variant_lookup_value(selector, "id", NULL);
        g_autoptr(GVariant) selector_number = g_variant_lookup_value(selector, "number", NULL);
        gboolean valid_selector =
            (selector_id && g_variant_is_of_type(selector_id, G_VARIANT_TYPE_STRING) &&
             !selector_number) ||
            (selector_number && g_variant_is_of_type(selector_number, G_VARIANT_TYPE_INT64) &&
             !selector_id);
        if (!valid_selector ||
            (follow_value && !g_variant_is_of_type(follow_value, G_VARIANT_TYPE_BOOLEAN))) {
            g_set_error_literal(
                error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                "workspace must contain exactly an ID or number; follow must be boolean");
            return NULL;
        }
    }
    if (allowed == monitor_keys) {
        monitor_index = gnoblin_native_monitor_selector_resolve(display, arguments, error);
        if (monitor_index < 0)
            return NULL;
    }

    backend = meta_context_get_backend(meta_display_get_context(display));
    if (meta_backend_get_gnoblin_session_lock_active(backend)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window management is unavailable while the session is locked");
        return NULL;
    }

    window = gnoblin_native_resolve_window(display, arguments, error);
    if (!window)
        return NULL;
    stable_sequence = meta_window_get_stable_sequence(window);
    stable_id = g_strdup_printf("%u", stable_sequence);

    if (g_str_equal(method, "window.close")) {
        if (!meta_window_can_close(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be closed");
            return NULL;
        }
        {
            /* A close requested without an input event has no server time. Mutter refuses to ping a
             * window with serial 0, so the liveness check that finds a frozen client never ran. */
            guint32 timestamp = meta_display_get_current_time(display);
            if (timestamp == 0)
                timestamp = (guint32)(g_get_monotonic_time() / 1000) | 1;
            meta_window_delete(window, timestamp);
        }
    } else if (g_str_equal(method, "window.raise"))
        meta_window_raise(window);
    else if (g_str_equal(method, "window.lower"))
        meta_window_lower(window);
    else if (g_str_equal(method, "window.kill")) {
        if (meta_window_get_pid(window) <= 0) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                "window has no client process to kill");
            return NULL;
        }
        meta_window_kill(window);
    } else if (g_str_equal(method, "window.minimize")) {
        if (!meta_window_can_minimize(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be minimized");
            return NULL;
        }
        meta_window_minimize(window);
    } else if (g_str_equal(method, "window.unminimize"))
        meta_window_unminimize(window);
    else if (g_str_equal(method, "window.toggle_minimize")) {
        gboolean minimized = FALSE;
        g_object_get(window, "minimized", &minimized, NULL);
        if (minimized)
            meta_window_unminimize(window);
        else if (meta_window_can_minimize(window))
            meta_window_minimize(window);
        else {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be minimized");
            return NULL;
        }
    } else if (g_str_equal(method, "window.restore")) {
        gboolean maximized = meta_window_is_maximized(window);
        gboolean minimized = FALSE;
        g_object_get(window, "minimized", &minimized, NULL);
        if (minimized)
            meta_window_unminimize(window);
        if (maximized)
            meta_window_unmaximize(window);
    } else if (g_str_equal(method, "window.set_maximized")) {
        if (enabled && !meta_window_is_maximized(window)) {
            if (!meta_window_can_maximize(window)) {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                    "window cannot be maximized");
                return NULL;
            }
            meta_window_maximize(window);
        } else if (!enabled && meta_window_is_maximized(window))
            meta_window_unmaximize(window);
    } else if (g_str_equal(method, "window.set_fullscreen")) {
        if (enabled && !meta_window_is_fullscreen(window))
            meta_window_make_fullscreen(window);
        else if (!enabled && meta_window_is_fullscreen(window))
            meta_window_unmake_fullscreen(window);
    } else if (g_str_equal(method, "window.set_above")) {
        if (enabled && !meta_window_is_above(window))
            meta_window_make_above(window);
        else if (!enabled && meta_window_is_above(window))
            meta_window_unmake_above(window);
    } else if (g_str_equal(method, "window.set_sticky")) {
        gboolean sticky = meta_window_is_always_on_all_workspaces(window);
        if (enabled && !sticky)
            meta_window_stick(window);
        else if (!enabled && sticky)
            meta_window_unstick(window);
    } else if (g_str_equal(method, "window.move")) {
        gint64 x, y;
        g_variant_lookup(arguments, "x", "x", &x);
        g_variant_lookup(arguments, "y", "x", &y);
        if (!meta_window_allows_move(window) || meta_window_is_fullscreen(window) ||
            meta_window_get_maximize_flags(window) != META_MAXIMIZE_NONE) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                                "window cannot move; unmaximize or leave fullscreen first");
            return NULL;
        }
        meta_window_move_frame(window, TRUE, (int)x, (int)y);
    } else if (g_str_equal(method, "window.resize")) {
        gint64 width, height;
        MtkRectangle frame;
        g_variant_lookup(arguments, "width", "x", &width);
        g_variant_lookup(arguments, "height", "x", &height);
        if (!meta_window_allows_resize(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be resized");
            return NULL;
        }
        meta_window_get_frame_rect(window, &frame);
        if (frame.width != width || frame.height != height) {
            MtkRectangle old_buffer_rect;

            meta_window_get_buffer_rect(window, &old_buffer_rect);
            meta_compositor_size_change_window(display->compositor, window, META_SIZE_CHANGE_RESIZE,
                                               &frame, &old_buffer_rect);
            meta_window_move_resize_frame(window, TRUE, frame.x, frame.y, (int)width, (int)height);
        }
    } else if (g_str_equal(method, "window.move_to_workspace")) {
        if (!meta_window_allows_move(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be moved");
            return NULL;
        }
        MetaWorkspaceManager* manager = meta_display_get_workspace_manager(display);
        g_autoptr(GVariant) selector =
            g_variant_lookup_value(arguments, "workspace", G_VARIANT_TYPE_VARDICT);
        gboolean follow = FALSE;
        MetaWorkspace* workspace;
        g_variant_lookup(arguments, "follow", "b", &follow);
        workspace = gnoblin_native_workspace_resolve(manager, selector, error);
        if (!workspace)
            return NULL;
        meta_window_change_workspace(window, workspace);
        if (follow)
            meta_workspace_activate(workspace, meta_display_get_current_time(display));
    } else if (g_str_equal(method, "window.move_to_monitor")) {
        if (!meta_window_allows_move(window)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "window cannot be moved");
            return NULL;
        }
        meta_window_move_to_monitor(window, monitor_index);
    }

    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "id", g_variant_new_string(stable_id));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}

GVariant* meta_gnoblin_dispatch_native_api(MetaDisplay* display, const char* method,
                                           GVariant* arguments, GError** error) {
    static const char* const typed_window_methods[] = {"window.close",
                                                       "window.kill",
                                                       "window.raise",
                                                       "window.lower",
                                                       "window.minimize",
                                                       "window.toggle_minimize",
                                                       "window.unminimize",
                                                       "window.restore",
                                                       "window.set_maximized",
                                                       "window.set_fullscreen",
                                                       "window.set_above",
                                                       "window.set_sticky",
                                                       "window.move",
                                                       "window.resize",
                                                       "window.move_to_workspace",
                                                       "window.move_to_monitor",
                                                       "window.focus",
                                                       NULL};
    GVariantBuilder result;
    GVariantBuilder monitors;
    int n_monitors;
    int primary;

    g_return_val_if_fail(META_IS_DISPLAY(display), NULL);
    if (!method) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "native API method is required");
        return NULL;
    }

    if (g_str_has_prefix(method, "animation."))
        return meta_gnoblin_animation_dispatch(display, method, arguments, error);
    if (g_strcmp0(method, "window.action") == 0)
        return gnoblin_native_window_action(display, arguments, error);
    if (g_strcmp0(method, "window.list") == 0)
        return gnoblin_native_window_list(display, arguments, error);
    if (g_strv_contains(typed_window_methods, method))
        return gnoblin_native_typed_window_operation(display, method, arguments, error);
    if (g_strcmp0(method, "window.match") == 0)
        return gnoblin_native_window_match(display, arguments, error);
    if (g_strcmp0(method, "session.lock") == 0)
        return gnoblin_native_control_request_session_lock(display, arguments, error);
    if (g_str_has_prefix(method, "launch."))
        return gnoblin_native_control_dispatch_launch(display, method, arguments, error);
    if (g_str_has_prefix(method, "window.")) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                    "unsupported native window method '%s'", method);
        return NULL;
    }
    if (g_strcmp0(method, "layer.list") == 0) {
        static const char* const filter_keys[] = {"monitor_id", "namespace", "layer", NULL};
        GVariantBuilder layers;
        const char* monitor_filter = NULL;
        const char* namespace_filter = NULL;
        const char* layer_filter = NULL;
        g_autoptr(GList) windows = NULL;

        if (arguments && !g_variant_is_of_type(arguments, G_VARIANT_TYPE_VARDICT)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "layer.list arguments must be a dictionary");
            return NULL;
        }
        if (arguments) {
            GVariantIter iter;
            const char* key;
            GVariant* value;

            g_variant_iter_init(&iter, arguments);
            while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
                gboolean known = g_strv_contains(filter_keys, key);
                gboolean string_value = g_variant_is_of_type(value, G_VARIANT_TYPE_STRING);

                g_variant_unref(value);
                if (!known) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "unknown layer.list filter '%s'", key);
                    return NULL;
                }
                if (!string_value) {
                    g_set_error(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "layer.list filter '%s' must be a string", key);
                    return NULL;
                }
            }

            g_variant_lookup(arguments, "monitor_id", "&s", &monitor_filter);
            g_variant_lookup(arguments, "namespace", "&s", &namespace_filter);
            g_variant_lookup(arguments, "layer", "&s", &layer_filter);
        }

        g_variant_builder_init(&layers, G_VARIANT_TYPE("aa{sv}"));
        windows = meta_display_list_all_windows(display);
        for (GList* l = windows; l; l = l->next) {
            MetaWindow* window = l->data;
            int monitor_index = meta_window_get_monitor(window);
            g_autofree char* monitor_id =
                gnoblin_native_monitor_id_for_index(display, monitor_index);
            g_autoptr(GVariant) record =
                monitor_id ? meta_wayland_layer_shell_get_snapshot_record(window, monitor_id)
                           : NULL;
            const char* record_value = NULL;

            if (!record)
                continue;
            if (monitor_filter && g_strcmp0(monitor_id, monitor_filter) != 0)
                continue;
            if (namespace_filter && (!g_variant_lookup(record, "namespace", "&s", &record_value) ||
                                     g_strcmp0(record_value, namespace_filter) != 0))
                continue;
            if (layer_filter && (!g_variant_lookup(record, "layer", "&s", &record_value) ||
                                 g_strcmp0(record_value, layer_filter) != 0))
                continue;

            g_variant_builder_add_value(&layers, g_steal_pointer(&record));
        }

        g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&result, "{sv}", "layers", g_variant_builder_end(&layers));
        return g_variant_ref_sink(g_variant_builder_end(&result));
    }
    if (g_strcmp0(method, "workspace.list") == 0 || g_strcmp0(method, "workspace.create") == 0 ||
        g_strcmp0(method, "workspace.rename") == 0 || g_strcmp0(method, "workspace.remove") == 0 ||
        g_strcmp0(method, "workspace.switch") == 0 || g_strcmp0(method, "workspace.next") == 0 ||
        g_strcmp0(method, "workspace.previous") == 0 ||
        g_strcmp0(method, "workspace.move_active") == 0 ||
        g_strcmp0(method, "workspace.move_window") == 0)
        return gnoblin_native_workspace_api(display, method, arguments, error);
    if (g_strcmp0(method, "monitor.list") != 0)
        return NULL;

    g_return_val_if_fail(META_IS_DISPLAY(display), NULL);

    n_monitors = meta_display_get_n_monitors(display);
    primary = meta_display_get_primary_monitor(display);
    g_variant_builder_init(&monitors, G_VARIANT_TYPE("aa{sv}"));
    g_autoptr(GHashTable) monitor_ids =
        g_hash_table_new_full(g_str_hash, g_str_equal, g_free, NULL);
    for (int index = 0; index < n_monitors; index++) {
        GVariantBuilder monitor;
        MtkRectangle geometry;
        MetaLogicalMonitor* logical_monitor =
            gnoblin_native_logical_monitor_for_index(display, index);
        MetaMonitor* physical_monitor = gnoblin_native_monitor_for_index(display, index);
        g_autofree char* monitor_id = gnoblin_native_monitor_id_for_index(display, index);

        if (!monitor_id || g_hash_table_contains(monitor_ids, monitor_id)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                "a listed monitor has no unique active connector ID");
            return NULL;
        }
        g_hash_table_add(monitor_ids, g_strdup(monitor_id));

        meta_display_get_monitor_geometry(display, index, &geometry);
        g_variant_builder_init(&monitor, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&monitor, "{sv}", "id", g_variant_new_string(monitor_id));
        g_variant_builder_add(&monitor, "{sv}", "index", g_variant_new_int32(index));
        g_variant_builder_add(&monitor, "{sv}", "x", g_variant_new_int32(geometry.x));
        g_variant_builder_add(&monitor, "{sv}", "y", g_variant_new_int32(geometry.y));
        g_variant_builder_add(&monitor, "{sv}", "width", g_variant_new_int32(geometry.width));
        g_variant_builder_add(&monitor, "{sv}", "height", g_variant_new_int32(geometry.height));
        g_variant_builder_add(&monitor, "{sv}", "primary", g_variant_new_boolean(index == primary));
        g_variant_builder_add(&monitor, "{sv}", "scale",
                              g_variant_new_double(meta_display_get_monitor_scale(display, index)));
        g_variant_builder_add(&monitor, "{sv}", "enabled", g_variant_new_boolean(TRUE));
        if (physical_monitor) {
            const char* name = meta_monitor_get_display_name(physical_monitor);
            const char* vendor = meta_monitor_get_vendor(physical_monitor);
            const char* product = meta_monitor_get_product(physical_monitor);
            const char* serial = meta_monitor_get_serial(physical_monitor);
            MetaMonitorMode* current_mode = meta_monitor_get_current_mode(physical_monitor);

            if (name && *name)
                g_variant_builder_add(&monitor, "{sv}", "name", g_variant_new_string(name));
            if (vendor && *vendor)
                g_variant_builder_add(&monitor, "{sv}", "make", g_variant_new_string(vendor));
            if (product && *product)
                g_variant_builder_add(&monitor, "{sv}", "model", g_variant_new_string(product));
            if (serial && *serial)
                g_variant_builder_add(&monitor, "{sv}", "serial", g_variant_new_string(serial));
            if (current_mode)
                g_variant_builder_add(
                    &monitor, "{sv}", "refresh_rate",
                    g_variant_new_double(meta_monitor_mode_get_refresh_rate(current_mode)));
        }
        if (logical_monitor)
            g_variant_builder_add(&monitor, "{sv}", "transform",
                                  g_variant_new_string(gnoblin_native_monitor_transform_name(
                                      meta_logical_monitor_get_transform(logical_monitor))));
        g_variant_builder_add_value(&monitors, g_variant_builder_end(&monitor));
    }

    g_variant_builder_init(&result, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&result, "{sv}", "monitors", g_variant_builder_end(&monitors));
    return g_variant_ref_sink(g_variant_builder_end(&result));
}
