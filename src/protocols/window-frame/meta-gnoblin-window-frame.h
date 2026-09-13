/* SPDX-License-Identifier: GPL-2.0-or-later */
#pragma once

#include "meta/window.h"
#include "meta/util.h"
#include "wayland/meta-wayland-types.h"

typedef struct {
    int crop[4]; /* top, right, bottom, left; logical pixels */
    int border[4];
    int mode; /* committed protocol mode: 1 CSD, 2 SSD */
} MetaGnoblinFrameLayout;

void meta_gnoblin_window_frame_init(MetaWaylandCompositor* compositor);
void meta_gnoblin_window_frame_configure(MetaWindow* window,
                                         MetaWaylandWindowConfiguration* configuration);
void meta_gnoblin_window_frame_commit(MetaWindow* window,
                                      MetaWaylandWindowConfiguration* configuration,
                                      MtkRectangle* geometry);
void meta_gnoblin_window_frame_sync_actor(MetaWindow* window, ClutterActor* surface);
void meta_gnoblin_window_frame_emit_changed(MetaWindow* window);
gboolean meta_gnoblin_window_frame_is_active(MetaWindow* window);
