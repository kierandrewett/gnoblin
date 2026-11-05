/* Gnoblin-owned native compositor animation API. */
#pragma once

#include "meta/meta-window-actor.h"
#include "meta/meta-plugin.h"
#include "core/gnoblin-native-control.h"
#include "meta/util.h"

void meta_gnoblin_animation_window_mapped(MetaWindowActor* window_actor);
META_EXPORT
void meta_gnoblin_animation_size_change(MetaPlugin* plugin, MetaWindowActor* window_actor,
                                        MetaSizeChange which_change, MtkRectangle* old_frame_rect,
                                        MtkRectangle* old_buffer_rect);
META_EXPORT
void meta_gnoblin_animation_size_changed(MetaPlugin* plugin, MetaWindowActor* window_actor);
META_EXPORT
gboolean meta_gnoblin_animation_window_destroying(MetaPlugin* plugin,
                                                  MetaWindowActor* window_actor);
META_EXPORT
gboolean meta_gnoblin_animation_window_minimizing(MetaPlugin* plugin,
                                                  MetaWindowActor* window_actor);
META_EXPORT
gboolean meta_gnoblin_animation_window_restoring(MetaPlugin* plugin, MetaWindowActor* window_actor);
META_EXPORT
void meta_gnoblin_animation_cancel_window_effects(MetaPlugin* plugin,
                                                  MetaWindowActor* window_actor);
META_EXPORT
gboolean meta_gnoblin_animation_switch_workspace(MetaPlugin* plugin, gint from, gint to,
                                                 MetaMotionDirection direction);
META_EXPORT
void meta_gnoblin_animation_kill_switch_workspace(MetaPlugin* plugin);
GVariant* meta_gnoblin_animation_dispatch(MetaDisplay* display, const char* method,
                                          GVariant* arguments, GError** error);
