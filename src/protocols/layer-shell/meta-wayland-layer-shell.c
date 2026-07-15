/*
 * gnoblin: wlr-layer-shell-unstable-v1 support for mutter.
 *
 * Implements zwlr_layer_shell_v1 / zwlr_layer_surface_v1 by mirroring the
 * xdg-shell role machinery: a layer surface is a MetaWaylandShellSurface whose
 * backing MetaWindow is positioned by the compositor against an output edge
 * (anchors + margins + size), placed in a stacking layer derived from the
 * requested wlr layer, and optionally given keyboard focus.
 *
 * Scope: global + surface->window mapping + requested wl_output +
 * anchors/margins/size + keyboard focus + layer->stacking + positive
 * exclusive-zone work-area reservation + xdg-popup parent handoff.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-layer-shell.h"

#include <gio/gio.h>

#include "backends/meta-backend-private.h"
#include "backends/meta-logical-monitor-private.h"
#include "backends/meta-monitor-private.h"
#include "backends/meta-monitor-manager-private.h"
#include "core/boxes-private.h"
#include "core/display-private.h"
#include "core/window-private.h"
#include "core/workspace-private.h"
#include "meta/meta-backend.h"
#include "meta/meta-workspace-manager.h"
#include "wayland/meta-wayland-outputs.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-seat.h"
#include "wayland/gnoblin-config.h"
#include "wayland/meta-wayland-shell-surface.h"
#include "wayland/meta-wayland-surface-private.h"
#include "wayland/meta-wayland-window-configuration.h"
#include "wayland/meta-wayland.h"
#include "wayland/meta-window-wayland.h"
#include "wayland/meta-wayland-xdg-shell.h"

#include "wlr-layer-shell-unstable-v1-server-protocol.h"

/* Advertise version 5: anchors, margins, set_layer, on_demand keyboard
 * interactivity, and explicit exclusive edge. */
#define META_WLR_LAYER_SHELL_V1_VERSION 5

/* When a requested wl_output disappears, Mutter tears down the virtual output,
 * stage view and native CRTC in the same main-loop turn that layer-shell sends
 * closed. Destroying the MetaWindow from an idle in that turn can race native
 * view cleanup and trip stale CRTC casts. Give clients a brief chance to process
 * closed and destroy the layer object; if they do not, clean the window up once
 * hot-unplug has settled. */
#define CLOSED_LAYER_WINDOW_DESTROY_DELAY_MS 250

#define META_LAYER_SURFACE_ANCHOR_MASK \
  (ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP | \
   ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM | \
   ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT | \
   ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT)

typedef struct _MetaWaylandLayerSurfaceState
{
  uint32_t anchor;
  int32_t  exclusive_zone;
  int32_t  margin_top;
  int32_t  margin_right;
  int32_t  margin_bottom;
  int32_t  margin_left;
  uint32_t keyboard_interactivity;
  uint32_t desired_width;
  uint32_t desired_height;
  uint32_t layer;
  uint32_t exclusive_edge;
  gboolean has_exclusive_edge;
} MetaWaylandLayerSurfaceState;

struct _MetaWaylandLayerSurface
{
  MetaWaylandShellSurface parent;

  struct wl_resource *resource;
  MetaWaylandOutput  *output;
  char               *namespace;

  MetaWaylandLayerSurfaceState pending;
  MetaWaylandLayerSurfaceState current;
  gboolean has_pending_state;

  gboolean configured;
  gboolean has_acked_configure;
  GQueue   configure_serials;
  uint32_t configure_serial;
  uint32_t acked_configure_serial;
  int      last_sent_width;
  int      last_sent_height;
  uint32_t initial_layer;
  gulong   output_destroyed_handler_id;
  guint    destroy_window_idle_id;
  gboolean closed;
};

typedef struct _DestroyWindowIdleData
{
  GWeakRef layer_surface;
} DestroyWindowIdleData;

#define META_TYPE_WAYLAND_LAYER_SURFACE (meta_wayland_layer_surface_get_type ())
G_DECLARE_FINAL_TYPE (MetaWaylandLayerSurface,
                      meta_wayland_layer_surface,
                      META, WAYLAND_LAYER_SURFACE,
                      MetaWaylandShellSurface)

G_DEFINE_TYPE (MetaWaylandLayerSurface,
               meta_wayland_layer_surface,
               META_TYPE_WAYLAND_SHELL_SURFACE)

/* ------------------------------------------------------------------ */

static void close_layer_surface (MetaWaylandLayerSurface *layer_surface,
                                 gboolean                 invalidate_work_areas);

static void
gnoblin_layer_dismiss_trampoline (MetaWindow *window)
{
  MetaWaylandLayerSurface *layer_surface =
    g_object_get_data (G_OBJECT (window), "gnoblin-layer-surface");

  if (layer_surface)
    close_layer_surface (layer_surface, FALSE);
}

gboolean
meta_wayland_surface_is_layer_shell (MetaWaylandSurface *surface)
{
  return surface &&
         surface->role &&
         META_IS_WAYLAND_LAYER_SURFACE (surface->role);
}

static MetaDisplay *
display_from_surface (MetaWaylandSurface *surface)
{
  MetaContext *context =
    meta_wayland_compositor_get_context (surface->compositor);

  return meta_context_get_display (context);
}

static MtkRectangle
get_monitor_layout (MetaWaylandLayerSurface *layer_surface)
{
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (layer_surface));
  MetaContext *context =
    meta_wayland_compositor_get_context (surface->compositor);
  MetaBackend *backend = meta_context_get_backend (context);
  MetaMonitorManager *monitor_manager =
    meta_backend_get_monitor_manager (backend);
  MetaLogicalMonitor *logical_monitor = NULL;

  /* Honour the output the client requested in get_layer_surface, if any and
   * still connected; otherwise fall back to the primary monitor. */
  if (layer_surface->output)
    {
      MetaMonitor *monitor =
        meta_wayland_output_get_monitor (layer_surface->output);

      if (monitor)
        logical_monitor = meta_monitor_get_logical_monitor (monitor);
    }

  if (!logical_monitor)
    logical_monitor =
      meta_monitor_manager_get_primary_logical_monitor (monitor_manager);

  if (logical_monitor)
    return meta_logical_monitor_get_layout (logical_monitor);

  return (MtkRectangle) { 0, 0, 1920, 1080 };
}

static gboolean
is_single_anchor_edge (uint32_t edge)
{
  return (edge == ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP ||
          edge == ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM ||
          edge == ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT ||
          edge == ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
}

static gboolean
has_horizontal_span (uint32_t anchor)
{
  return ((anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT) &&
          (anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT));
}

static gboolean
has_vertical_span (uint32_t anchor)
{
  return ((anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP) &&
          (anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM));
}

static gboolean
validate_layer_surface_state (MetaWaylandLayerSurface      *layer_surface,
                              MetaWaylandLayerSurfaceState *state)
{
  if (state->anchor & ~META_LAYER_SURFACE_ANCHOR_MASK)
    {
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_ANCHOR,
                              "invalid anchor bitfield 0x%x", state->anchor);
      return FALSE;
    }

  if (state->desired_width == 0 && !has_horizontal_span (state->anchor))
    {
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SIZE,
                              "width 0 requires left and right anchors");
      return FALSE;
    }

  if (state->desired_height == 0 && !has_vertical_span (state->anchor))
    {
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SIZE,
                              "height 0 requires top and bottom anchors");
      return FALSE;
    }

  if (state->keyboard_interactivity >
      ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_ON_DEMAND)
    {
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_KEYBOARD_INTERACTIVITY,
                              "invalid keyboard interactivity %u",
                              state->keyboard_interactivity);
      return FALSE;
    }

  if (state->layer > ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY)
    {
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SURFACE_STATE,
                              "invalid layer %u", state->layer);
      return FALSE;
    }

  if (state->has_exclusive_edge &&
      (!is_single_anchor_edge (state->exclusive_edge) ||
       !(state->anchor & state->exclusive_edge)))
    {
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_EXCLUSIVE_EDGE,
                              "exclusive edge 0x%x is not one anchored edge",
                              state->exclusive_edge);
      return FALSE;
    }

  return TRUE;
}

static gboolean
layer_surface_state_equal (const MetaWaylandLayerSurfaceState *a,
                           const MetaWaylandLayerSurfaceState *b)
{
  return (a->anchor == b->anchor &&
          a->exclusive_zone == b->exclusive_zone &&
          a->margin_top == b->margin_top &&
          a->margin_right == b->margin_right &&
          a->margin_bottom == b->margin_bottom &&
          a->margin_left == b->margin_left &&
          a->keyboard_interactivity == b->keyboard_interactivity &&
          a->desired_width == b->desired_width &&
          a->desired_height == b->desired_height &&
          a->layer == b->layer &&
          a->exclusive_edge == b->exclusive_edge &&
          a->has_exclusive_edge == b->has_exclusive_edge);
}

static gboolean
get_effective_exclusive_edge (MetaWaylandLayerSurfaceState *state,
                              uint32_t                     *edge)
{
  gboolean anchor_left   = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT);
  gboolean anchor_right  = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
  gboolean anchor_top    = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP);
  gboolean anchor_bottom = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM);

  if (state->has_exclusive_edge)
    {
      *edge = state->exclusive_edge;
      return TRUE;
    }

  if (anchor_top && !anchor_bottom && (!anchor_left || anchor_right) &&
      (!anchor_right || anchor_left))
    {
      *edge = ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP;
      return TRUE;
    }
  else if (anchor_bottom && !anchor_top && (!anchor_left || anchor_right) &&
           (!anchor_right || anchor_left))
    {
      *edge = ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM;
      return TRUE;
    }
  else if (anchor_left && !anchor_right && (!anchor_top || anchor_bottom) &&
           (!anchor_bottom || anchor_top))
    {
      *edge = ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT;
      return TRUE;
    }
  else if (anchor_right && !anchor_left && (!anchor_top || anchor_bottom) &&
           (!anchor_bottom || anchor_top))
    {
      *edge = ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT;
      return TRUE;
    }

  return FALSE;
}

static MtkRectangle
calculate_geometry (MetaWaylandLayerSurface *layer_surface,
                    MtkRectangle             mon)
{
  MetaWaylandLayerSurfaceState *state = &layer_surface->current;
  gboolean anchor_left   = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT);
  gboolean anchor_right  = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
  gboolean anchor_top    = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP);
  gboolean anchor_bottom = !!(state->anchor & ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM);
  int ml = state->margin_left, mr = state->margin_right;
  int mt = state->margin_top, mb = state->margin_bottom;
  int w = (int) state->desired_width;
  int h = (int) state->desired_height;
  MtkRectangle r;

  /* A zero dimension means "stretch between opposite anchors". */
  if (w <= 0)
    w = MAX (1, mon.width - ml - mr);
  if (h <= 0)
    h = MAX (1, mon.height - mt - mb);

  /* Clamp an over-large size to the monitor when anchored to a single edge of
   * that axis. This lets a surface that wants to span the whole output but
   * reserve only one edge (a bar with a drop-down area: anchor TOP|LEFT|RIGHT +
   * a huge height) be sized to the output without the client having to know the
   * resolution — and, crucially, keep a single-edge anchor so the exclusive
   * zone resolves to that edge (all-four anchors make the edge ambiguous, and
   * sctk has no v5 set_exclusive_edge). */
  if (anchor_top != anchor_bottom)
    h = MIN (h, mon.height - (anchor_top ? mt : mb));
  if (anchor_left != anchor_right)
    w = MIN (w, mon.width - (anchor_left ? ml : mr));

  if (anchor_left && !anchor_right)
    r.x = mon.x + ml;
  else if (anchor_right && !anchor_left)
    r.x = mon.x + mon.width - w - mr;
  else
    r.x = mon.x + (mon.width - w) / 2 + (ml - mr) / 2;

  if (anchor_top && !anchor_bottom)
    r.y = mon.y + mt;
  else if (anchor_bottom && !anchor_top)
    r.y = mon.y + mon.height - h - mb;
  else
    r.y = mon.y + (mon.height - h) / 2 + (mt - mb) / 2;

  r.width = w;
  r.height = h;
  return r;
}

static void
send_configure (MetaWaylandLayerSurface *layer_surface,
                int                      width,
                int                      height)
{
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (layer_surface));
  struct wl_display *display = surface->compositor->wayland_display;
  uint32_t serial = wl_display_next_serial (display);

  layer_surface->configure_serial = serial;
  g_queue_push_tail (&layer_surface->configure_serials,
                     GUINT_TO_POINTER (serial));
  layer_surface->last_sent_width = width;
  layer_surface->last_sent_height = height;
  layer_surface->configured = TRUE;

  zwlr_layer_surface_v1_send_configure (layer_surface->resource,
                                        serial, width, height);
}

static void
reset_layer_surface_state (MetaWaylandLayerSurface *layer_surface)
{
  layer_surface->pending = (MetaWaylandLayerSurfaceState) {
    .layer = layer_surface->initial_layer,
  };
  layer_surface->current = layer_surface->pending;
  layer_surface->has_pending_state = FALSE;
  layer_surface->configured = FALSE;
  layer_surface->has_acked_configure = FALSE;
  g_queue_clear (&layer_surface->configure_serials);
  layer_surface->configure_serial = 0;
  layer_surface->acked_configure_serial = 0;
  layer_surface->last_sent_width = 0;
  layer_surface->last_sent_height = 0;
}

static MetaStackLayer
stack_layer_for_wlr_layer (uint32_t wlr_layer)
{
  switch (wlr_layer)
    {
    case ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND:
      return META_LAYER_DESKTOP;
    case ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM:
      return META_LAYER_BOTTOM;
    case ZWLR_LAYER_SHELL_V1_LAYER_TOP:
      return META_LAYER_DOCK;
    case ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY:
    default:
      return META_LAYER_OVERRIDE_REDIRECT;
    }
}

static void
apply_window_type_and_layer (MetaWaylandLayerSurface *layer_surface,
                             MetaWindow              *window)
{
  MetaStackLayer stack_layer =
    stack_layer_for_wlr_layer (layer_surface->current.layer);

  /* Window type drives general dock-like behaviour (skip taskbar, etc.). */
  if (layer_surface->current.layer <= ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM)
    window->type = META_WINDOW_DESKTOP;
  else
    window->type = META_WINDOW_DOCK;

  meta_window_recalc_features (window);

  /* Pin the exact stacking layer (background/bottom/top/overlay). The
   * gnoblin calculate_layer patch in meta-window-wayland.c reads this. */
  g_object_set_data (G_OBJECT (window),
                     META_WAYLAND_LAYER_SHELL_STACK_LAYER_KEY,
                     GINT_TO_POINTER ((int) stack_layer + 1));
  g_object_set_data (G_OBJECT (window),
                     META_WAYLAND_LAYER_SHELL_KEYBOARD_FOCUSABLE_KEY,
                     GINT_TO_POINTER (
                       layer_surface->current.keyboard_interactivity !=
                       ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE));

  meta_window_update_layer (window);

  meta_window_on_all_workspaces_changed (window);
}

static void
invalidate_work_areas_for_window (MetaWindow *window)
{
  MetaWorkspace *workspace;

  workspace = meta_window_get_workspace (window);
  if (workspace)
    {
      meta_workspace_invalidate_work_area (workspace);
    }
  else
    {
      MetaWorkspaceManager *workspace_manager =
        meta_display_get_workspace_manager (meta_window_get_display (window));
      GList *l;

      for (l = meta_workspace_manager_get_workspaces (workspace_manager);
           l; l = l->next)
        meta_workspace_invalidate_work_area (l->data);
    }
}

static void
clear_exclusive_zone_struts (MetaWindow *window,
                             gboolean    invalidate_work_areas)
{
  g_clear_slist (&window->struts, g_free);

  if (invalidate_work_areas)
    invalidate_work_areas_for_window (window);
}

static void
update_exclusive_zone_struts (MetaWaylandLayerSurface *layer_surface,
                              MetaWindow              *window,
                              MtkRectangle             mon)
{
  MetaWaylandLayerSurfaceState *state = &layer_surface->current;
  int zone = state->exclusive_zone;
  uint32_t edge;

  /* Drop any strut we previously reserved before recomputing. */
  g_clear_slist (&window->struts, g_free);

  if (zone > 0 && get_effective_exclusive_edge (state, &edge))
    {
      MetaStrut *strut = g_new0 (MetaStrut, 1);

      if (edge == ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP)
        {
          strut->side = META_SIDE_TOP;
          strut->rect = (MtkRectangle) {
            mon.x, mon.y, mon.width, zone + state->margin_top
          };
        }
      else if (edge == ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM)
        {
          strut->side = META_SIDE_BOTTOM;
          strut->rect = (MtkRectangle) {
            mon.x, mon.y + mon.height - (zone + state->margin_bottom),
            mon.width, zone + state->margin_bottom
          };
        }
      else if (edge == ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT)
        {
          strut->side = META_SIDE_LEFT;
          strut->rect = (MtkRectangle) {
            mon.x, mon.y, zone + state->margin_left, mon.height
          };
        }
      else
        {
          strut->side = META_SIDE_RIGHT;
          strut->rect = (MtkRectangle) {
            mon.x + mon.width - (zone + state->margin_right), mon.y,
            zone + state->margin_right, mon.height
          };
        }

      window->struts = g_slist_prepend (NULL, strut);
    }

  /* Recompute work areas so normal windows reflow around (or reclaim) the
   * reserved space. Layer docks are usually on all workspaces, so if the
   * window has no single workspace, invalidate them all. */
  invalidate_work_areas_for_window (window);
}

static gboolean
destroy_closed_layer_window_idle (gpointer data)
{
  DestroyWindowIdleData *idle_data = data;
  MetaWaylandLayerSurface *layer_surface =
    g_weak_ref_get (&idle_data->layer_surface);

  if (!layer_surface)
    return G_SOURCE_REMOVE;

  layer_surface->destroy_window_idle_id = 0;

  if (layer_surface->closed)
    meta_wayland_shell_surface_destroy_window (
      META_WAYLAND_SHELL_SURFACE (layer_surface));

  g_object_unref (layer_surface);

  return G_SOURCE_REMOVE;
}

static void
destroy_window_idle_data_free (gpointer data)
{
  DestroyWindowIdleData *idle_data = data;

  g_weak_ref_clear (&idle_data->layer_surface);
  g_free (idle_data);
}

static void
queue_closed_layer_window_destroy (MetaWaylandLayerSurface *layer_surface)
{
  DestroyWindowIdleData *idle_data;

  if (layer_surface->destroy_window_idle_id)
    return;

  idle_data = g_new0 (DestroyWindowIdleData, 1);
  g_weak_ref_init (&idle_data->layer_surface, layer_surface);

  /* Do not keep the role alive just for this timeout. MetaWaylandSurfaceRole
   * keeps only a weak surface pointer, so extending the role after wl_surface
   * teardown can make the shell-surface dispose path dereference a finalized
   * surface. */
  layer_surface->destroy_window_idle_id =
    g_timeout_add_full (G_PRIORITY_DEFAULT,
                        CLOSED_LAYER_WINDOW_DESTROY_DELAY_MS,
                        destroy_closed_layer_window_idle,
                        idle_data,
                        destroy_window_idle_data_free);
}

static void
disconnect_layer_surface_output (MetaWaylandLayerSurface *layer_surface)
{
  if (layer_surface->output && layer_surface->output_destroyed_handler_id)
    g_clear_signal_handler (&layer_surface->output_destroyed_handler_id,
                            layer_surface->output);

  layer_surface->output_destroyed_handler_id = 0;
  layer_surface->output = NULL;
}

static void
close_layer_surface (MetaWaylandLayerSurface *layer_surface,
                     gboolean                 invalidate_work_areas)
{
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (layer_surface));
  MetaWindow *window;

  if (layer_surface->closed)
    return;

  layer_surface->closed = TRUE;

  window = meta_wayland_surface_get_window (surface);
  if (window)
    {
      clear_exclusive_zone_struts (window, invalidate_work_areas);
      queue_closed_layer_window_destroy (layer_surface);
    }

  if (layer_surface->resource)
    zwlr_layer_surface_v1_send_closed (layer_surface->resource);
}

static void
layer_surface_handle_output_destroyed (MetaWaylandOutput       *output,
                                       MetaWaylandLayerSurface *layer_surface)
{
  (void) output;

  layer_surface->output_destroyed_handler_id = 0;
  layer_surface->output = NULL;

  /* This signal is emitted during MetaMonitorManager::monitors-changing, before
   * Mutter has rebuilt native stage views for the new monitor set. Invalidating
   * work areas here queues every window and can schedule a frame on the removed
   * virtual CRTC. The following monitors-changed-internal handler reloads work
   * areas after the backend has settled, so just drop our stale strut now. */
  close_layer_surface (layer_surface, FALSE);
}

static void
move_resize_layer_window (MetaWindow   *window,
                          MtkRectangle  geom)
{
  MetaMoveResizeFlags flags =
    (META_MOVE_RESIZE_WAYLAND_FINISH_MOVE_RESIZE |
     META_MOVE_RESIZE_MOVE_ACTION |
     META_MOVE_RESIZE_RESIZE_ACTION |
     META_MOVE_RESIZE_FORCE_UPDATE_MONITOR);

  /* Layer-shell has its own configure/ack handshake. Once the client commits the
   * matching buffer, apply the compositor-chosen rect directly instead of
   * feeding it through normal toplevel placement/constraining. */
  meta_window_move_resize_internal (window, flags, META_PLACE_FLAG_NONE,
                                    geom, NULL);
  window->unconstrained_rect = geom;
  window->unconstrained_rect_valid = TRUE;
  window->placed = TRUE;
}

/* ---- zwlr_layer_surface_v1 requests ------------------------------ */

static void
layer_surface_set_size (struct wl_client   *client,
                        struct wl_resource *resource,
                        uint32_t            width,
                        uint32_t            height)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  if (layer_surface->closed)
    return;

  layer_surface->pending.desired_width = width;
  layer_surface->pending.desired_height = height;
  layer_surface->has_pending_state = TRUE;
}

static void
layer_surface_set_anchor (struct wl_client   *client,
                          struct wl_resource *resource,
                          uint32_t            anchor)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  if (layer_surface->closed)
    return;

  if (anchor & ~META_LAYER_SURFACE_ANCHOR_MASK)
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_ANCHOR,
                              "invalid anchor bitfield 0x%x", anchor);
      return;
    }

  layer_surface->pending.anchor = anchor;
  layer_surface->has_pending_state = TRUE;
}

static void
layer_surface_set_exclusive_zone (struct wl_client   *client,
                                  struct wl_resource *resource,
                                  int32_t             zone)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  if (layer_surface->closed)
    return;

  layer_surface->pending.exclusive_zone = zone;
  layer_surface->has_pending_state = TRUE;
}

static void
layer_surface_set_margin (struct wl_client   *client,
                          struct wl_resource *resource,
                          int32_t             top,
                          int32_t             right,
                          int32_t             bottom,
                          int32_t             left)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  if (layer_surface->closed)
    return;

  layer_surface->pending.margin_top = top;
  layer_surface->pending.margin_right = right;
  layer_surface->pending.margin_bottom = bottom;
  layer_surface->pending.margin_left = left;
  layer_surface->has_pending_state = TRUE;
}

static void
layer_surface_set_keyboard_interactivity (struct wl_client   *client,
                                          struct wl_resource *resource,
                                          uint32_t            keyboard_interactivity)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  if (layer_surface->closed)
    return;

  if (keyboard_interactivity >
      ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_ON_DEMAND)
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_KEYBOARD_INTERACTIVITY,
                              "invalid keyboard interactivity %u",
                              keyboard_interactivity);
      return;
    }

  layer_surface->pending.keyboard_interactivity = keyboard_interactivity;
  layer_surface->has_pending_state = TRUE;
}

static void
layer_surface_get_popup (struct wl_client   *client,
                         struct wl_resource *resource,
                         struct wl_resource *popup_resource)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (layer_surface));

  if (layer_surface->closed)
    return;

  if (!meta_wayland_xdg_popup_set_parent_surface (popup_resource, surface))
    wl_resource_post_error (resource,
                            ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SURFACE_STATE,
                            "xdg_popup is not ready for a layer-surface parent");
}

static void
layer_surface_ack_configure (struct wl_client   *client,
                             struct wl_resource *resource,
                             uint32_t            serial)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);
  GList *serial_link;

  if (layer_surface->closed)
    return;

  serial_link = g_queue_find (&layer_surface->configure_serials,
                              GUINT_TO_POINTER (serial));
  if (!layer_surface->configured || serial == 0 || !serial_link)
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SURFACE_STATE,
                              "invalid configure serial %u", serial);
      return;
    }

  while (!g_queue_is_empty (&layer_surface->configure_serials))
    {
      uint32_t queued_serial =
        GPOINTER_TO_UINT (g_queue_pop_head (&layer_surface->configure_serials));

      if (queued_serial == serial)
        break;
    }

  layer_surface->acked_configure_serial = serial;
  /* Per wlr-layer-shell, attaching a buffer is permitted once the client has
   * acked *any* configure — it may render against an earlier configure while a
   * newer one is still in flight (e.g. a configure storm during a monitor
   * hotplug/resize). Gating this on the *latest* serial spuriously rejected
   * valid buffers mid-storm, which then tripped the frame-callback assertion in
   * apply_state. Once acked, the surface stays acked. */
  layer_surface->has_acked_configure = TRUE;
}

static void
layer_surface_destroy (struct wl_client   *client,
                       struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static void
layer_surface_set_layer (struct wl_client   *client,
                         struct wl_resource *resource,
                         uint32_t            layer)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  if (layer_surface->closed)
    return;

  if (layer > ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY)
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SURFACE_STATE,
                              "invalid layer %u", layer);
      return;
    }

  layer_surface->pending.layer = layer;
  layer_surface->has_pending_state = TRUE;
}

static void
layer_surface_set_exclusive_edge (struct wl_client   *client,
                                  struct wl_resource *resource,
                                  uint32_t            edge)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  if (layer_surface->closed)
    return;

  if (!is_single_anchor_edge (edge))
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_EXCLUSIVE_EDGE,
                              "exclusive edge 0x%x is not a single edge", edge);
      return;
    }

  layer_surface->pending.exclusive_edge = edge;
  layer_surface->pending.has_exclusive_edge = TRUE;
  layer_surface->has_pending_state = TRUE;
}

static const struct zwlr_layer_surface_v1_interface layer_surface_implementation = {
  .set_size = layer_surface_set_size,
  .set_anchor = layer_surface_set_anchor,
  .set_exclusive_zone = layer_surface_set_exclusive_zone,
  .set_margin = layer_surface_set_margin,
  .set_keyboard_interactivity = layer_surface_set_keyboard_interactivity,
  .get_popup = layer_surface_get_popup,
  .ack_configure = layer_surface_ack_configure,
  .destroy = layer_surface_destroy,
  .set_layer = layer_surface_set_layer,
  .set_exclusive_edge = layer_surface_set_exclusive_edge,
};

static void
layer_surface_resource_destroy (struct wl_resource *resource)
{
  MetaWaylandLayerSurface *layer_surface = wl_resource_get_user_data (resource);

  wl_resource_set_user_data (resource, NULL);

  if (!layer_surface)
    return;

  if (layer_surface->resource == resource)
    layer_surface->resource = NULL;

  disconnect_layer_surface_output (layer_surface);

  {
    MetaWaylandSurface *surface =
      meta_wayland_surface_role_get_surface (META_WAYLAND_SURFACE_ROLE (layer_surface));
    MetaWindow *window = surface ? meta_wayland_surface_get_window (surface) : NULL;

    if (window)
      {
        g_object_set_data (G_OBJECT (window), "gnoblin-layer-surface", NULL);
        g_object_set_data (G_OBJECT (window), "gnoblin-layer-dismiss", NULL);
      }
  }

  if (layer_surface->destroy_window_idle_id)
    g_clear_handle_id (&layer_surface->destroy_window_idle_id, g_source_remove);

  meta_wayland_shell_surface_destroy_window (META_WAYLAND_SHELL_SURFACE (layer_surface));
}

static void
focus_exclusive_layer_surface (MetaWaylandLayerSurface *layer_surface,
                               MetaWaylandSurface      *surface,
                               MetaWindow              *window)
{
  if (layer_surface->current.keyboard_interactivity !=
      ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE)
    return;

  {
    MetaDisplay *display = display_from_surface (surface);
    MetaWaylandSeat *seat = surface->compositor->seat;

    meta_window_focus (window, meta_display_get_current_time (display));
    if (seat && meta_wayland_seat_has_keyboard (seat))
      meta_wayland_keyboard_set_focus (seat->keyboard, surface);
  }
}

/* ---- MetaWaylandLayerSurface role ------------------------------- */

static void
meta_wayland_layer_surface_apply_state (MetaWaylandSurfaceRole  *surface_role,
                                        MetaWaylandSurfaceState *pending)
{
  MetaWaylandLayerSurface *layer_surface =
    META_WAYLAND_LAYER_SURFACE (surface_role);
  MetaWaylandActorSurface *actor_surface =
    META_WAYLAND_ACTOR_SURFACE (surface_role);
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (surface_role);
  MetaWaylandSurfaceRoleClass *surface_role_class;
  MetaWindow *window;
  gboolean unmapping;
  gboolean state_changed = FALSE;

  window = meta_wayland_surface_get_window (surface);
  if (!window)
    {
      meta_wayland_actor_surface_queue_frame_callbacks (actor_surface, pending);
      return;
    }

  if (layer_surface->closed)
    {
      meta_wayland_actor_surface_queue_frame_callbacks (actor_surface, pending);
      return;
    }

  if (pending->newly_attached && pending->buffer && !layer_surface->configured)
    {
      /* Consume the pending frame callbacks before bailing: mutter asserts that
       * apply_state always empties state->frame_callback_list, and posting an
       * error here does not tear the surface down synchronously. */
      meta_wayland_actor_surface_queue_frame_callbacks (actor_surface, pending);
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SURFACE_STATE,
                              "cannot attach a buffer before first configure");
      return;
    }

  if (pending->newly_attached && pending->buffer &&
      !layer_surface->has_acked_configure)
    {
      meta_wayland_actor_surface_queue_frame_callbacks (actor_surface, pending);
      wl_resource_post_error (layer_surface->resource,
                              ZWLR_LAYER_SURFACE_V1_ERROR_INVALID_SURFACE_STATE,
                              "cannot attach a buffer before ack_configure");
      return;
    }

  unmapping = pending->newly_attached && !pending->buffer;
  if (unmapping)
    {
      reset_layer_surface_state (layer_surface);
      update_exclusive_zone_struts (layer_surface, window,
                                    get_monitor_layout (layer_surface));
    }

  if (layer_surface->has_pending_state)
    {
      if (!validate_layer_surface_state (layer_surface,
                                         &layer_surface->pending))
        {
          meta_wayland_actor_surface_queue_frame_callbacks (actor_surface,
                                                            pending);
          return;
        }

      state_changed =
        !layer_surface_state_equal (&layer_surface->pending,
                                    &layer_surface->current);
      layer_surface->current = layer_surface->pending;
      layer_surface->has_pending_state = FALSE;
    }

  surface_role_class =
    META_WAYLAND_SURFACE_ROLE_CLASS (meta_wayland_layer_surface_parent_class);
  surface_role_class->apply_state (surface_role, pending);

  if (unmapping)
    return;

  if (!validate_layer_surface_state (layer_surface, &layer_surface->current))
    return;

  focus_exclusive_layer_surface (layer_surface, surface, window);

  {
    MtkRectangle mon = get_monitor_layout (layer_surface);
    MtkRectangle geom = calculate_geometry (layer_surface, mon);

    if (!layer_surface->configured ||
        state_changed ||
        geom.width != layer_surface->last_sent_width ||
        geom.height != layer_surface->last_sent_height)
      send_configure (layer_surface, geom.width, geom.height);
  }
}

static void
meta_wayland_layer_surface_post_apply_state (MetaWaylandSurfaceRole  *surface_role,
                                             MetaWaylandSurfaceState *pending)
{
  MetaWaylandLayerSurface *layer_surface =
    META_WAYLAND_LAYER_SURFACE (surface_role);
  MetaWaylandSurface *surface =
    meta_wayland_surface_role_get_surface (surface_role);
  MetaWaylandSurfaceRoleClass *surface_role_class;
  MetaWindow *window;

  window = meta_wayland_surface_get_window (surface);
  if (!window)
    return;

  if (layer_surface->closed)
    return;

  surface_role_class =
    META_WAYLAND_SURFACE_ROLE_CLASS (meta_wayland_layer_surface_parent_class);
  if (surface_role_class->post_apply_state)
    surface_role_class->post_apply_state (surface_role, pending);

  if (meta_wayland_surface_get_buffer (surface))
    {
      MtkRectangle mon = get_monitor_layout (layer_surface);
      MtkRectangle geom = calculate_geometry (layer_surface, mon);

      apply_window_type_and_layer (layer_surface, window);
      window->input = TRUE;
      move_resize_layer_window (window, geom);
      update_exclusive_zone_struts (layer_surface, window, mon);
      meta_window_update_visibility (window);
      {
        MetaContext *context =
          meta_wayland_compositor_get_context (surface->compositor);
        MetaBackend *backend = meta_context_get_backend (context);
        ClutterActor *stage = meta_backend_get_stage (backend);

        if (stage)
          clutter_stage_schedule_update (CLUTTER_STAGE (stage));
      }

      if (layer_surface->current.keyboard_interactivity ==
          ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE)
        focus_exclusive_layer_surface (layer_surface, surface, window);
    }
}

static MetaWaylandSurface *
meta_wayland_layer_surface_get_toplevel (MetaWaylandSurfaceRole *surface_role)
{
  return meta_wayland_surface_role_get_surface (surface_role);
}

static void
meta_wayland_layer_surface_configure (MetaWaylandShellSurface        *shell_surface,
                                      MetaWaylandWindowConfiguration *configuration)
{
  MetaWaylandLayerSurface *layer_surface =
    META_WAYLAND_LAYER_SURFACE (shell_surface);
  MtkRectangle mon;
  MtkRectangle geom;

  if (!layer_surface->resource)
    return;

  if (layer_surface->closed)
    return;

  mon = get_monitor_layout (layer_surface);
  geom = calculate_geometry (layer_surface, mon);

  if (!layer_surface->configured ||
      geom.width != layer_surface->last_sent_width ||
      geom.height != layer_surface->last_sent_height)
    send_configure (layer_surface, geom.width, geom.height);
}

static void
meta_wayland_layer_surface_managed (MetaWaylandShellSurface *shell_surface,
                                    MetaWindow              *window)
{
}

static void
meta_wayland_layer_surface_ping (MetaWaylandShellSurface *shell_surface,
                                 uint32_t                 serial)
{
  /* wlr-layer-shell has no ping/pong request pair. MetaWindowWayland is taught
   * not to ping layer-shell windows, but keep this vfunc non-NULL as a guard
   * against future direct shell-surface ping callers. */
}

static void
meta_wayland_layer_surface_close (MetaWaylandShellSurface *shell_surface)
{
  MetaWaylandLayerSurface *layer_surface =
    META_WAYLAND_LAYER_SURFACE (shell_surface);

  disconnect_layer_surface_output (layer_surface);
  close_layer_surface (layer_surface, TRUE);
}

static void
meta_wayland_layer_surface_finalize (GObject *object)
{
  MetaWaylandLayerSurface *layer_surface = META_WAYLAND_LAYER_SURFACE (object);

  disconnect_layer_surface_output (layer_surface);
  g_clear_pointer (&layer_surface->resource, wl_resource_destroy);
  if (layer_surface->destroy_window_idle_id)
    g_clear_handle_id (&layer_surface->destroy_window_idle_id, g_source_remove);
  g_queue_clear (&layer_surface->configure_serials);
  g_clear_pointer (&layer_surface->namespace, g_free);

  G_OBJECT_CLASS (meta_wayland_layer_surface_parent_class)->finalize (object);
}

static void
meta_wayland_layer_surface_init (MetaWaylandLayerSurface *layer_surface)
{
  g_queue_init (&layer_surface->configure_serials);
}

static void
meta_wayland_layer_surface_class_init (MetaWaylandLayerSurfaceClass *klass)
{
  GObjectClass *object_class = G_OBJECT_CLASS (klass);
  MetaWaylandSurfaceRoleClass *surface_role_class =
    META_WAYLAND_SURFACE_ROLE_CLASS (klass);
  MetaWaylandShellSurfaceClass *shell_surface_class =
    META_WAYLAND_SHELL_SURFACE_CLASS (klass);

  object_class->finalize = meta_wayland_layer_surface_finalize;

  surface_role_class->apply_state = meta_wayland_layer_surface_apply_state;
  surface_role_class->post_apply_state =
    meta_wayland_layer_surface_post_apply_state;
  surface_role_class->get_toplevel = meta_wayland_layer_surface_get_toplevel;

  shell_surface_class->configure = meta_wayland_layer_surface_configure;
  shell_surface_class->managed = meta_wayland_layer_surface_managed;
  shell_surface_class->ping = meta_wayland_layer_surface_ping;
  shell_surface_class->close = meta_wayland_layer_surface_close;
}

/* ---- zwlr_layer_shell_v1 requests ------------------------------- */

static void
layer_shell_get_layer_surface (struct wl_client   *client,
                               struct wl_resource *resource,
                               uint32_t            id,
                               struct wl_resource *surface_resource,
                               struct wl_resource *output_resource,
                               uint32_t            layer,
                               const char         *namespace)
{
  MetaWaylandSurface *surface = wl_resource_get_user_data (surface_resource);
  MetaWaylandLayerSurface *layer_surface;
  MetaWindow *window;
  MetaWaylandSurfaceState *pending;

  if (layer > ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY)
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SHELL_V1_ERROR_INVALID_LAYER,
                              "invalid layer %u", layer);
      return;
    }

  pending = meta_wayland_surface_get_pending_state (surface);
  if ((pending->newly_attached && pending->buffer) ||
      meta_wayland_surface_get_buffer (surface) ||
      meta_wayland_surface_has_initial_commit (surface))
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SHELL_V1_ERROR_ALREADY_CONSTRUCTED,
                              "wl_surface@%d already has a buffer attached or committed",
                              wl_resource_get_id (surface_resource));
      return;
    }

  if (!meta_wayland_surface_assign_role (surface,
                                         META_TYPE_WAYLAND_LAYER_SURFACE,
                                         NULL))
    {
      wl_resource_post_error (resource,
                              ZWLR_LAYER_SHELL_V1_ERROR_ROLE,
                              "wl_surface@%d already has a different role",
                              wl_resource_get_id (surface_resource));
      return;
    }

  layer_surface = META_WAYLAND_LAYER_SURFACE (surface->role);
  layer_surface->namespace = g_strdup (namespace);
  layer_surface->output =
    output_resource ? wl_resource_get_user_data (output_resource) : NULL;
  layer_surface->initial_layer = layer;
  layer_surface->pending.layer = layer;
  layer_surface->current.layer = layer;
  layer_surface->closed = FALSE;
  layer_surface->output_destroyed_handler_id = 0;
  layer_surface->destroy_window_idle_id = 0;

  if (layer_surface->output)
    {
      layer_surface->output_destroyed_handler_id =
        g_signal_connect (layer_surface->output, "output-destroyed",
                          G_CALLBACK (layer_surface_handle_output_destroyed),
                          layer_surface);
    }

  layer_surface->resource =
    wl_resource_create (client, &zwlr_layer_surface_v1_interface,
                        wl_resource_get_version (resource), id);
  wl_resource_set_implementation (layer_surface->resource,
                                  &layer_surface_implementation,
                                  layer_surface,
                                  layer_surface_resource_destroy);

  window = meta_window_wayland_new (display_from_surface (surface), surface);
  if (layer <= ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM)
    window->type = META_WINDOW_DESKTOP;
  else
    window->type = META_WINDOW_DOCK;
  /* Stash the wlr-layer-shell namespace on the MetaWindow so gnoblin's window
   * rules can target layer surfaces (a `layer=<namespace>` matcher) and apply
   * compositor-managed rounding/border/blur to the shell's own panels. The
   * namespace is otherwise private to the layer-surface object. */
  if (layer_surface->namespace)
    g_object_set_data_full (G_OBJECT (window), "gnoblin-layer-namespace",
                            g_strdup (layer_surface->namespace), g_free);
  g_object_set_data (G_OBJECT (window), "gnoblin-layer-surface", layer_surface);
  g_object_set_data (G_OBJECT (window), "gnoblin-layer-dismiss",
                     (gpointer) gnoblin_layer_dismiss_trampoline);
  apply_window_type_and_layer (layer_surface, window);
  meta_wayland_shell_surface_set_window (META_WAYLAND_SHELL_SURFACE (layer_surface),
                                         window);
}

static void
layer_shell_destroy (struct wl_client   *client,
                     struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct zwlr_layer_shell_v1_interface layer_shell_implementation = {
  .get_layer_surface = layer_shell_get_layer_surface,
  .destroy = layer_shell_destroy,
};

static void
bind_layer_shell (struct wl_client *client,
                  void             *data,
                  uint32_t          version,
                  uint32_t          id)
{
  struct wl_resource *resource;

  resource = wl_resource_create (client, &zwlr_layer_shell_v1_interface,
                                 version, id);
  wl_resource_set_implementation (resource, &layer_shell_implementation,
                                  data, NULL);
}

void
meta_wayland_init_layer_shell (MetaWaylandCompositor *compositor)
{
  if (!gnoblin_config_protocol_enabled ("wlr-layer-shell"))
    {
      g_message ("Gnoblin wlr-layer-shell protocol disabled by settings");
      return;
    }

  if (wl_global_create (compositor->wayland_display,
                        &zwlr_layer_shell_v1_interface,
                        META_WLR_LAYER_SHELL_V1_VERSION,
                        compositor, bind_layer_shell) == NULL)
    g_error ("Failed to register a global zwlr_layer_shell_v1 object");
}
