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

#include "meta/meta-plugin.h"
#include "core/gnoblin-animation.h"
#include "core/gnoblin-native-control.h"

typedef struct _MetaGnoblinPlugin MetaGnoblinPlugin;
typedef struct _MetaGnoblinPluginClass MetaGnoblinPluginClass;

struct _MetaGnoblinPlugin {
    MetaPlugin parent;
};

struct _MetaGnoblinPluginClass {
    MetaPluginClass parent_class;
};

META_PLUGIN_DECLARE(MetaGnoblinPlugin, meta_gnoblin_plugin);

static void gnoblin_plugin_map(MetaPlugin* plugin, MetaWindowActor* actor) {
    /* No asynchronous effect is active, so release Mutter's map lifecycle now. */
    meta_plugin_map_completed(plugin, actor);
}

static void gnoblin_plugin_destroy(MetaPlugin* plugin, MetaWindowActor* actor) {
    if (!meta_gnoblin_animation_window_destroying(plugin, actor))
        meta_plugin_destroy_completed(plugin, actor);
}

static void gnoblin_plugin_kill_window_effects(MetaPlugin* plugin, MetaWindowActor* actor) {
    meta_gnoblin_animation_cancel_window_effects(plugin, actor);
}

static void gnoblin_plugin_show_window_menu(MetaPlugin* plugin, MetaWindow* window,
                                            MetaWindowMenuType menu, int x, int y) {
    gnoblin_native_control_window_menu_requested(meta_plugin_get_display(plugin), window, menu, x,
                                                 y);
}

static void meta_gnoblin_plugin_class_init(MetaGnoblinPluginClass* klass) {
    MetaPluginClass* plugin_class = META_PLUGIN_CLASS(klass);

    plugin_class->show_window_menu = gnoblin_plugin_show_window_menu;
    plugin_class->map = gnoblin_plugin_map;
    plugin_class->destroy = gnoblin_plugin_destroy;
    plugin_class->kill_window_effects = gnoblin_plugin_kill_window_effects;
}

static void meta_gnoblin_plugin_init(MetaGnoblinPlugin* self) {
    (void)self;
}
