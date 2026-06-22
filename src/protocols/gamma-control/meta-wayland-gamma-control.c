/*
 * gnoblin: wlr-gamma-control-unstable-v1 support for mutter.
 *
 * Implements the wlroots gamma-control protocol on top of mutter's per-CRTC
 * gamma LUT API (meta_crtc_set_gamma_lut). This lets colour-temperature daemons
 * such as wlsunset and gammastep drive a night-light effect in a Gnoblin
 * session without GNOME's built-in night light.
 *
 * Each output may have at most one gamma control; creating a second one fails
 * the first. The original gamma table is restored when the control is
 * destroyed. Gamma controls are failed (not restored) when the monitor layout
 * changes, since the underlying CRTCs may no longer exist.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-gamma-control.h"

#include <errno.h>
#include <gio/gio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#include <wayland-server.h>

#include "backends/meta-backend-private.h"
#include "backends/meta-crtc.h"
#include "backends/meta-monitor-private.h"
#include "backends/meta-output.h"
#include "meta/meta-backend.h"
#include "meta/meta-context.h"
#include "meta/meta-monitor-manager.h"
#include "wayland/meta-wayland-outputs.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/gnoblin-config.h"

#include "wlr-gamma-control-unstable-v1-server-protocol.h"

#define META_WLR_GAMMA_CONTROL_VERSION 1

typedef struct _MetaWaylandGammaContext
{
  MetaMonitorManager *monitor_manager;
  gulong monitors_changed_id;
  GHashTable *controls; /* MetaMonitor* -> MetaWaylandGammaControl* */
} MetaWaylandGammaContext;

typedef struct _MetaWaylandGammaSaved
{
  MetaCrtc *crtc;
  MetaGammaLut *original;
} MetaWaylandGammaSaved;

typedef struct _MetaWaylandGammaControl
{
  struct wl_resource *resource;
  MetaWaylandGammaContext *ctx;
  MetaMonitor *monitor; /* NULL once failed/invalidated */
  size_t gamma_size;
  GList *saved; /* MetaWaylandGammaSaved* */
} MetaWaylandGammaControl;

static size_t
monitor_gamma_size (MetaMonitor *monitor)
{
  MetaOutput *output = meta_monitor_get_main_output (monitor);
  MetaCrtc *crtc;

  if (!output)
    return 0;

  crtc = meta_output_get_assigned_crtc (output);
  if (!crtc)
    return 0;

  return meta_crtc_get_gamma_lut_size (crtc);
}

static void
gamma_control_restore (MetaWaylandGammaControl *control)
{
  GList *l;

  for (l = control->saved; l; l = l->next)
    {
      MetaWaylandGammaSaved *saved = l->data;

      if (saved->original)
        meta_crtc_set_gamma_lut (saved->crtc, saved->original);
      g_clear_pointer (&saved->original, meta_gamma_lut_free);
      g_free (saved);
    }
  g_clear_pointer (&control->saved, g_list_free);
}

static void
gamma_control_drop_saved (MetaWaylandGammaControl *control)
{
  GList *l;

  for (l = control->saved; l; l = l->next)
    {
      MetaWaylandGammaSaved *saved = l->data;

      g_clear_pointer (&saved->original, meta_gamma_lut_free);
      g_free (saved);
    }
  g_clear_pointer (&control->saved, g_list_free);
}

static void
gamma_control_fail (MetaWaylandGammaControl *control)
{
  if (!control->monitor)
    return;

  /* The CRTCs may be gone (monitor layout change); drop saved state without
   * touching hardware. */
  gamma_control_drop_saved (control);
  if (control->ctx)
    g_hash_table_remove (control->ctx->controls, control->monitor);
  control->monitor = NULL;

  zwlr_gamma_control_v1_send_failed (control->resource);
}

static void
on_monitors_changed (MetaMonitorManager *monitor_manager,
                     gpointer            user_data)
{
  MetaWaylandGammaContext *ctx = user_data;
  GList *controls = g_hash_table_get_values (ctx->controls);
  GList *l;

  for (l = controls; l; l = l->next)
    gamma_control_fail (l->data);

  g_list_free (controls);
}

static void
gamma_control_destroy (struct wl_resource *resource)
{
  MetaWaylandGammaControl *control = wl_resource_get_user_data (resource);

  if (control->monitor)
    {
      gamma_control_restore (control);
      if (control->ctx)
        g_hash_table_remove (control->ctx->controls, control->monitor);
    }
  else
    {
      gamma_control_drop_saved (control);
    }

  g_free (control);
}

static void
gamma_control_handle_set_gamma (struct wl_client   *client,
                                struct wl_resource *resource,
                                int32_t             fd)
{
  MetaWaylandGammaControl *control = wl_resource_get_user_data (resource);
  size_t size = control->gamma_size;
  size_t n_bytes;
  g_autofree uint16_t *table = NULL;
  const uint16_t *red, *green, *blue;
  GList *l;

  if (!control->monitor)
    {
      close (fd);
      return;
    }

  n_bytes = size * 3 * sizeof (uint16_t);

  /* mmap the client fd instead of read()ing it: a regular/memfd file maps, but
   * a pipe or socket — which a blocking read() would wait on forever, hanging
   * the whole compositor main loop — is not mappable, so a malicious client
   * cannot stall us. fstat() guards against a too-small file. */
  {
    struct stat st;
    void *map;

    if (fstat (fd, &st) != 0 || st.st_size < 0 || (size_t) st.st_size < n_bytes)
      {
        close (fd);
        wl_resource_post_error (resource,
                                ZWLR_GAMMA_CONTROL_V1_ERROR_INVALID_GAMMA,
                                "gamma table fd is too small");
        return;
      }

    map = mmap (NULL, n_bytes, PROT_READ, MAP_PRIVATE, fd, 0);
    close (fd);
    if (map == MAP_FAILED)
      {
        wl_resource_post_error (resource,
                                ZWLR_GAMMA_CONTROL_V1_ERROR_INVALID_GAMMA,
                                "failed to map the gamma table");
        return;
      }

    table = g_malloc (n_bytes);
    memcpy (table, map, n_bytes);
    munmap (map, n_bytes);
  }

  red = table;
  green = table + size;
  blue = table + size * 2;

  for (l = meta_monitor_get_outputs (control->monitor); l; l = l->next)
    {
      MetaOutput *output = l->data;
      MetaCrtc *crtc = meta_output_get_assigned_crtc (output);
      MetaGammaLut *lut;

      if (!crtc)
        continue;

      /* Save the original table once, so destroy can restore it. */
      {
        gboolean already_saved = FALSE;
        GList *s;

        for (s = control->saved; s; s = s->next)
          {
            MetaWaylandGammaSaved *saved = s->data;
            if (saved->crtc == crtc)
              {
                already_saved = TRUE;
                break;
              }
          }
        if (!already_saved)
          {
            MetaWaylandGammaSaved *saved = g_new0 (MetaWaylandGammaSaved, 1);
            saved->crtc = crtc;
            saved->original = meta_crtc_get_gamma_lut (crtc);
            control->saved = g_list_prepend (control->saved, saved);
          }
      }

      lut = meta_gamma_lut_new (size, red, green, blue);
      meta_crtc_set_gamma_lut (crtc, lut);
      meta_gamma_lut_free (lut);
    }
}

static void
gamma_control_handle_destroy (struct wl_client   *client,
                              struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct zwlr_gamma_control_v1_interface gamma_control_interface = {
  gamma_control_handle_set_gamma,
  gamma_control_handle_destroy,
};

static void
manager_get_gamma_control (struct wl_client   *client,
                           struct wl_resource *manager_resource,
                           uint32_t            id,
                           struct wl_resource *output_resource)
{
  MetaWaylandGammaContext *ctx = wl_resource_get_user_data (manager_resource);
  MetaWaylandOutput *wayland_output =
    wl_resource_get_user_data (output_resource);
  MetaWaylandGammaControl *control;
  struct wl_resource *resource;
  MetaMonitor *monitor;
  size_t size;

  resource = wl_resource_create (client, &zwlr_gamma_control_v1_interface,
                                 wl_resource_get_version (manager_resource),
                                 id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  control = g_new0 (MetaWaylandGammaControl, 1);
  control->resource = resource;
  control->ctx = ctx;
  wl_resource_set_implementation (resource, &gamma_control_interface, control,
                                  gamma_control_destroy);

  monitor = wayland_output ? meta_wayland_output_get_monitor (wayland_output)
                           : NULL;
  size = monitor ? monitor_gamma_size (monitor) : 0;

  if (!monitor || size == 0)
    {
      zwlr_gamma_control_v1_send_failed (resource);
      return;
    }

  /* At most one gamma control per output: fail any existing one. */
  if (g_hash_table_contains (ctx->controls, monitor))
    {
      MetaWaylandGammaControl *existing =
        g_hash_table_lookup (ctx->controls, monitor);
      gamma_control_fail (existing);
    }

  control->monitor = monitor;
  control->gamma_size = size;
  g_hash_table_insert (ctx->controls, monitor, control);

  zwlr_gamma_control_v1_send_gamma_size (resource, size);
}

static void
manager_destroy (struct wl_client   *client,
                 struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct zwlr_gamma_control_manager_v1_interface manager_interface = {
  manager_get_gamma_control,
  manager_destroy,
};

static void
bind_gamma_control_manager (struct wl_client *client,
                            void             *data,
                            uint32_t          version,
                            uint32_t          id)
{
  MetaWaylandGammaContext *ctx = data;
  struct wl_resource *resource;

  resource = wl_resource_create (client,
                                 &zwlr_gamma_control_manager_v1_interface,
                                 version, id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (resource, &manager_interface, ctx, NULL);
}

void
meta_wayland_init_gamma_control (MetaWaylandCompositor *compositor)
{
  MetaContext *context = meta_wayland_compositor_get_context (compositor);
  MetaBackend *backend = meta_context_get_backend (context);
  MetaWaylandGammaContext *ctx;

  if (!gnoblin_config_get_bool ("protocols", "wlr-gamma-control", TRUE))
    {
      g_message ("Gnoblin wlr-gamma-control protocol disabled by settings");
      return;
    }

  ctx = g_new0 (MetaWaylandGammaContext, 1);
  ctx->monitor_manager = meta_backend_get_monitor_manager (backend);
  ctx->controls = g_hash_table_new (NULL, NULL);
  ctx->monitors_changed_id =
    g_signal_connect (ctx->monitor_manager, "monitors-changed",
                      G_CALLBACK (on_monitors_changed), ctx);

  if (!wl_global_create (compositor->wayland_display,
                         &zwlr_gamma_control_manager_v1_interface,
                         META_WLR_GAMMA_CONTROL_VERSION,
                         ctx,
                         bind_gamma_control_manager))
    g_error ("Failed to register wlr-gamma-control global");
}
