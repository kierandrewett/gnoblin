/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Committed frame layout and xdg-decoration negotiation. */
#include "config.h"
#include "wayland/meta-gnoblin-window-frame.h"
#include "wayland/meta-gnoblin-frame-renderer.h"

#include <gio/gio.h>
#include <math.h>
#include <string.h>
#include <wayland-server.h>
#include "core/window-private.h"
#include "wayland/gnoblin-config.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-surface-private.h"
#include "wayland/meta-wayland-shell-surface.h"
#include "wayland/meta-wayland-window-configuration.h"
#include "wayland/meta-wayland-xdg-shell.h"
#include "wayland/meta-window-wayland.h"
#include "xdg-decoration-unstable-v1-server-protocol.h"

typedef struct {
    int policy; /* 0 disabled, 1 client preference, 2 prefer SSD, 3 replace CSD */
    MetaGnoblinFrameLayout requested;
    MetaGnoblinFrameLayout committed;
    MtkRectangle client_geometry;
    guint32 serial;
    gboolean changed;
} FrameState;

typedef struct {
    struct wl_resource* resource;
    struct wl_listener toplevel_destroy;
    MetaWaylandSurface* surface;
    int preference;
    int sent_mode;
} Decoration;

static FrameState* frame_state(MetaWindow* window) {
    FrameState* state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (!state) {
        state = g_new0(FrameState, 1);
        /* Advertising xdg-decoration is not permission to add chrome. */
        state->policy = 0;
        state->requested.border[0] = 32;
        state->requested.border[1] = 1;
        state->requested.border[2] = 1;
        state->requested.border[3] = 1;
        state->committed.mode = 1;
        g_object_set_data_full(G_OBJECT(window), "gnoblin-frame", state, g_free);
    }
    return state;
}

static MetaWaylandSurface* window_surface(MetaWindow* window) {
    MetaWaylandSurface* surface;
    /* The window actor can outlive its destroyed Wayland surface during an
     * animation or queued rules update. Never dereference that retired surface. */
    if (!META_IS_WINDOW_WAYLAND(window) || window->unmanaging)
        return NULL;
    surface = meta_window_get_wayland_surface(window);
    return surface && META_IS_WAYLAND_XDG_TOPLEVEL(surface->role) ? surface : NULL;
}

static void request_configuration(MetaWindow* window) {
    g_autoptr(MetaWaylandWindowConfiguration) configuration = NULL;
    MtkRectangle rect;
    if (!window || window->unmanaging || !window_surface(window))
        return;
    meta_window_get_frame_rect(window, &rect);
    configuration = meta_wayland_window_configuration_new(
        window, rect, 0, 0, meta_window_wayland_get_geometry_scale(window),
        META_MOVE_RESIZE_STATE_CHANGED, META_GRAVITY_NORTH_WEST);
    meta_window_wayland_configure(META_WINDOW_WAYLAND(window), configuration);
}

/**
 * meta_gnoblin_window_frame_set:
 * @window: managed Wayland toplevel
 * @policy: tuple (mode, crop top/right/bottom/left, border top/right/bottom/left)
 * @error: return location for an error
 *
 * Layout becomes visible only with the acknowledging client commit.
 * Returns: whether the policy is valid
 */
gboolean meta_gnoblin_window_frame_set(MetaWindow* window, GVariant* policy, GError** error) {
    int values[9];
    FrameState* state;
    if (!window_surface(window) || !g_variant_is_of_type(policy, G_VARIANT_TYPE("(iiiiiiiii)"))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "Frame policy requires a Wayland toplevel and nine integers");
        return FALSE;
    }
    for (int i = 0; i < 9; i++) {
        g_variant_get_child(policy, i, "i", &values[i]);
        if (values[i] < 0 || values[i] > (i == 0 ? 3 : 256)) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                                "Invalid frame mode or margin (expected 0..256)");
            return FALSE;
        }
    }
    state = frame_state(window);
    if (state->policy == values[0] &&
        memcmp(state->requested.crop, values + 1, 4 * sizeof(int)) == 0 &&
        memcmp(state->requested.border, values + 5, 4 * sizeof(int)) == 0)
        return TRUE;
    state->policy = values[0];
    memcpy(state->requested.crop, values + 1, 4 * sizeof(int));
    memcpy(state->requested.border, values + 5, 4 * sizeof(int));
    request_configuration(window);
    return TRUE;
}

/**
 * meta_gnoblin_window_frame_get:
 * @window: managed window
 * Returns: (transfer full): committed layout dictionary, in logical pixels
 */
GVariant* meta_gnoblin_window_frame_get(MetaWindow* window) {
    FrameState* state = frame_state(window);
    MetaGnoblinFrameLayout* layout = &state->committed;
    GVariantBuilder b;
    g_variant_builder_init(&b, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&b, "{sv}", "native", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&b, "{sv}", "presentation", meta_gnoblin_frame_renderer_status(window));
    g_variant_builder_add(&b, "{sv}", "supported",
                          g_variant_new_boolean(window_surface(window) != NULL));
    g_variant_builder_add(&b, "{sv}", "scale",
                          g_variant_new_int32(window_surface(window)
                                                  ? meta_window_wayland_get_geometry_scale(window)
                                                  : 1));
    g_variant_builder_add(&b, "{sv}", "mode", g_variant_new_int32(layout->mode));
    g_variant_builder_add(&b, "{sv}", "crop",
                          g_variant_new("(iiii)", layout->crop[0], layout->crop[1], layout->crop[2],
                                        layout->crop[3]));
    g_variant_builder_add(&b, "{sv}", "border",
                          g_variant_new("(iiii)", layout->border[0], layout->border[1],
                                        layout->border[2], layout->border[3]));
    g_variant_builder_add(&b, "{sv}", "client",
                          g_variant_new("(iiii)", state->client_geometry.x,
                                        state->client_geometry.y, state->client_geometry.width,
                                        state->client_geometry.height));
    g_variant_builder_add(&b, "{sv}", "serial", g_variant_new_uint32(state->serial));
    return g_variant_ref_sink(g_variant_builder_end(&b));
}

void meta_gnoblin_window_frame_configure(MetaWindow* window,
                                         MetaWaylandWindowConfiguration* configuration) {
    MetaWaylandSurface* surface = window_surface(window);
    FrameState* state;
    Decoration* decoration;
    MetaGnoblinFrameLayout layout = {.mode = 1};
    gboolean fullscreen;
    if (!surface)
        return;
    state = frame_state(window);
    decoration = g_object_get_data(G_OBJECT(surface->role), "gnoblin-decoration");
    fullscreen =
        configuration->config && meta_window_config_get_is_fullscreen(configuration->config);
    if (decoration && state->policy != 0 && state->policy != 3 &&
        (state->policy == 2 || decoration->preference != 1))
        layout.mode = 2;
    if (!fullscreen &&
        (layout.mode == 2 || state->policy == 3 ||
         (state->policy == 2 && (state->requested.crop[0] || state->requested.crop[1] ||
                                 state->requested.crop[2] || state->requested.crop[3])))) {
        memcpy(layout.border, state->requested.border, sizeof layout.border);
        if (layout.mode != 2)
            memcpy(layout.crop, state->requested.crop, sizeof layout.crop);
    }
    configuration->gnoblin_frame = layout;
    if (decoration && decoration->sent_mode != layout.mode) {
        zxdg_toplevel_decoration_v1_send_configure(decoration->resource, layout.mode);
        decoration->sent_mode = layout.mode;
    }
}

void meta_gnoblin_window_frame_commit(MetaWindow* window,
                                      MetaWaylandWindowConfiguration* configuration,
                                      MtkRectangle* geometry) {
    FrameState* state;
    MetaGnoblinFrameLayout layout;
    if (!window_surface(window))
        return;
    state = frame_state(window);
    state->client_geometry = *geometry;
    layout = configuration ? configuration->gnoblin_frame : state->committed;
    if (layout.crop[1] + layout.crop[3] >= geometry->width ||
        layout.crop[0] + layout.crop[2] >= geometry->height)
        memset(layout.crop, 0, sizeof layout.crop);
    state->changed |= memcmp(&layout, &state->committed, sizeof layout) != 0;
    state->committed = layout;
    if (configuration)
        state->serial = configuration->serial;
    geometry->x += layout.crop[3] - layout.border[3];
    geometry->y += layout.crop[0] - layout.border[0];
    geometry->width += layout.border[1] + layout.border[3] - layout.crop[1] - layout.crop[3];
    geometry->height += layout.border[0] + layout.border[2] - layout.crop[0] - layout.crop[2];
}

void meta_gnoblin_window_frame_sync_actor(MetaWindow* window, ClutterActor* surface) {
    FrameState* state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (!state)
        return;
    if (state->committed.mode == 2 || state->committed.crop[0] || state->committed.crop[1] ||
        state->committed.crop[2] || state->committed.crop[3])
        clutter_actor_set_clip(
            surface, state->client_geometry.x + state->committed.crop[3],
            state->client_geometry.y + state->committed.crop[0],
            state->client_geometry.width - state->committed.crop[1] - state->committed.crop[3],
            state->client_geometry.height - state->committed.crop[0] - state->committed.crop[2]);
    else
        clutter_actor_remove_clip(surface);
    meta_gnoblin_frame_renderer_sync(window, &state->committed);
}

/**
 * meta_gnoblin_window_frame_style:
 * @window: managed window
 * @options: validated frame appearance and renderer selection
 *
 * Configuration bridge only; all presentation and input live in native code.
 */
void meta_gnoblin_window_frame_style(MetaWindow* window, GVariant* options) {
    if (!window_surface(window) || !g_variant_is_of_type(options, G_VARIANT_TYPE_VARDICT))
        return;
    double radius = 0, exponent = 2;
    g_variant_lookup(options, "radius", "d", &radius);
    g_variant_lookup(options, "exponent", "d", &exponent);
    if (!isfinite(radius) || radius < 0 || radius > 256 || !isfinite(exponent) || exponent < 2 ||
        exponent > 6)
        return;
    GVariant* old = g_object_get_data(G_OBJECT(window), "gnoblin-frame-style");
    if (old && g_variant_equal(old, options))
        return;
    g_object_set_data_full(G_OBJECT(window), "gnoblin-frame-style", g_variant_ref(options),
                           (GDestroyNotify)g_variant_unref);
    meta_gnoblin_frame_renderer_style_changed(window);
}

void meta_gnoblin_window_frame_emit_changed(MetaWindow* window) {
    FrameState* state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (state && state->changed) {
        state->changed = FALSE;
        g_signal_emit_by_name(window, "gnoblin-frame-changed");
    }
}

gboolean meta_gnoblin_window_frame_is_active(MetaWindow* window) {
    FrameState* state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (!state)
        return FALSE;
    for (int i = 0; i < 4; i++)
        if (state->committed.border[i] || state->committed.crop[i])
            return TRUE;
    return FALSE;
}

static void decoration_free(struct wl_resource* resource) {
    Decoration* d = wl_resource_get_user_data(resource);
    MetaWindow* window = meta_wayland_surface_get_window(d->surface);
    g_object_set_data(G_OBJECT(d->surface->role), "gnoblin-decoration", NULL);
    wl_list_remove(&d->toplevel_destroy.link);
    request_configuration(window);
    g_object_unref(d->surface);
    g_free(d);
}

static void toplevel_destroyed(struct wl_listener* listener, void* data) {
    Decoration* d = wl_container_of(listener, d, toplevel_destroy);
    wl_resource_post_error(d->resource, ZXDG_TOPLEVEL_DECORATION_V1_ERROR_ORPHANED,
                           "Destroy the decoration before its toplevel");
    wl_resource_destroy(d->resource);
}

static void destroy_resource(struct wl_client* client, struct wl_resource* resource) {
    wl_resource_destroy(resource);
}

static void set_mode(struct wl_client* client, struct wl_resource* resource, uint32_t mode) {
    Decoration* d = wl_resource_get_user_data(resource);
    if (mode != 1 && mode != 2) {
        wl_resource_post_error(resource, ZXDG_TOPLEVEL_DECORATION_V1_ERROR_INVALID_MODE,
                               "Invalid decoration mode %u", mode);
        return;
    }
    d->preference = mode;
    request_configuration(meta_wayland_surface_get_window(d->surface));
}

static void unset_mode(struct wl_client* client, struct wl_resource* resource) {
    Decoration* d = wl_resource_get_user_data(resource);
    d->preference = 0;
    request_configuration(meta_wayland_surface_get_window(d->surface));
}

static const struct zxdg_toplevel_decoration_v1_interface decoration_impl = {destroy_resource,
                                                                             set_mode, unset_mode};

static void get_decoration(struct wl_client* client, struct wl_resource* manager, uint32_t id,
                           struct wl_resource* toplevel_resource) {
    MetaWaylandXdgToplevel* toplevel = wl_resource_get_user_data(toplevel_resource);
    MetaWaylandSurface* surface =
        meta_wayland_surface_role_get_surface(META_WAYLAND_SURFACE_ROLE(toplevel));
    Decoration* d;
    if (g_object_get_data(G_OBJECT(toplevel), "gnoblin-decoration")) {
        wl_resource_post_error(manager, ZXDG_TOPLEVEL_DECORATION_V1_ERROR_ALREADY_CONSTRUCTED,
                               "Toplevel already has a decoration");
        return;
    }
    if (meta_wayland_surface_get_buffer(surface)) {
        wl_resource_post_error(manager, ZXDG_TOPLEVEL_DECORATION_V1_ERROR_UNCONFIGURED_BUFFER,
                               "Decoration must be created before mapping");
        return;
    }
    d = g_new0(Decoration, 1);
    d->resource = wl_resource_create(client, &zxdg_toplevel_decoration_v1_interface, 1, id);
    if (!d->resource) {
        g_free(d);
        wl_client_post_no_memory(client);
        return;
    }
    d->surface = g_object_ref(surface);
    d->toplevel_destroy.notify = toplevel_destroyed;
    wl_resource_add_destroy_listener(toplevel_resource, &d->toplevel_destroy);
    g_object_set_data(G_OBJECT(toplevel), "gnoblin-decoration", d);
    wl_resource_set_implementation(d->resource, &decoration_impl, d, decoration_free);
}

static const struct zxdg_decoration_manager_v1_interface manager_impl = {destroy_resource,
                                                                         get_decoration};

static void bind_manager(struct wl_client* client, void* data, uint32_t version, uint32_t id) {
    struct wl_resource* resource =
        wl_resource_create(client, &zxdg_decoration_manager_v1_interface, 1, id);
    if (!resource) {
        wl_client_post_no_memory(client);
        return;
    }
    wl_resource_set_implementation(resource, &manager_impl, NULL, NULL);
}

void meta_gnoblin_window_frame_init(MetaWaylandCompositor* compositor) {
    meta_gnoblin_frame_renderer_init(compositor);
    g_signal_new("gnoblin-frame-changed", META_TYPE_WINDOW, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                 G_TYPE_NONE, 0);
    if (gnoblin_config_protocol_enabled("xdg-decoration"))
        wl_global_create(compositor->wayland_display, &zxdg_decoration_manager_v1_interface, 1,
                         NULL, bind_manager);
}
