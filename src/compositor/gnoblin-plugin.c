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
#include "meta/meta-wayland-compositor.h"
#include "core/gnoblin-animation.h"
#include "core/gnoblin-native-control.h"
#include "wayland/meta-wayland-session-lock.h"
#ifdef HAVE_IBUS
#include "backends/gnoblin-ibus-input-method.h"
#endif

#include "gnoblin-plugin-ui.h"

typedef struct _MetaGnoblinPlugin MetaGnoblinPlugin;
typedef struct _MetaGnoblinPluginClass MetaGnoblinPluginClass;

struct _MetaGnoblinPlugin {
    MetaPlugin parent;
    ClutterActor* background_group;
    GnoblinPluginUi* diagnostics_ui;
    gulong stage_capture_id;
    gulong session_lock_callback_id;
    MetaWaylandCompositor* wayland_compositor;
};

struct _MetaGnoblinPluginClass {
    MetaPluginClass parent_class;
};

#define META_TYPE_GNOBLIN_PLUGIN (meta_gnoblin_plugin_get_type())
#define META_GNOBLIN_PLUGIN(obj) \
    (G_TYPE_CHECK_INSTANCE_CAST((obj), META_TYPE_GNOBLIN_PLUGIN, MetaGnoblinPlugin))
#define META_GNOBLIN_PLUGIN_CLASS(klass) \
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

static void gnoblin_plugin_monitors_changed(MetaMonitorManager* manager,
                                             MetaGnoblinPlugin* self) {
    (void)manager;
    gnoblin_plugin_update_backgrounds(self);
}

static gboolean
gnoblin_plugin_session_is_locked(MetaGnoblinPlugin* self) {
    MetaDisplay* display = meta_plugin_get_display(META_PLUGIN(self));
    MetaContext* context = meta_display_get_context(display);
    MetaWaylandCompositor* compositor = meta_context_get_wayland_compositor(context);

    return compositor && meta_wayland_session_lock_is_active(compositor);
}

static gboolean
gnoblin_plugin_stage_captured_event(ClutterActor* stage, ClutterEvent* event,
                                    MetaGnoblinPlugin* self) {
    (void)stage;

    /* Diagnostic UI must never accept or observe input in a locked session. */
    if (gnoblin_plugin_session_is_locked(self)) {
        gnoblin_plugin_ui_set_session_locked(self->diagnostics_ui, TRUE);
        return CLUTTER_EVENT_PROPAGATE;
    }

    return gnoblin_plugin_ui_handle_event(self->diagnostics_ui, event)
               ? CLUTTER_EVENT_STOP
               : CLUTTER_EVENT_PROPAGATE;
}

static void
gnoblin_plugin_session_lock_changed(MetaWaylandCompositor* compositor,
                                    MetaWaylandSessionLockState state,
                                    gpointer user_data) {
    (void)compositor;
    MetaGnoblinPlugin* self = user_data;

    gnoblin_plugin_ui_set_session_locked(self->diagnostics_ui,
                                         state != META_WAYLAND_SESSION_LOCK_UNLOCKED);
}

static gboolean
gnoblin_plugin_toggle_console_bridge(MetaDisplay* display, gpointer user_data) {
    MetaGnoblinPlugin* self = user_data;
    if (!self || display != meta_plugin_get_display(META_PLUGIN(self)) ||
        gnoblin_plugin_session_is_locked(self))
        return FALSE;
    gnoblin_plugin_ui_toggle_console(self->diagnostics_ui);
    return TRUE;
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
    self->diagnostics_ui = gnoblin_plugin_ui_new(
        meta_backend_get_stage(backend), display);
    self->stage_capture_id = g_signal_connect(
        meta_backend_get_stage(backend), "captured-event",
        G_CALLBACK(gnoblin_plugin_stage_captured_event), self);
    self->wayland_compositor = meta_context_get_wayland_compositor(context);
    if (self->wayland_compositor)
        self->session_lock_callback_id = meta_wayland_session_lock_add_state_changed_callback(
            self->wayland_compositor, gnoblin_plugin_session_lock_changed, self, NULL);
    gnoblin_plugin_ui_set_session_locked(self->diagnostics_ui,
                                         gnoblin_plugin_session_is_locked(self));
    gnoblin_native_control_set_console_toggle_handler(
        display, gnoblin_plugin_toggle_console_bridge, self);

    clutter_actor_show(meta_backend_get_stage(backend));
}

static void
meta_gnoblin_plugin_dispose(GObject* object) {
    MetaGnoblinPlugin* self = META_GNOBLIN_PLUGIN(object);
    MetaDisplay* display = meta_plugin_get_display(META_PLUGIN(self));
    gnoblin_native_control_set_console_toggle_handler(display, NULL, NULL);

    if (self->stage_capture_id != 0) {
        MetaDisplay* display = meta_plugin_get_display(META_PLUGIN(self));
        MetaContext* context = meta_display_get_context(display);
        MetaBackend* backend = meta_context_get_backend(context);
        g_signal_handler_disconnect(meta_backend_get_stage(backend), self->stage_capture_id);
        self->stage_capture_id = 0;
    }
    if (self->wayland_compositor && self->session_lock_callback_id != 0) {
        meta_wayland_session_lock_remove_state_changed_callback(self->wayland_compositor,
                                                                 self->session_lock_callback_id);
        self->session_lock_callback_id = 0;
        self->wayland_compositor = NULL;
    }
    g_clear_pointer(&self->diagnostics_ui, gnoblin_plugin_ui_free);
    g_clear_pointer(&self->background_group, clutter_actor_destroy);

    G_OBJECT_CLASS(meta_gnoblin_plugin_parent_class)->dispose(object);
}

static void gnoblin_plugin_map(MetaPlugin* plugin, MetaWindowActor* actor) {
    /* No asynchronous effect is active, so release Mutter's map lifecycle now. */
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
