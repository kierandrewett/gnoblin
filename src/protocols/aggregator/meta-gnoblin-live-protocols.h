/*
 * Runtime-owned registration for configurable Wayland globals.
 *
 * Gnoblin protocol globals are created from the validated config and can be
 * withdrawn or re-announced during a config transaction. Existing resources
 * remain valid until their clients disconnect, as required by Wayland.
 */
#pragma once

#include <gio/gio.h>
#include <wayland-server-core.h>

#include "meta/meta-wayland-compositor.h"
#include "wayland/meta-wayland-types.h"

typedef void (*MetaGnoblinProtocolGlobalChangedFunc)(MetaWaylandCompositor *compositor,
                                                      struct wl_global *global,
                                                      gboolean enabled,
                                                      gpointer user_data);

gboolean meta_gnoblin_register_protocol_global(MetaWaylandCompositor *compositor,
                                               const char *config_name,
                                               const struct wl_interface *interface,
                                               uint32_t version,
                                               void *data,
                                               wl_global_bind_func_t bind,
                                               GError **error);

/* Like meta_gnoblin_register_protocol_global(), with a lifecycle hook for
 * protocol-specific registration such as a filter-manager ACL. The hook runs
 * after a global is created and before it is withdrawn and destroyed. */
gboolean meta_gnoblin_register_protocol_global_full(
    MetaWaylandCompositor *compositor, const char *config_name,
    const struct wl_interface *interface, uint32_t version, void *data,
    wl_global_bind_func_t bind, MetaGnoblinProtocolGlobalChangedFunc changed,
    gpointer changed_data, GError **error);

gboolean meta_gnoblin_apply_protocol_configuration(MetaWaylandCompositor *compositor,
                                                   GError **error);
