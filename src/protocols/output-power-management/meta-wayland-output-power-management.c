/*
 * gnoblin: wlr-output-power-management-unstable-v1 support for mutter.
 *
 * Implements the wlroots output-power-management protocol so DPMS tools such
 * as wlopm can blank and unblank outputs in a Gnoblin session.
 *
 * Limitation: mutter's power-save state is global (all outputs share one
 * MetaPowerSave mode), so setting any output's power mode applies to every
 * output. The protocol is per-output, but the effect is system-wide. This
 * matches how DPMS is wired in mutter; per-output DPMS would require backend
 * changes beyond this protocol shim.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "config.h"

#include "wayland/meta-wayland-output-power-management.h"

#include <gio/gio.h>
#include <wayland-server.h>

#include "backends/meta-backend-private.h"
#include "backends/meta-display-config-shared.h"
#include "backends/meta-monitor-manager-private.h"
#include "meta/meta-backend.h"
#include "meta/meta-context.h"
#include "meta/meta-monitor-manager.h"
#include "wayland/meta-wayland-outputs.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/gnoblin-config.h"

#include "wlr-output-power-management-unstable-v1-server-protocol.h"

#define META_WLR_OUTPUT_POWER_MANAGEMENT_VERSION 1

typedef struct _MetaWaylandOutputPowerContext
{
  MetaMonitorManager *monitor_manager;
  gulong power_save_changed_id;
  gulong monitors_changed_id;
  GList *controls; /* MetaWaylandOutputPower* */
} MetaWaylandOutputPowerContext;

typedef struct _MetaWaylandOutputPower
{
  struct wl_resource *resource;
  MetaWaylandOutputPowerContext *ctx;
  MetaMonitor *monitor; /* NULL once invalidated */
} MetaWaylandOutputPower;

static uint32_t
current_protocol_mode (MetaWaylandOutputPowerContext *ctx)
{
  MetaPowerSave mode =
    meta_monitor_manager_get_power_save_mode (ctx->monitor_manager);

  return mode == META_POWER_SAVE_ON ? ZWLR_OUTPUT_POWER_V1_MODE_ON
                                    : ZWLR_OUTPUT_POWER_V1_MODE_OFF;
}

static void
output_power_send_mode (MetaWaylandOutputPower *power)
{
  if (!power->monitor)
    return;

  zwlr_output_power_v1_send_mode (power->resource,
                                  current_protocol_mode (power->ctx));
}

static void
on_power_save_changed (MetaMonitorManager        *manager,
                       MetaPowerSaveChangeReason  reason,
                       gpointer                   user_data)
{
  MetaWaylandOutputPowerContext *ctx = user_data;
  GList *l;

  for (l = ctx->controls; l; l = l->next)
    output_power_send_mode (l->data);
}

static void
output_power_fail (MetaWaylandOutputPower *power)
{
  if (!power->monitor)
    return;

  power->monitor = NULL;
  zwlr_output_power_v1_send_failed (power->resource);
}

static void
on_monitors_changed (MetaMonitorManager *manager,
                     gpointer            user_data)
{
  MetaWaylandOutputPowerContext *ctx = user_data;
  GList *controls = g_list_copy (ctx->controls);
  GList *l;

  for (l = controls; l; l = l->next)
    output_power_fail (l->data);

  g_list_free (controls);
}

static void
output_power_destroy (struct wl_resource *resource)
{
  MetaWaylandOutputPower *power = wl_resource_get_user_data (resource);

  if (power->ctx)
    power->ctx->controls = g_list_remove (power->ctx->controls, power);

  g_free (power);
}

static void
output_power_handle_set_mode (struct wl_client   *client,
                              struct wl_resource *resource,
                              uint32_t            mode)
{
  MetaWaylandOutputPower *power = wl_resource_get_user_data (resource);
  MetaMonitorManager *manager;
  MetaMonitorManagerClass *klass;
  MetaPowerSave target;

  if (mode != ZWLR_OUTPUT_POWER_V1_MODE_OFF &&
      mode != ZWLR_OUTPUT_POWER_V1_MODE_ON)
    {
      wl_resource_post_error (resource,
                              ZWLR_OUTPUT_POWER_V1_ERROR_INVALID_MODE,
                              "invalid power save mode");
      return;
    }

  if (!power->monitor)
    return;

  manager = power->ctx->monitor_manager;
  klass = META_MONITOR_MANAGER_GET_CLASS (manager);
  target = mode == ZWLR_OUTPUT_POWER_V1_MODE_ON ? META_POWER_SAVE_ON
                                                : META_POWER_SAVE_OFF;

  /* mutter DPMS is global: apply to the whole backend (see file header). */
  if (klass->set_power_save_mode)
    klass->set_power_save_mode (manager, target);
  meta_monitor_manager_power_save_mode_changed (
    manager, target, META_POWER_SAVE_CHANGE_REASON_MODE_CHANGE);
}

static void
output_power_handle_destroy (struct wl_client   *client,
                             struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct zwlr_output_power_v1_interface output_power_interface = {
  output_power_handle_set_mode,
  output_power_handle_destroy,
};

static void
manager_get_output_power (struct wl_client   *client,
                          struct wl_resource *manager_resource,
                          uint32_t            id,
                          struct wl_resource *output_resource)
{
  MetaWaylandOutputPowerContext *ctx =
    wl_resource_get_user_data (manager_resource);
  MetaWaylandOutput *wayland_output =
    wl_resource_get_user_data (output_resource);
  MetaWaylandOutputPower *power;
  struct wl_resource *resource;
  MetaMonitor *monitor;

  resource = wl_resource_create (client, &zwlr_output_power_v1_interface,
                                 wl_resource_get_version (manager_resource),
                                 id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  power = g_new0 (MetaWaylandOutputPower, 1);
  power->resource = resource;
  power->ctx = ctx;
  wl_resource_set_implementation (resource, &output_power_interface, power,
                                  output_power_destroy);

  ctx->controls = g_list_prepend (ctx->controls, power);

  monitor = wayland_output ? meta_wayland_output_get_monitor (wayland_output)
                           : NULL;
  if (!monitor)
    {
      zwlr_output_power_v1_send_failed (resource);
      return;
    }

  power->monitor = monitor;
  output_power_send_mode (power);
}

static void
manager_destroy (struct wl_client   *client,
                 struct wl_resource *resource)
{
  wl_resource_destroy (resource);
}

static const struct zwlr_output_power_manager_v1_interface manager_interface = {
  manager_get_output_power,
  manager_destroy,
};

static void
bind_output_power_manager (struct wl_client *client,
                           void             *data,
                           uint32_t          version,
                           uint32_t          id)
{
  MetaWaylandOutputPowerContext *ctx = data;
  struct wl_resource *resource;

  resource = wl_resource_create (client,
                                 &zwlr_output_power_manager_v1_interface,
                                 version, id);
  if (!resource)
    {
      wl_client_post_no_memory (client);
      return;
    }

  wl_resource_set_implementation (resource, &manager_interface, ctx, NULL);
}

void
meta_wayland_init_output_power_management (MetaWaylandCompositor *compositor)
{
  MetaContext *context = meta_wayland_compositor_get_context (compositor);
  MetaBackend *backend = meta_context_get_backend (context);
  MetaWaylandOutputPowerContext *ctx;

  if (!gnoblin_config_protocol_enabled ("wlr-output-power-management"))
    {
      g_message ("Gnoblin wlr-output-power-management protocol disabled by settings");
      return;
    }

  ctx = g_new0 (MetaWaylandOutputPowerContext, 1);
  ctx->monitor_manager = meta_backend_get_monitor_manager (backend);
  ctx->power_save_changed_id =
    g_signal_connect (ctx->monitor_manager, "power-save-mode-changed",
                      G_CALLBACK (on_power_save_changed), ctx);
  ctx->monitors_changed_id =
    g_signal_connect (ctx->monitor_manager, "monitors-changed",
                      G_CALLBACK (on_monitors_changed), ctx);

  if (!wl_global_create (compositor->wayland_display,
                         &zwlr_output_power_manager_v1_interface,
                         META_WLR_OUTPUT_POWER_MANAGEMENT_VERSION,
                         ctx,
                         bind_output_power_manager))
    g_error ("Failed to register wlr-output-power-management global");
}
