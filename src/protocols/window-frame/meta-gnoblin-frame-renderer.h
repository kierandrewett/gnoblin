/* SPDX-License-Identifier: GPL-2.0-or-later */
#pragma once
#include "wayland/meta-gnoblin-window-frame.h"
#include "meta/meta-plugin.h"
#include "meta/meta-window-actor.h"

void meta_gnoblin_frame_renderer_init(MetaWaylandCompositor* compositor);
void meta_gnoblin_frame_renderer_sync(MetaWindow* window, const MetaGnoblinFrameLayout* layout);
void meta_gnoblin_frame_renderer_style_changed(MetaWindow* window);
META_EXPORT gboolean meta_gnoblin_frame_renderer_prepare_map(MetaPlugin* plugin,
                                                             MetaWindowActor* actor);
META_EXPORT void meta_gnoblin_frame_renderer_cancel_map(MetaWindowActor* actor);
void meta_gnoblin_frame_renderer_merge_state(MetaWaylandSurfaceState* from,
                                             MetaWaylandSurfaceState* to);
GVariant* meta_gnoblin_frame_renderer_status(MetaWindow* window);
