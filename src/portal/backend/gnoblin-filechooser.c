// SPDX-License-Identifier: LGPL-2.1-or-later
// Gnoblin's GTK4 file chooser for the portal implementation interface.

#include "config.h"

#include <gtk/gtk.h>
#include <gxdp.h>
#include <string.h>

#include "filechooser.h"
#include "request.h"
#include "utils.h"

typedef struct {
    GtkFileFilter* filter;
    GVariant* portal_value;
} PortalFilter;

typedef struct {
    XdpImplFileChooser* impl;
    GDBusMethodInvocation* invocation;
    Request* request;
    GtkWidget* dialog;
    GtkWindow* parent;
    GxdpExternalWindow* external_parent;
    GPtrArray* filters;
    GPtrArray* choice_ids;
    GPtrArray* names;
    gboolean save_files;
    gboolean completed;
} FileDialog;

static void portal_filter_free(gpointer data) {
    PortalFilter* filter = data;
    g_clear_object(&filter->filter);
    g_clear_pointer(&filter->portal_value, g_variant_unref);
    g_free(filter);
}

static void file_dialog_free(FileDialog* handle) {
    g_signal_handlers_disconnect_by_data(handle->dialog, handle);
    gtk_window_destroy(GTK_WINDOW(handle->dialog));
    gtk_window_destroy(handle->parent);
    g_clear_object(&handle->dialog);
    g_clear_object(&handle->parent);
    g_clear_object(&handle->external_parent);
    g_clear_pointer(&handle->filters, g_ptr_array_unref);
    g_clear_pointer(&handle->choice_ids, g_ptr_array_unref);
    g_clear_pointer(&handle->names, g_ptr_array_unref);
    g_clear_object(&handle->invocation);
    g_clear_object(&handle->request);
    g_free(handle);
}

static gboolean add_selected_files(FileDialog* handle, GVariantBuilder* uris) {
    GtkFileChooser* chooser = GTK_FILE_CHOOSER(handle->dialog);
    g_autoptr(GListModel) files = gtk_file_chooser_get_files(chooser);
    guint count = g_list_model_get_n_items(files);

    if (count == 0)
        return FALSE;

    for (guint i = 0; i < count; i++) {
        g_autoptr(GFile) file = g_list_model_get_item(files, i);
        if (!g_file_is_native(file))
            return FALSE;
        if (handle->save_files) {
            if (g_file_query_file_type(file, G_FILE_QUERY_INFO_NONE, NULL) != G_FILE_TYPE_DIRECTORY)
                return FALSE;
            for (guint j = 0; j < handle->names->len; j++) {
                g_autoptr(GFile) child =
                    g_file_get_child(file, g_ptr_array_index(handle->names, j));
                g_autofree char* uri = g_file_get_uri(child);
                g_variant_builder_add(uris, "s", uri);
            }
            return TRUE;
        }
        if (g_str_equal(g_dbus_method_invocation_get_method_name(handle->invocation), "OpenFile") &&
            !g_file_query_exists(file, NULL))
            return FALSE;
        g_autofree char* uri = g_file_get_uri(file);
        g_variant_builder_add(uris, "s", uri);
    }
    return TRUE;
}

static void file_dialog_complete(FileDialog* handle, guint response) {
    if (handle->completed)
        return;
    handle->completed = TRUE;

    GVariantBuilder results;
    GVariantBuilder uris;
    GVariantBuilder choices;
    GtkFileChooser* chooser = GTK_FILE_CHOOSER(handle->dialog);
    const char* method = g_dbus_method_invocation_get_method_name(handle->invocation);

    g_variant_builder_init(&results, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&uris, G_VARIANT_TYPE_STRING_ARRAY);
    if (response == 0 && !add_selected_files(handle, &uris))
        response = 2;
    g_variant_builder_add(&results, "{sv}", "uris", g_variant_builder_end(&uris));

    if (response == 0) {
        g_variant_builder_init(&choices, G_VARIANT_TYPE("a(ss)"));
        for (guint i = 0; i < handle->choice_ids->len; i++) {
            const char* id = g_ptr_array_index(handle->choice_ids, i);
            const char* selected = gtk_file_chooser_get_choice(chooser, id);
            g_variant_builder_add(&choices, "(ss)", id, selected ? selected : "");
        }
        g_variant_builder_add(&results, "{sv}", "choices", g_variant_builder_end(&choices));

        GtkFileFilter* selected = gtk_file_chooser_get_filter(chooser);
        for (guint i = 0; selected && i < handle->filters->len; i++) {
            PortalFilter* filter = g_ptr_array_index(handle->filters, i);
            if (filter->filter == selected) {
                g_variant_builder_add(&results, "{sv}", "current_filter", filter->portal_value);
                break;
            }
        }
    }

    if (handle->request->exported)
        request_unexport(handle->request);
    GVariant* value = g_variant_builder_end(&results);
    if (g_str_equal(method, "OpenFile"))
        xdp_impl_file_chooser_complete_open_file(handle->impl, handle->invocation, response, value);
    else if (g_str_equal(method, "SaveFile"))
        xdp_impl_file_chooser_complete_save_file(handle->impl, handle->invocation, response, value);
    else
        xdp_impl_file_chooser_complete_save_files(handle->impl, handle->invocation, response,
                                                  value);
}

static void dialog_response(GtkDialog* dialog, int response, FileDialog* handle) {
    file_dialog_complete(handle, response == GTK_RESPONSE_ACCEPT ? 0 : 1);
    file_dialog_free(handle);
}

static gboolean request_close(XdpImplRequest* request, GDBusMethodInvocation* invocation,
                              FileDialog* handle) {
    file_dialog_complete(handle, 2);
    xdp_impl_request_complete_close(request, invocation);
    file_dialog_free(handle);
    return TRUE;
}

static void configure_choices(FileDialog* handle, GVariant* options) {
    GtkFileChooser* chooser = GTK_FILE_CHOOSER(handle->dialog);
    g_autoptr(GVariant) choices =
        g_variant_lookup_value(options, "choices", G_VARIANT_TYPE("a(ssa(ss)s)"));
    if (!choices)
        return;

    GVariantIter iter;
    GVariant* row;
    g_variant_iter_init(&iter, choices);
    while ((row = g_variant_iter_next_value(&iter))) {
        const char* id;
        const char* label;
        const char* selected;
        g_autoptr(GVariant) items = NULL;
        g_variant_get(row, "(&s&s@a(ss)&s)", &id, &label, &items, &selected);
        guint n = g_variant_n_children(items);
        g_auto(GStrv) ids = n ? g_new0(char*, n + 1) : NULL;
        g_auto(GStrv) labels = n ? g_new0(char*, n + 1) : NULL;
        for (guint i = 0; i < n; i++) {
            const char* option_id;
            const char* option_label;
            g_variant_get_child(items, i, "(&s&s)", &option_id, &option_label);
            ids[i] = g_strdup(option_id);
            labels[i] = g_strdup(option_label);
        }
        gtk_file_chooser_add_choice(chooser, id, label, (const char**)ids, (const char**)labels);
        if (*selected)
            gtk_file_chooser_set_choice(chooser, id, selected);
        g_ptr_array_add(handle->choice_ids, g_strdup(id));
        g_variant_unref(row);
    }
}

static void configure_filters(FileDialog* handle, GVariant* options) {
    GtkFileChooser* chooser = GTK_FILE_CHOOSER(handle->dialog);
    g_autoptr(GVariant) filters =
        g_variant_lookup_value(options, "filters", G_VARIANT_TYPE("a(sa(us))"));
    g_autoptr(GVariant) current =
        g_variant_lookup_value(options, "current_filter", G_VARIANT_TYPE("(sa(us))"));
    GtkFileFilter* initial = NULL;

    if (filters) {
        GVariantIter iter;
        GVariant* row;
        g_variant_iter_init(&iter, filters);
        while ((row = g_variant_iter_next_value(&iter))) {
            const char* name;
            g_autoptr(GVariant) rules = NULL;
            g_variant_get(row, "(&s@a(us))", &name, &rules);
            PortalFilter* entry = g_new0(PortalFilter, 1);
            entry->filter = gtk_file_filter_new();
            entry->portal_value = row;
            gtk_file_filter_set_name(entry->filter, name);
            GVariantIter rule_iter;
            guint kind;
            const char* pattern;
            g_variant_iter_init(&rule_iter, rules);
            while (g_variant_iter_next(&rule_iter, "(u&s)", &kind, &pattern)) {
                if (kind == 0)
                    gtk_file_filter_add_pattern(entry->filter, pattern);
                else if (kind == 1)
                    gtk_file_filter_add_mime_type(entry->filter, pattern);
            }
            gtk_file_chooser_add_filter(chooser, entry->filter);
            if (current && g_variant_equal(current, row))
                initial = entry->filter;
            g_ptr_array_add(handle->filters, entry);
        }
    }

    if (!initial && current && handle->filters->len == 0) {
        const char* name;
        g_autoptr(GVariant) rules = NULL;
        g_variant_get(current, "(&s@a(us))", &name, &rules);
        PortalFilter* entry = g_new0(PortalFilter, 1);
        entry->filter = gtk_file_filter_new();
        entry->portal_value = g_variant_ref(current);
        gtk_file_filter_set_name(entry->filter, name);
        GVariantIter iter;
        guint kind;
        const char* pattern;
        g_variant_iter_init(&iter, rules);
        while (g_variant_iter_next(&iter, "(u&s)", &kind, &pattern)) {
            if (kind == 0)
                gtk_file_filter_add_pattern(entry->filter, pattern);
            else if (kind == 1)
                gtk_file_filter_add_mime_type(entry->filter, pattern);
        }
        gtk_file_chooser_add_filter(chooser, entry->filter);
        initial = entry->filter;
        g_ptr_array_add(handle->filters, entry);
    }
    if (initial)
        gtk_file_chooser_set_filter(chooser, initial);
}

static void configure_paths(FileDialog* handle, GVariant* options) {
    GtkFileChooser* chooser = GTK_FILE_CHOOSER(handle->dialog);
    g_autoptr(GVariant) path =
        g_variant_lookup_value(options, "current_file", G_VARIANT_TYPE_BYTESTRING);
    if (path) {
        g_autoptr(GFile) file = g_file_new_for_path(g_variant_get_bytestring(path));
        gtk_file_chooser_set_file(chooser, file, NULL);
    } else {
        path = g_variant_lookup_value(options, "current_folder", G_VARIANT_TYPE_BYTESTRING);
        if (path) {
            g_autoptr(GFile) folder = g_file_new_for_path(g_variant_get_bytestring(path));
            gtk_file_chooser_set_current_folder(chooser, folder, NULL);
        }
    }
    const char* name;
    if (g_variant_lookup(options, "current_name", "&s", &name))
        gtk_file_chooser_set_current_name(chooser, name);
}

static gboolean handle_open(XdpImplFileChooser* impl, GDBusMethodInvocation* invocation,
                            const char* handle_path, const char* app_id, const char* parent_window,
                            const char* title, GVariant* options) {
    const char* method = g_dbus_method_invocation_get_method_name(invocation);
    gboolean multiple = FALSE;
    gboolean directory = FALSE;
    gboolean modal = TRUE;
    const char* accept = NULL;
    GtkFileChooserAction action;

    if (g_str_equal(method, "SaveFile"))
        action = GTK_FILE_CHOOSER_ACTION_SAVE;
    else if (g_str_equal(method, "SaveFiles"))
        action = GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER;
    else {
        g_variant_lookup(options, "multiple", "b", &multiple);
        g_variant_lookup(options, "directory", "b", &directory);
        action = directory ? GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER : GTK_FILE_CHOOSER_ACTION_OPEN;
    }
    g_variant_lookup(options, "modal", "b", &modal);
    if (!g_variant_lookup(options, "accept_label", "&s", &accept))
        accept = action == GTK_FILE_CHOOSER_ACTION_SAVE ? "_Save" : "_Open";

    FileDialog* handle = g_new0(FileDialog, 1);
    handle->impl = impl;
    handle->invocation = g_object_ref(invocation);
    handle->request =
        request_new(g_dbus_method_invocation_get_sender(invocation), app_id, handle_path);
    handle->save_files = g_str_equal(method, "SaveFiles");
    handle->filters = g_ptr_array_new_with_free_func(portal_filter_free);
    handle->choice_ids = g_ptr_array_new_with_free_func(g_free);
    handle->names = g_ptr_array_new_with_free_func(g_free);
    handle->parent = GTK_WINDOW(g_object_ref_sink(gtk_window_new()));
    if (parent_window && *parent_window)
        handle->external_parent = gxdp_external_window_new_from_handle(parent_window);
    handle->dialog = g_object_ref_sink(
        gtk_file_chooser_dialog_new(title, handle->parent, action, "_Cancel", GTK_RESPONSE_CANCEL,
                                    accept, GTK_RESPONSE_ACCEPT, NULL));
    gtk_window_set_modal(GTK_WINDOW(handle->dialog), modal);
    gtk_file_chooser_set_select_multiple(GTK_FILE_CHOOSER(handle->dialog), multiple);
    gtk_dialog_set_default_response(GTK_DIALOG(handle->dialog), GTK_RESPONSE_ACCEPT);

    if (handle->save_files) {
        gboolean valid_names = TRUE;
        g_autoptr(GVariant) names = g_variant_lookup_value(options, "files", G_VARIANT_TYPE("aay"));
        if (!names || g_variant_n_children(names) == 0)
            valid_names = FALSE;
        else
            for (guint i = 0; i < g_variant_n_children(names); i++) {
                g_autoptr(GVariant) item = g_variant_get_child_value(names, i);
                const char* name = g_variant_get_bytestring(item);
                if (!*name || strchr(name, '/') || g_str_equal(name, ".") ||
                    g_str_equal(name, "..")) {
                    valid_names = FALSE;
                    break;
                }
                g_ptr_array_add(handle->names, g_strdup(name));
            }
        if (!valid_names) {
            file_dialog_complete(handle, 2);
            file_dialog_free(handle);
            return TRUE;
        }
    }
    configure_choices(handle, options);
    configure_filters(handle, options);
    configure_paths(handle, options);

    g_signal_connect(handle->request, "handle-close", G_CALLBACK(request_close), handle);
    g_signal_connect(handle->dialog, "response", G_CALLBACK(dialog_response), handle);
    request_export(handle->request, g_dbus_method_invocation_get_connection(invocation));
    gtk_widget_realize(handle->dialog);
    if (handle->external_parent)
        gxdp_external_window_set_parent_of(handle->external_parent,
                                           gtk_native_get_surface(GTK_NATIVE(handle->dialog)));
    gtk_window_present(GTK_WINDOW(handle->dialog));
    return TRUE;
}

gboolean file_chooser_init(GDBusConnection* bus, GError** error) {
    GDBusInterfaceSkeleton* helper =
        G_DBUS_INTERFACE_SKELETON(xdp_impl_file_chooser_skeleton_new());
    g_signal_connect(helper, "handle-open-file", G_CALLBACK(handle_open), NULL);
    g_signal_connect(helper, "handle-save-file", G_CALLBACK(handle_open), NULL);
    g_signal_connect(helper, "handle-save-files", G_CALLBACK(handle_open), NULL);
    if (!g_dbus_interface_skeleton_export(helper, bus, DESKTOP_PORTAL_OBJECT_PATH, error)) {
        g_object_unref(helper);
        return FALSE;
    }
    g_debug("providing %s", g_dbus_interface_skeleton_get_info(helper)->name);
    return TRUE;
}
