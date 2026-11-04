/* window.present: focus a window that belongs to a process the runtime started. */
#include "config.h"

#include "core/gnoblin-control-internal.h"
#include "core/gnoblin-native-control-private.h"

/* window.present: focus a window that belongs to a process the runtime started, without a
 * key-press context. Attribution is the authority: an application cannot join a session it did
 * not start in, and its pid comes from the display server. To keep a window from taking the
 * keystrokes the user is typing elsewhere, focus waits until no key press or click reached a
 * window for PRESENT_QUIET_MS, and until the window is shown. */
#define PRESENT_QUIET_MS 250
#define PRESENT_POLL_MS 50
#define PRESENT_LIMIT_US (5 * G_USEC_PER_SEC)

typedef struct {
    GnoblinNativeControl* control;
    gint64 operation_id;
    MetaWindow* window; /* Weak; cleared when the window goes away. */
    char* id;
    gint64 deadline_us;
    guint source_id;
    gboolean activated;
} PendingPresent;

static void present_free(gpointer data) {
    PendingPresent* present = data;
    g_clear_handle_id(&present->source_id, g_source_remove);
    if (present->window)
        g_object_remove_weak_pointer(G_OBJECT(present->window), (gpointer*)&present->window);
    g_free(present->id);
    g_free(present);
}

static void present_finish(PendingPresent* present, int code, const char* message) {
    GnoblinNativeControl* control = present->control;
    present->source_id = 0;
    g_autoptr(JsonNode) result = NULL;
    g_autoptr(GError) error = NULL;
    if (message)
        g_set_error_literal(&error, G_IO_ERROR, code, message);
    else {
        JsonObject* object = json_object_new();
        json_object_set_string_member(object, "id", present->id);
        json_object_set_boolean_member(object, "focused", TRUE);
        result = json_node_new(JSON_NODE_OBJECT);
        json_node_take_object(result, object);
    }
    gint64 operation_id = present->operation_id;
    g_hash_table_remove(control->presents, &operation_id);
    if (!control->stopping)
        gnoblin_control_dispatch_operation_completion_full(
            control, operation_id, "window.present", result != NULL, result,
            gnoblin_control_operation_error_code(error), error ? error->message : NULL, FALSE);
}

static gboolean present_step(gpointer user_data) {
    PendingPresent* present = user_data;
    GnoblinNativeControl* control = present->control;
    MetaWindow* window = present->window;
    if (!window) {
        present_finish(present, G_IO_ERROR_NOT_FOUND, "the window closed before it was focused");
        return G_SOURCE_REMOVE;
    }
    if (control->wayland_compositor &&
        meta_wayland_session_lock_is_active(control->wayland_compositor)) {
        present_finish(present, G_IO_ERROR_PERMISSION_DENIED,
                       "a window cannot be focused while the session is locked");
        return G_SOURCE_REMOVE;
    }
    MetaDisplay* display = control->display;
    if (present->activated) {
        if (meta_display_get_focus_window(display) == window)
            present_finish(present, 0, NULL);
        else
            present_finish(present, G_IO_ERROR_PERMISSION_DENIED,
                           "the compositor did not give the window focus");
        return G_SOURCE_REMOVE;
    }
    guint32 now = meta_display_get_current_time_roundtrip(display);
    guint32 quiet = now - meta_display_get_last_user_time(display);
    if (meta_window_is_hidden(window) || quiet < PRESENT_QUIET_MS) {
        if (g_get_monotonic_time() < present->deadline_us)
            return G_SOURCE_CONTINUE;
        present_finish(present, G_IO_ERROR_BUSY,
                       meta_window_is_hidden(window)
                           ? "the window was not shown in time"
                           : "the user kept typing or clicking, so the window was not focused");
        return G_SOURCE_REMOVE;
    }
    meta_window_activate_with_workspace(window, now, NULL);
    present->activated = TRUE;
    /* Wayland focus is immediate; an X11 client confirms it a little later. */
    if (meta_display_get_focus_window(display) == window) {
        present_finish(present, 0, NULL);
        return G_SOURCE_REMOVE;
    }
    /* Returning G_SOURCE_REMOVE ends this poll; the check below replaces it. */
    present->source_id = g_timeout_add(100, present_step, present);
    return G_SOURCE_REMOVE;
}

gboolean gnoblin_control_present_window(GnoblinNativeControl* control, gint64 operation_id,
                                        GVariant* arguments, GError** error) {
    const char* wanted_id = NULL;
    if (!g_variant_lookup(arguments, "id", "&s", &wanted_id) || !*wanted_id ||
        g_variant_n_children(arguments) != 1) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "window.present requires a stable string ID");
        return FALSE;
    }
    g_autoptr(GList) windows = meta_display_list_all_windows(control->display);
    MetaWindow* target = NULL;
    for (GList* item = windows; item && !target; item = item->next) {
        g_autofree char* id = native_window_id(item->data);
        if (g_str_equal(id, wanted_id) && !meta_window_is_override_redirect(item->data))
            target = item->data;
    }
    if (!target) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                            "window ID not found; list windows first");
        return FALSE;
    }
    if (!g_object_get_data(G_OBJECT(target), "gnoblin-process") &&
        !gnoblin_control_process_for_pid(control, meta_window_get_pid(target))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PERMISSION_DENIED,
                            "window.present needs a window of a process started with "
                            "gnoblin.process.spawn; use Window:focus with a Grant");
        return FALSE;
    }
    PendingPresent* present = g_new0(PendingPresent, 1);
    present->control = control;
    present->operation_id = operation_id;
    present->window = target;
    g_object_add_weak_pointer(G_OBJECT(target), (gpointer*)&present->window);
    present->id = g_strdup(wanted_id);
    present->deadline_us = g_get_monotonic_time() + PRESENT_LIMIT_US;
    present->source_id = g_timeout_add(PRESENT_POLL_MS, present_step, present);
    g_hash_table_insert(control->presents, &present->operation_id, present);
    return TRUE;
}

void gnoblin_control_present_init(GnoblinNativeControl* control) {
    control->presents = g_hash_table_new_full(g_int64_hash, g_int64_equal, NULL, present_free);
}

/* Compositor shutdown: drop waiting presents without completing them. */
void gnoblin_control_present_clear(GnoblinNativeControl* control) {
    if (control && control->presents)
        g_hash_table_remove_all(control->presents);
}
