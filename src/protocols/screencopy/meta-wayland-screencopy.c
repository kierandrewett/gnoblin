/*
 * gnoblin: wlr-screencopy-unstable-v1 support for mutter.
 *
 * Implements the wlroots screencopy protocol for wl_shm buffers. This is
 * intentionally scoped to output/region capture into SHM so tools such as grim
 * can capture Gnoblin's nested Devkit compositor output, including layer-shell
 * surfaces, without going through the host compositor.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-screencopy.h"

#include <errno.h>
#include <gio/gio.h>
#include <math.h>
#include <time.h>
#include <wayland-server.h>

#include "backends/meta-backend-private.h"
#include "backends/meta-logical-monitor-private.h"
#include "backends/meta-monitor-private.h"
#include "clutter/clutter.h"
#include "meta/meta-backend.h"
#include "wayland/meta-wayland-buffer.h"
#include "wayland/meta-wayland-outputs.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/gnoblin-config.h"

#include "wlr-screencopy-unstable-v1-server-protocol.h"

#define META_WLR_SCREENCOPY_VERSION 3

typedef struct _MetaWaylandScreencopyFrame
{
  MetaWaylandCompositor *compositor;
  struct wl_resource *resource;
  MtkRectangle rect;
  int buffer_width;
  int buffer_height;
  int buffer_stride;
  float scale;
  gboolean overlay_cursor;
  gboolean with_damage;
  gboolean copied;
} MetaWaylandScreencopyFrame;

static void
destroy_frame (struct wl_resource *resource)
{
  MetaWaylandScreencopyFrame *frame = wl_resource_get_user_data (resource);

  g_free (frame);
}

static void
send_failed (MetaWaylandScreencopyFrame *frame)
{
  if (frame->resource)
    zwlr_screencopy_frame_v1_send_failed (frame->resource);
}

static void
frame_destroy (struct wl_client   *client,
               struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static gboolean
calculate_buffer_geometry (const MtkRectangle *rect,
                           float               scale,
                           int                *width,
                           int                *height,
                           int                *stride)
{
  double scaled_width;
  double scaled_height;
  gsize buffer_size;

  if (!isfinite (scale) || scale <= 0.0f)
    return FALSE;

  scaled_width = (double) rect->width * scale;
  scaled_height = (double) rect->height * scale;
  if (scaled_width <= 0.0 || scaled_width > G_MAXINT ||
      scaled_height <= 0.0 || scaled_height > G_MAXINT)
    return FALSE;

  *width = (int) round (scaled_width);
  *height = (int) round (scaled_height);
  if (*width <= 0 || *height <= 0 || *width > G_MAXINT / 4)
    return FALSE;

  *stride = *width * 4;
  if ((gsize) *stride > G_MAXSIZE / (gsize) *height)
    return FALSE;

  buffer_size = (gsize) *stride * (gsize) *height;
  return buffer_size > 0;
}

static gboolean
validate_shm_buffer (MetaWaylandScreencopyFrame *frame,
                     struct wl_shm_buffer       *shm_buffer)
{
  int stride = wl_shm_buffer_get_stride (shm_buffer);

  if (wl_shm_buffer_get_width (shm_buffer) != frame->buffer_width ||
      wl_shm_buffer_get_height (shm_buffer) != frame->buffer_height ||
      stride != frame->buffer_stride)
    return FALSE;

  switch (wl_shm_buffer_get_format (shm_buffer))
    {
    case WL_SHM_FORMAT_ARGB8888:
    case WL_SHM_FORMAT_XRGB8888:
      return TRUE;
    default:
      return FALSE;
    }
}

static void
copy_frame_to_buffer (struct wl_client   *client,
                      struct wl_resource *resource,
                      struct wl_resource *buffer_resource,
                      gboolean            with_damage)
{
  MetaWaylandScreencopyFrame *frame = wl_resource_get_user_data (resource);
  struct wl_shm_buffer *shm_buffer;
  MetaContext *context;
  MetaBackend *backend;
  ClutterStage *stage;
  ClutterPaintFlag paint_flags = CLUTTER_PAINT_FLAG_CLEAR;
  g_autoptr (GError) error = NULL;
  struct timespec ts;
  uint8_t *data;

  if (frame->copied)
    {
      wl_resource_post_error (resource,
                              ZWLR_SCREENCOPY_FRAME_V1_ERROR_ALREADY_USED,
                              "screencopy frame already used");
      return;
    }

  frame->copied = TRUE;
  frame->with_damage = with_damage;

  shm_buffer = wl_shm_buffer_get (buffer_resource);
  if (!shm_buffer || !validate_shm_buffer (frame, shm_buffer))
    {
      wl_resource_post_error (resource,
                              ZWLR_SCREENCOPY_FRAME_V1_ERROR_INVALID_BUFFER,
                              "invalid screencopy wl_shm buffer");
      return;
    }

  context = meta_wayland_compositor_get_context (frame->compositor);
  backend = meta_context_get_backend (context);
  stage = CLUTTER_STAGE (meta_backend_get_stage (backend));

  if (frame->overlay_cursor)
    paint_flags |= CLUTTER_PAINT_FLAG_FORCE_CURSORS;
  else
    paint_flags |= CLUTTER_PAINT_FLAG_NO_CURSORS;

  wl_shm_buffer_begin_access (shm_buffer);
  data = wl_shm_buffer_get_data (shm_buffer);

  if (!clutter_stage_paint_to_buffer (stage,
                                      &frame->rect,
                                      frame->scale,
                                      data,
                                      wl_shm_buffer_get_stride (shm_buffer),
                                      COGL_PIXEL_FORMAT_CAIRO_ARGB32_COMPAT,
                                      paint_flags,
                                      &error))
    {
      wl_shm_buffer_end_access (shm_buffer);
      g_warning ("screencopy failed: %s", error->message);
      send_failed (frame);
      return;
    }

  wl_shm_buffer_end_access (shm_buffer);

  if (frame->with_damage)
    {
      zwlr_screencopy_frame_v1_send_damage (resource,
                                            0,
                                            0,
                                            frame->buffer_width,
                                            frame->buffer_height);
    }

  zwlr_screencopy_frame_v1_send_flags (resource, 0);

  if (clock_gettime (CLOCK_MONOTONIC, &ts) != 0)
    {
      ts.tv_sec = 0;
      ts.tv_nsec = 0;
    }

  zwlr_screencopy_frame_v1_send_ready (resource,
                                       (uint32_t) (((uint64_t) ts.tv_sec) >> 32),
                                       (uint32_t) ts.tv_sec,
                                       (uint32_t) ts.tv_nsec);
}

static void
frame_copy (struct wl_client   *client,
            struct wl_resource *resource,
            struct wl_resource *buffer_resource)
{
  copy_frame_to_buffer (client, resource, buffer_resource, FALSE);
}

static void
frame_copy_with_damage (struct wl_client   *client,
                        struct wl_resource *resource,
                        struct wl_resource *buffer_resource)
{
  copy_frame_to_buffer (client, resource, buffer_resource, TRUE);
}

static const struct zwlr_screencopy_frame_v1_interface frame_interface = {
  frame_copy,
  frame_destroy,
  frame_copy_with_damage,
};

static float
scale_for_monitor (MetaBackend        *backend,
                   MetaLogicalMonitor *logical_monitor)
{
  if (meta_backend_is_stage_views_scaled (backend))
    return meta_logical_monitor_get_scale (logical_monitor);

  return 1.0;
}

static gboolean
clip_region_to_monitor (const MtkRectangle *monitor_rect,
                        int32_t             x,
                        int32_t             y,
                        int32_t             width,
                        int32_t             height,
                        MtkRectangle       *clipped)
{
  int64_t monitor_x1 = monitor_rect->x;
  int64_t monitor_y1 = monitor_rect->y;
  int64_t monitor_x2 = monitor_x1 + monitor_rect->width;
  int64_t monitor_y2 = monitor_y1 + monitor_rect->height;
  int64_t requested_x1;
  int64_t requested_y1;
  int64_t requested_x2;
  int64_t requested_y2;
  int64_t clipped_x1;
  int64_t clipped_y1;
  int64_t clipped_x2;
  int64_t clipped_y2;

  if (monitor_rect->width <= 0 || monitor_rect->height <= 0 ||
      monitor_x2 > G_MAXINT || monitor_y2 > G_MAXINT ||
      width <= 0 || height <= 0)
    return FALSE;

  requested_x1 = monitor_x1 + x;
  requested_y1 = monitor_y1 + y;
  requested_x2 = requested_x1 + width;
  requested_y2 = requested_y1 + height;
  clipped_x1 = CLAMP (requested_x1, monitor_x1, monitor_x2);
  clipped_y1 = CLAMP (requested_y1, monitor_y1, monitor_y2);
  clipped_x2 = CLAMP (requested_x2, monitor_x1, monitor_x2);
  clipped_y2 = CLAMP (requested_y2, monitor_y1, monitor_y2);

  if (clipped_x2 <= clipped_x1 || clipped_y2 <= clipped_y1)
    return FALSE;

  *clipped = (MtkRectangle) {
    .x = (int) clipped_x1,
    .y = (int) clipped_y1,
    .width = (int) (clipped_x2 - clipped_x1),
    .height = (int) (clipped_y2 - clipped_y1),
  };
  return TRUE;
}


static void
create_frame (struct wl_client   *client,
              struct wl_resource *manager_resource,
              uint32_t            frame_id,
              gboolean            overlay_cursor,
              struct wl_resource *output_resource,
              gboolean            region,
              int32_t             x,
              int32_t             y,
              int32_t             width,
              int32_t             height)
{
  MetaWaylandCompositor *compositor = wl_resource_get_user_data (manager_resource);
  MetaWaylandOutput *wayland_output = wl_resource_get_user_data (output_resource);
  g_autofree MetaWaylandScreencopyFrame *frame = NULL;
  MetaContext *context;
  MetaBackend *backend;
  MetaMonitor *monitor;
  MetaLogicalMonitor *logical_monitor;
  MtkRectangle monitor_rect;
  int version = wl_resource_get_version (manager_resource);

  frame = g_new0 (MetaWaylandScreencopyFrame, 1);
  frame->compositor = compositor;
  frame->overlay_cursor = overlay_cursor;

  frame->resource =
    wl_resource_create (client,
                        &zwlr_screencopy_frame_v1_interface,
                        version,
                        frame_id);
  if (!frame->resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (frame->resource,
                                  &frame_interface,
                                  frame,
                                  destroy_frame);

  monitor = wayland_output ? meta_wayland_output_get_monitor (wayland_output) : NULL;
  logical_monitor = monitor ? meta_monitor_get_logical_monitor (monitor) : NULL;
  if (!logical_monitor)
    {
      send_failed (frame);
      g_steal_pointer (&frame);
      return;
    }

  monitor_rect = meta_logical_monitor_get_layout (logical_monitor);
  if (region)
    {
      if (!clip_region_to_monitor (&monitor_rect,
                                   x,
                                   y,
                                   width,
                                   height,
                                   &frame->rect))
        {
          send_failed (frame);
          g_steal_pointer (&frame);
          return;
        }
    }
  else
    frame->rect = monitor_rect;

  if (frame->rect.width <= 0 || frame->rect.height <= 0)
    {
      send_failed (frame);
      g_steal_pointer (&frame);
      return;
    }

  context = meta_wayland_compositor_get_context (compositor);
  backend = meta_context_get_backend (context);
  frame->scale = scale_for_monitor (backend, logical_monitor);

  if (!calculate_buffer_geometry (&frame->rect,
                                  frame->scale,
                                  &frame->buffer_width,
                                  &frame->buffer_height,
                                  &frame->buffer_stride))
    {
      send_failed (frame);
      g_steal_pointer (&frame);
      return;
    }

  zwlr_screencopy_frame_v1_send_buffer (frame->resource,
                                        WL_SHM_FORMAT_ARGB8888,
                                        frame->buffer_width,
                                        frame->buffer_height,
                                        frame->buffer_stride);
  if (version >= ZWLR_SCREENCOPY_FRAME_V1_BUFFER_DONE_SINCE_VERSION)
    zwlr_screencopy_frame_v1_send_buffer_done (frame->resource);

  g_steal_pointer (&frame);
}

static void
manager_capture_output (struct wl_client   *client,
                        struct wl_resource *resource,
                        uint32_t            frame,
                        int32_t             overlay_cursor,
                        struct wl_resource *output)
{
  create_frame (client, resource, frame, overlay_cursor, output,
                FALSE, 0, 0, 0, 0);
}

static void
manager_capture_output_region (struct wl_client   *client,
                               struct wl_resource *resource,
                               uint32_t            frame,
                               int32_t             overlay_cursor,
                               struct wl_resource *output,
                               int32_t             x,
                               int32_t             y,
                               int32_t             width,
                               int32_t             height)
{
  create_frame (client, resource, frame, overlay_cursor, output,
                TRUE, x, y, width, height);
}

static void
manager_destroy (struct wl_client   *client,
                 struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct zwlr_screencopy_manager_v1_interface manager_interface = {
  manager_capture_output,
  manager_capture_output_region,
  manager_destroy,
};

static void
bind_screencopy (struct wl_client *client,
                 void             *data,
                 uint32_t          version,
                 uint32_t          id)
{
  MetaWaylandCompositor *compositor = data;
  struct wl_resource *resource;

  resource = wl_resource_create (client,
                                 &zwlr_screencopy_manager_v1_interface,
                                 version,
                                 id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (resource,
                                  &manager_interface,
                                  compositor,
                                  NULL);
}

void
meta_wayland_init_screencopy (MetaWaylandCompositor *compositor)
{
  if (!gnoblin_config_protocol_enabled ("wlr-screencopy"))
    {
      g_message ("Gnoblin wlr-screencopy protocol disabled by settings");
      return;
    }

  if (!wl_global_create (compositor->wayland_display,
                         &zwlr_screencopy_manager_v1_interface,
                         META_WLR_SCREENCOPY_VERSION,
                         compositor,
                         bind_screencopy))
    g_error ("Failed to register wlr-screencopy global");
}
