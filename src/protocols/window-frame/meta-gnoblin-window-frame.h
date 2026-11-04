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
void meta_gnoblin_window_frame_surface_role_assigned(MetaWaylandSurface* surface,
                                                     gboolean is_toplevel);
void meta_gnoblin_window_frame_configure(MetaWindow* window,
                                         MetaWaylandWindowConfiguration* configuration);
void meta_gnoblin_window_frame_commit(MetaWindow* window,
                                      MetaWaylandWindowConfiguration* configuration,
                                      MtkRectangle* geometry);
void meta_gnoblin_window_frame_rect_to_client(MetaWindow* window,
                                              const MetaWaylandWindowConfiguration* configuration,
                                              MtkRectangle* rect);
void meta_gnoblin_window_frame_sync_actor(MetaWindow* window, ClutterActor* surface);
void meta_gnoblin_window_frame_sync_renderer(MetaWindow* window);
gboolean meta_gnoblin_window_frame_get_visible_geometry(MetaWindow* window, MtkRectangle* geometry);
/* Geometry for compositor-owned frame effects. CSD uses the client's declared
 * visible bounds; negotiated SSD expands those bounds by the server frame. */
gboolean meta_gnoblin_window_frame_get_effect_geometry(MetaWindow* window, MtkRectangle* geometry);
void meta_gnoblin_window_frame_emit_changed(MetaWindow* window);
gboolean meta_gnoblin_window_frame_is_active(MetaWindow* window);

/* Xwayland windows. Mutter creates no X11 frame window; the frame around the client is drawn by
 * the frame renderer and the geometry follows the Wayland commit (custom_frame_extents). */
gboolean meta_gnoblin_window_frame_x11_supported(MetaWindow* window);
void meta_gnoblin_window_frame_x11_update(MetaWindow* window);
void meta_gnoblin_window_frame_x11_set_declared_extents(MetaWindow* window,
                                                        const MetaFrameBorder* extents);
gboolean meta_gnoblin_window_frame_x11_get_border(MetaWindow* window, MetaFrameBorder* border);
