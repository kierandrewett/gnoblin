/* -*- mode: C; c-file-style: "gnu"; indent-tabs-mode: nil; -*- */

/*
 * Copyright (C) 2026 Gnoblin contributors
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation; either version 2 of the License, or (at your
 * option) any later version.
 */

#include "config.h"

#include "clutter/clutter.h"
#include "meta/display.h"
#include "meta/meta-backend.h"
#include "meta/meta-background-actor.h"
#include "meta/meta-background-content.h"
#include "meta/meta-background-group.h"
#include "meta/meta-background.h"
#include "meta/compositor.h"
#include "meta/meta-context.h"
#include "meta/meta-monitor-manager.h"
#include "meta/meta-plugin.h"
#include "meta/keybindings.h"
#include "meta/meta-selection-source-memory.h"
#include "meta/meta-selection.h"
#include "meta/meta-wayland-compositor.h"
#include "core/gnoblin-animation.h"
#include "core/gnoblin-native-control.h"
#include "wayland/meta-wayland-session-lock.h"
#include "wayland/meta-gnoblin-frame-renderer.h"
#ifdef HAVE_IBUS
#include "backends/gnoblin-ibus-input-method.h"
#endif

#include "gnoblin-plugin-ui.h"
#include "gnoblin-plugin-input.h"

typedef struct _MetaGnoblinPlugin MetaGnoblinPlugin;
typedef struct _MetaGnoblinPluginClass MetaGnoblinPluginClass;

struct _MetaGnoblinPlugin {
    MetaPlugin parent;
    ClutterActor* background_group;
    GnoblinPluginUi* diagnostics_ui;
    GnoblinPluginInput input;
    gulong session_lock_callback_id;
    MetaWaylandCompositor* wayland_compositor;
};

struct _MetaGnoblinPluginClass {
    MetaPluginClass parent_class;
};

#define META_TYPE_GNOBLIN_PLUGIN (meta_gnoblin_plugin_get_type())
#define META_GNOBLIN_PLUGIN(obj)                                                                   \
    (G_TYPE_CHECK_INSTANCE_CAST((obj), META_TYPE_GNOBLIN_PLUGIN, MetaGnoblinPlugin))
#define META_GNOBLIN_PLUGIN_CLASS(klass)                                                           \
    (G_TYPE_CHECK_CLASS_CAST((klass), META_TYPE_GNOBLIN_PLUGIN, MetaGnoblinPluginClass))

G_DEFINE_TYPE(MetaGnoblinPlugin, meta_gnoblin_plugin, META_TYPE_PLUGIN);

static void gnoblin_plugin_update_backgrounds(MetaGnoblinPlugin* self) {
    MetaDisplay* display = meta_plugin_get_display(META_PLUGIN(self));
    clutter_actor_destroy_all_children(self->background_group);

    int n_monitors = meta_display_get_n_monitors(display);
    for (int i = 0; i < n_monitors; i++) {
        MetaBackgroundContent* content;
        ClutterActor* actor = meta_background_actor_new(display, i);
        MtkRectangle geometry;

        meta_display_get_monitor_geometry(display, i, &geometry);
        clutter_actor_set_position(actor, geometry.x, geometry.y);
        clutter_actor_set_size(actor, geometry.width, geometry.height);

        content = META_BACKGROUND_CONTENT(clutter_actor_get_content(actor));
        g_autoptr(MetaBackground) background = meta_background_new(display);
        meta_background_set_color(background, &COGL_COLOR_INIT(0, 0, 0, 255));
        meta_background_content_set_background(content, background);

        clutter_actor_add_child(self->background_group, actor);
    }
}

static void gnoblin_plugin_monitors_changed(MetaMonitorManager* manager, MetaGnoblinPlugin* self) {
    (void)manager;
    gnoblin_plugin_update_backgrounds(self);
}

static gboolean gnoblin_plugin_session_is_locked(MetaGnoblinPlugin* self) {
    MetaDisplay* display = meta_plugin_get_display(META_PLUGIN(self));
    MetaContext* context = meta_display_get_context(display);
    MetaWaylandCompositor* compositor = meta_context_get_wayland_compositor(context);

    return compositor && meta_wayland_session_lock_is_active(compositor);
}

static gboolean gnoblin_plugin_handle_event(const ClutterEvent* event, gpointer user_data) {
    MetaGnoblinPlugin* self = user_data;

    /* These events never entered the old stage captured-event path. */
    switch (clutter_event_type(event)) {
    case CLUTTER_NOTHING:
    case CLUTTER_DEVICE_ADDED:
    case CLUTTER_DEVICE_REMOVED:
    case CLUTTER_EVENT_LAST:
        return CLUTTER_EVENT_PROPAGATE;
    default:
        break;
    }

    /* Diagnostic UI must never accept or observe input in a locked session. */
    if (gnoblin_plugin_session_is_locked(self)) {
        gnoblin_plugin_ui_set_session_locked(self->diagnostics_ui, TRUE);
        return CLUTTER_EVENT_PROPAGATE;
    }

    return gnoblin_plugin_ui_handle_event(self->diagnostics_ui, event) ? CLUTTER_EVENT_STOP
                                                                       : CLUTTER_EVENT_PROPAGATE;
}

static gboolean gnoblin_plugin_is_modal(gpointer user_data) {
    MetaGnoblinPlugin* self = user_data;

    return !gnoblin_plugin_session_is_locked(self) &&
           gnoblin_plugin_ui_console_visible(self->diagnostics_ui);
}

static gboolean gnoblin_plugin_handle_key(const ClutterEvent* event, gpointer user_data) {
    MetaGnoblinPlugin* self = user_data;

    if (gnoblin_plugin_session_is_locked(self))
        return CLUTTER_EVENT_PROPAGATE;
    return gnoblin_plugin_ui_handle_key_event(self->diagnostics_ui, event)
               ? CLUTTER_EVENT_STOP
               : CLUTTER_EVENT_PROPAGATE;
}

static void gnoblin_plugin_session_lock_changed(MetaWaylandCompositor* compositor,
                                                MetaWaylandSessionLockState state,
                                                gpointer user_data) {
    (void)compositor;
    MetaGnoblinPlugin* self = user_data;

    gnoblin_plugin_ui_set_session_locked(self->diagnostics_ui,
                                         state != META_WAYLAND_SESSION_LOCK_UNLOCKED);
}

static gboolean gnoblin_plugin_toggle_console_bridge(MetaDisplay* display, gpointer user_data) {
    MetaGnoblinPlugin* self = user_data;
    if (!self || display != meta_plugin_get_display(META_PLUGIN(self)) ||
        gnoblin_plugin_session_is_locked(self))
        return FALSE;
    gnoblin_plugin_ui_toggle_console(self->diagnostics_ui);
    return TRUE;
}

/* --- Clipboard bridge for the diagnostic ImGui panels -------------------------------------
 * ImGui reads the clipboard synchronously, while Mutter's selection transfer is asynchronous.
 * Keep a cached copy of the clipboard text: refresh it whenever the selection owner changes,
 * and publish copies from the panels as an in-memory selection source. */

static MetaSelection* clipboard_selection;
static gulong clipboard_owner_handler;
static GCancellable* clipboard_cancellable;
static char* clipboard_cache;

typedef struct {
    GOutputStream* stream;
} ClipboardRead;

static const char* const clipboard_mimetypes[] = {"text/plain;charset=utf-8", "text/plain",
                                                  "UTF8_STRING", "STRING", NULL};

static void clipboard_read_done(GObject* source, GAsyncResult* result, gpointer user_data) {
    ClipboardRead* read = user_data;
    g_autoptr(GError) error = NULL;
    if (meta_selection_transfer_finish(META_SELECTION(source), result, &error)) {
        g_output_stream_close(read->stream, NULL, NULL);
        gsize size = g_memory_output_stream_get_data_size(G_MEMORY_OUTPUT_STREAM(read->stream));
        const char* data = g_memory_output_stream_get_data(G_MEMORY_OUTPUT_STREAM(read->stream));
        if (data && g_utf8_validate(data, (gssize)size, NULL)) {
            g_free(clipboard_cache);
            clipboard_cache = g_strndup(data, size);
        }
    }
    g_object_unref(read->stream);
    g_free(read);
}

static void clipboard_refresh(void) {
    if (!clipboard_selection)
        return;
    g_cancellable_cancel(clipboard_cancellable);
    g_object_unref(clipboard_cancellable);
    clipboard_cancellable = g_cancellable_new();

    GList* offered = meta_selection_get_mimetypes(clipboard_selection, META_SELECTION_CLIPBOARD);
    const char* chosen = NULL;
    for (guint i = 0; !chosen && clipboard_mimetypes[i]; i++)
        for (GList* item = offered; item; item = item->next)
            if (g_str_equal(item->data, clipboard_mimetypes[i])) {
                chosen = clipboard_mimetypes[i];
                break;
            }
    if (!chosen) {
        /* Nothing textual is on the clipboard. */
        g_free(clipboard_cache);
        clipboard_cache = NULL;
        g_list_free_full(offered, g_free);
        return;
    }
    ClipboardRead* read = g_new0(ClipboardRead, 1);
    read->stream = g_memory_output_stream_new_resizable();
    meta_selection_transfer_async(clipboard_selection, META_SELECTION_CLIPBOARD, chosen, -1,
                                  read->stream, clipboard_cancellable, clipboard_read_done, read);
    g_list_free_full(offered, g_free);
}

static void clipboard_owner_changed(MetaSelection* selection, MetaSelectionType type,
                                    MetaSelectionSource* owner, gpointer user_data) {
    (void)selection;
    (void)owner;
    (void)user_data;
    if (type == META_SELECTION_CLIPBOARD)
        clipboard_refresh();
}

static void gnoblin_plugin_clipboard_init(MetaDisplay* display) {
    clipboard_selection = meta_display_get_selection(display);
    clipboard_cancellable = g_cancellable_new();
    clipboard_owner_handler = g_signal_connect(clipboard_selection, "owner-changed",
                                               G_CALLBACK(clipboard_owner_changed), NULL);
    clipboard_refresh();
}

static void gnoblin_plugin_clipboard_shutdown(void) {
    if (clipboard_selection && clipboard_owner_handler)
        g_signal_handler_disconnect(clipboard_selection, clipboard_owner_handler);
    clipboard_owner_handler = 0;
    if (clipboard_cancellable) {
        g_cancellable_cancel(clipboard_cancellable);
        g_clear_object(&clipboard_cancellable);
    }
    clipboard_selection = NULL;
    g_clear_pointer(&clipboard_cache, g_free);
}

const char* gnoblin_plugin_clipboard_text(void) {
    return clipboard_cache ? clipboard_cache : "";
}

void gnoblin_plugin_clipboard_set_text(const char* text) {
    if (!clipboard_selection || !text || !g_utf8_validate(text, -1, NULL))
        return;
    g_free(clipboard_cache);
    clipboard_cache = g_strdup(text);
    g_autoptr(GBytes) bytes = g_bytes_new(text, strlen(text));
    g_autoptr(GError) error = NULL;
    g_autoptr(MetaSelectionSource) source =
        meta_selection_source_memory_new("text/plain;charset=utf-8", bytes, &error);
    if (source)
        meta_selection_set_owner(clipboard_selection, META_SELECTION_CLIPBOARD, source);
}

static void gnoblin_plugin_debug_bridge(gboolean enabled, gpointer user_data) {
    MetaGnoblinPlugin* plugin = user_data;
    gnoblin_plugin_ui_set_debug(plugin->diagnostics_ui, enabled);
}

static void gnoblin_plugin_start(MetaPlugin* plugin) {
    MetaGnoblinPlugin* self = META_GNOBLIN_PLUGIN(plugin);
    MetaDisplay* display = meta_plugin_get_display(plugin);
    MetaContext* context = meta_display_get_context(display);
    MetaBackend* backend = meta_context_get_backend(context);
    MetaCompositor* compositor = meta_display_get_compositor(display);
    MetaMonitorManager* monitor_manager = meta_backend_get_monitor_manager(backend);

#ifdef HAVE_IBUS
    /* GNOME Shell supplies the IBus bridge for text-input-v3 clients. Gnoblin has no shell,
     * so the compositor installs its own. Without ibus-daemon it passes every key through. */
    gnoblin_ibus_input_method_install(backend);
#endif

    self->background_group = meta_background_group_new();
    clutter_actor_insert_child_below(meta_compositor_get_window_group(compositor),
                                     self->background_group, NULL);
    g_signal_connect(monitor_manager, "monitors-changed",
                     G_CALLBACK(gnoblin_plugin_monitors_changed), plugin);
    gnoblin_plugin_update_backgrounds(self);

    /* Diagnostics belong directly to the stage, after Mutter's window,
     * popup, input-panel, and feedback groups. Layer-shell overlay surfaces
     * share the top-window group, so parenting this actor there lets a shell
     * cover recovery and console UI. */
    self->diagnostics_ui = gnoblin_plugin_ui_new(meta_backend_get_stage(backend), display);
    self->input = (GnoblinPluginInput){gnoblin_plugin_handle_event, self, gnoblin_plugin_handle_key,
                                       gnoblin_plugin_is_modal};
    gnoblin_plugin_input_attach(G_OBJECT(display), &self->input);
    self->wayland_compositor = meta_context_get_wayland_compositor(context);
    if (self->wayland_compositor)
        self->session_lock_callback_id = meta_wayland_session_lock_add_state_changed_callback(
            self->wayland_compositor, gnoblin_plugin_session_lock_changed, self, NULL);
    gnoblin_plugin_ui_set_session_locked(self->diagnostics_ui,
                                         gnoblin_plugin_session_is_locked(self));
    gnoblin_native_control_set_console_toggle_handler(display, gnoblin_plugin_toggle_console_bridge,
                                                      self);
    gnoblin_native_control_set_debug_handler(display, gnoblin_plugin_debug_bridge, self);
    gnoblin_plugin_clipboard_init(display);
    clutter_actor_show(meta_backend_get_stage(backend));
}

static void meta_gnoblin_plugin_dispose(GObject* object) {
    MetaGnoblinPlugin* self = META_GNOBLIN_PLUGIN(object);
    MetaDisplay* display = meta_plugin_get_display(META_PLUGIN(self));
    gnoblin_native_control_set_console_toggle_handler(display, NULL, NULL);

    gnoblin_plugin_input_detach(G_OBJECT(display), &self->input);
    if (self->wayland_compositor && self->session_lock_callback_id != 0) {
        meta_wayland_session_lock_remove_state_changed_callback(self->wayland_compositor,
                                                                self->session_lock_callback_id);
        self->session_lock_callback_id = 0;
        self->wayland_compositor = NULL;
    }
    gnoblin_plugin_clipboard_shutdown();
    g_clear_pointer(&self->diagnostics_ui, gnoblin_plugin_ui_free);
    g_clear_pointer(&self->background_group, clutter_actor_destroy);

    G_OBJECT_CLASS(meta_gnoblin_plugin_parent_class)->dispose(object);
}

static void gnoblin_plugin_map(MetaPlugin* plugin, MetaWindowActor* actor) {
    if (!meta_gnoblin_frame_renderer_prepare_map(plugin, actor))
        meta_plugin_map_completed(plugin, actor);
}

static void gnoblin_plugin_minimize(MetaPlugin* plugin, MetaWindowActor* actor) {
    if (!meta_gnoblin_animation_window_minimizing(plugin, actor))
        meta_plugin_minimize_completed(plugin, actor);
}

static void gnoblin_plugin_unminimize(MetaPlugin* plugin, MetaWindowActor* actor) {
    if (!meta_gnoblin_animation_window_restoring(plugin, actor))
        meta_plugin_unminimize_completed(plugin, actor);
}

static void gnoblin_plugin_destroy(MetaPlugin* plugin, MetaWindowActor* actor) {
    if (!meta_gnoblin_animation_window_destroying(plugin, actor))
        meta_plugin_destroy_completed(plugin, actor);
}

static void gnoblin_plugin_size_change(MetaPlugin* plugin, MetaWindowActor* actor,
                                       MetaSizeChange which_change, MtkRectangle* old_frame_rect,
                                       MtkRectangle* old_buffer_rect) {
    meta_gnoblin_animation_size_change(plugin, actor, which_change, old_frame_rect,
                                       old_buffer_rect);
}

static void gnoblin_plugin_size_changed(MetaPlugin* plugin, MetaWindowActor* actor) {
    meta_gnoblin_animation_size_changed(plugin, actor);
}

static void gnoblin_plugin_kill_window_effects(MetaPlugin* plugin, MetaWindowActor* actor) {
    meta_gnoblin_frame_renderer_cancel_map(actor);
    meta_gnoblin_animation_cancel_window_effects(plugin, actor);
}

static void gnoblin_plugin_switch_workspace(MetaPlugin* plugin, gint from, gint to,
                                            MetaMotionDirection direction) {
    if (!meta_gnoblin_animation_switch_workspace(plugin, from, to, direction))
        meta_plugin_switch_workspace_completed(plugin);
}

static void gnoblin_plugin_kill_switch_workspace(MetaPlugin* plugin) {
    meta_gnoblin_animation_kill_switch_workspace(plugin);
}

static void gnoblin_plugin_show_window_menu(MetaPlugin* plugin, MetaWindow* window,
                                            MetaWindowMenuType menu, int x, int y) {
    gnoblin_native_control_window_menu_requested(meta_plugin_get_display(plugin), window, menu, x,
                                                 y);
}

static void gnoblin_plugin_locate_pointer(MetaPlugin* plugin) {
    gnoblin_native_control_locate_pointer(meta_plugin_get_display(plugin));
}

static void meta_gnoblin_plugin_class_init(MetaGnoblinPluginClass* klass) {
    MetaPluginClass* plugin_class = META_PLUGIN_CLASS(klass);
    GObjectClass* object_class = G_OBJECT_CLASS(klass);

    object_class->dispose = meta_gnoblin_plugin_dispose;

    plugin_class->start = gnoblin_plugin_start;
    plugin_class->show_window_menu = gnoblin_plugin_show_window_menu;
    plugin_class->locate_pointer = gnoblin_plugin_locate_pointer;
    plugin_class->map = gnoblin_plugin_map;
    plugin_class->minimize = gnoblin_plugin_minimize;
    plugin_class->unminimize = gnoblin_plugin_unminimize;
    plugin_class->destroy = gnoblin_plugin_destroy;
    plugin_class->size_change = gnoblin_plugin_size_change;
    plugin_class->size_changed = gnoblin_plugin_size_changed;
    plugin_class->kill_window_effects = gnoblin_plugin_kill_window_effects;
    plugin_class->switch_workspace = gnoblin_plugin_switch_workspace;
    plugin_class->kill_switch_workspace = gnoblin_plugin_kill_switch_workspace;
}

static void meta_gnoblin_plugin_init(MetaGnoblinPlugin* self) {
    (void)self;
}

float gnoblin_plugin_paint_scale(ClutterPaintContext* paint_context) {
    ClutterStageView* view;

    if (!paint_context)
        return 1.0f;
    view = clutter_paint_context_get_stage_view(paint_context);
    return view ? clutter_stage_view_get_scale(view) : 1.0f;
}
