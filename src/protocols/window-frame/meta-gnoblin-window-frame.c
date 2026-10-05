/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Committed frame layout and Wayland decoration negotiation. */
#include "config.h"
#include "wayland/meta-gnoblin-window-frame.h"
#include "wayland/meta-gnoblin-frame-renderer.h"
#include "compositor/meta-gnoblin-window-effects.h"

#include <gio/gio.h>
#include <math.h>
#include <string.h>
#include <wayland-server.h>
#include "core/window-private.h"
#include "core/gnoblin-native-control.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-surface-private.h"
#include "wayland/meta-wayland-shell-surface.h"
#include "wayland/meta-wayland-window-configuration.h"
#include "wayland/meta-wayland-xdg-shell.h"
#include "wayland/meta-window-wayland.h"
#include "kde-server-decoration-server-protocol.h"
#include "xdg-decoration-unstable-v1-server-protocol.h"

typedef enum {
    DECORATION_XDG,
    DECORATION_KDE,
} DecorationProtocol;

typedef struct {
    int policy; /* 0 disabled, 1 client preference, 2 prefer SSD, 3 replace CSD */
    MetaGnoblinFrameLayout requested;
    MetaGnoblinFrameLayout committed;
    MtkRectangle client_geometry;
    guint32 serial;
    gboolean changed;
    gboolean policy_set;
} FrameState;

typedef struct {
    struct wl_resource* resource;
    struct wl_listener toplevel_destroy;
    struct wl_listener surface_destroy;
    MetaWaylandSurface* surface;
    DecorationProtocol protocol;
    int preference;
    int sent_mode;
    gboolean is_toplevel;
    gboolean role_assigned;
    gboolean has_toplevel_listener;
    gboolean has_surface_listener;
} Decoration;

static void toplevel_destroyed(struct wl_listener* listener, void* data);

static FrameState* frame_state(MetaWindow* window) {
    FrameState* state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (!state) {
        state = g_new0(FrameState, 1);
        /* Native chrome is opt-in except for GTK's server-decoration path. */
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

static int effective_policy(MetaWindow* window, FrameState* state) {
    MetaWaylandSurface* surface;
    Decoration* decoration;

    if (!state->policy_set && (surface = window_surface(window))) {
        decoration = g_object_get_data(G_OBJECT(surface->role), "gnoblin-decoration");
        /* GTK's Wayland backend uses the KDE protocol for decoration
         * negotiation. Its object creation is an explicit request that can
         * use the same default as the Lua "auto" mode. */
        if (decoration && decoration->protocol == DECORATION_KDE)
            return 1;
    }

    return state->policy;
}

static void request_configuration(MetaWindow* window) {
    g_autoptr(MetaWaylandWindowConfiguration) configuration = NULL;
    MtkRectangle rect;
    if (!window || window->unmanaging || !window_surface(window))
        return;
    /* xdg-decoration requests can arrive before the first client commit.
     * Mutter's initial configure consumes the saved frame state; sending an
     * extra configure here would skip its initial ready transition. */
    if (!meta_window_is_ready(window))
        return;
    meta_window_get_frame_rect(window, &rect);
    configuration = meta_wayland_window_configuration_new(
        window, rect, 0, 0, meta_window_wayland_get_geometry_scale(window),
        META_MOVE_RESIZE_STATE_CHANGED, META_GRAVITY_NORTH_WEST);
    meta_window_wayland_configure(META_WINDOW_WAYLAND(window), configuration);
}

static void kde_decoration_set_mode(Decoration* decoration, uint32_t mode) {
    decoration->preference = mode;
    if (decoration->sent_mode == (int)mode)
        return;
    org_kde_kwin_server_decoration_send_mode(decoration->resource, mode);
    decoration->sent_mode = mode;
}

static void kde_decoration_attach_toplevel(Decoration* decoration,
                                           MetaWaylandXdgToplevel* toplevel) {
    struct wl_resource* toplevel_resource;
    MetaWaylandSurface* surface;

    if (!decoration || decoration->protocol != DECORATION_KDE ||
        !META_IS_WAYLAND_XDG_TOPLEVEL(toplevel))
        return;

    surface = meta_wayland_surface_role_get_surface(META_WAYLAND_SURFACE_ROLE(toplevel));
    decoration->role_assigned = TRUE;
    decoration->is_toplevel = TRUE;
    decoration->preference = ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_SERVER;
    g_object_set_data(G_OBJECT(surface), "gnoblin-decoration", NULL);
    g_object_set_data(G_OBJECT(toplevel), "gnoblin-decoration", decoration);

    toplevel_resource = meta_wayland_xdg_toplevel_get_resource(toplevel);
    if (toplevel_resource && !decoration->has_toplevel_listener) {
        decoration->toplevel_destroy.notify = toplevel_destroyed;
        wl_resource_add_destroy_listener(toplevel_resource, &decoration->toplevel_destroy);
        decoration->has_toplevel_listener = TRUE;
    }

    kde_decoration_set_mode(decoration, ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_SERVER);
}

void meta_gnoblin_window_frame_surface_role_assigned(MetaWaylandSurface* surface,
                                                     gboolean is_toplevel) {
    Decoration* decoration = g_object_get_data(G_OBJECT(surface), "gnoblin-decoration");

    if (!decoration || decoration->protocol != DECORATION_KDE)
        return;

    decoration->role_assigned = TRUE;
    if (is_toplevel) {
        kde_decoration_attach_toplevel(decoration, META_WAYLAND_XDG_TOPLEVEL(surface->role));
        return;
    }

    decoration->is_toplevel = FALSE;
    kde_decoration_set_mode(decoration, ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_NONE);
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
    if (state->policy_set && state->policy == values[0] &&
        memcmp(state->requested.crop, values + 1, 4 * sizeof(int)) == 0 &&
        memcmp(state->requested.border, values + 5, 4 * sizeof(int)) == 0)
        return TRUE;
    state->policy_set = TRUE;
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
    if (decoration && decoration->protocol == DECORATION_KDE)
        kde_decoration_attach_toplevel(decoration, META_WAYLAND_XDG_TOPLEVEL(surface->role));
    fullscreen =
        configuration->config && meta_window_config_get_is_fullscreen(configuration->config);
    /* An unset preference means the client has not opted into SSD. Treat it
     * as CSD under auto policy: libdecor and similar clients can still paint
     * their own titlebar even after a compositor-selected server configure.
     * Only an explicit server-side preference may add SSD in auto mode. */
    const int policy = effective_policy(window, state);
    if (decoration && policy != 0 && policy != 3 && (policy == 2 || decoration->preference == 2))
        layout.mode = 2;
    if (!fullscreen && (layout.mode == 2 || policy == 3 ||
                        (policy == 2 && (state->requested.crop[0] || state->requested.crop[1] ||
                                         state->requested.crop[2] || state->requested.crop[3])))) {
        memcpy(layout.border, state->requested.border, sizeof layout.border);
        if (layout.mode != 2)
            memcpy(layout.crop, state->requested.crop, sizeof layout.crop);
    }
    configuration->gnoblin_frame = layout;
    if (decoration) {
        const int decoration_mode = decoration->protocol == DECORATION_KDE ? 2 : layout.mode;
        if (decoration->sent_mode != decoration_mode) {
            if (decoration->protocol == DECORATION_KDE)
                org_kde_kwin_server_decoration_send_mode(decoration->resource, decoration_mode);
            else
                zxdg_toplevel_decoration_v1_send_configure(decoration->resource, decoration_mode);
            decoration->sent_mode = decoration_mode;
        }
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

/* XDG positioners are relative to the client surface, whereas MetaWindow's
 * frame rectangle includes the optional Gnoblin frame. Keep the coordinate
 * conversion here with the inverse of meta_gnoblin_window_frame_commit(). */
void meta_gnoblin_window_frame_rect_to_client(MetaWindow* window,
                                              const MetaWaylandWindowConfiguration* configuration,
                                              MtkRectangle* rect) {
    FrameState* state;
    MetaGnoblinFrameLayout layout;

    if (!window_surface(window))
        return;

    state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (!state)
        return;

    layout = configuration ? configuration->gnoblin_frame : state->committed;
    rect->x += layout.border[3] - layout.crop[3];
    rect->y += layout.border[0] - layout.crop[0];
    rect->width += layout.crop[1] + layout.crop[3] - layout.border[1] - layout.border[3];
    rect->height += layout.crop[0] + layout.crop[2] - layout.border[0] - layout.border[2];
}

void meta_gnoblin_window_frame_sync_actor(MetaWindow* window, ClutterActor* surface) {
    FrameState* state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (!state)
        return;
    /* xdg_surface.set_window_geometry describes the visible window bounds;
     * clients use it to exclude invisible CSD shadows from the buffer. A
     * prefer-server policy must apply those bounds even when a client ignores
     * the requested SSD mode and keeps drawing its own frame. */
    MtkRectangle visible_geometry = {0};
    const gboolean has_visible_geometry =
        meta_gnoblin_window_frame_get_visible_geometry(window, &visible_geometry);
    const gboolean has_manual_crop = state->committed.crop[0] || state->committed.crop[1] ||
                                     state->committed.crop[2] || state->committed.crop[3];
    if (has_visible_geometry) {
        /* The rounded-clip effect maps these surface-buffer bounds into its
         * offscreen target. A Clutter actor clip uses local allocation
         * coordinates instead and would crop the visible content twice. */
        clutter_actor_remove_clip(surface);
    } else if (state->committed.mode == 2 || has_manual_crop) {
        clutter_actor_set_clip(
            surface, state->client_geometry.x + state->committed.crop[3],
            state->client_geometry.y + state->committed.crop[0],
            state->client_geometry.width - state->committed.crop[1] - state->committed.crop[3],
            state->client_geometry.height - state->committed.crop[0] - state->committed.crop[2]);
    } else {
        clutter_actor_remove_clip(surface);
    }

    double visible_bounds[4] = {visible_geometry.x, visible_geometry.y,
                                visible_geometry.x + visible_geometry.width,
                                visible_geometry.y + visible_geometry.height};
    meta_gnoblin_window_effects_set_rounded_clip_geometry(
        surface, has_visible_geometry ? visible_bounds : NULL,
        meta_window_wayland_get_geometry_scale(window));
    meta_gnoblin_frame_renderer_sync(window, &state->committed);
}

gboolean meta_gnoblin_window_frame_get_visible_geometry(MetaWindow* window,
                                                        MtkRectangle* geometry) {
    FrameState* state;
    g_return_val_if_fail(META_IS_WINDOW(window), FALSE);
    g_return_val_if_fail(geometry != NULL, FALSE);

    state = g_object_get_data(G_OBJECT(window), "gnoblin-frame");
    if (!state || effective_policy(window, state) != 2)
        return FALSE;

    *geometry = state->client_geometry;
    /* Manual crop applies only while the client keeps CSD. Negotiated SSD
     * uses the client's declared geometry without extra crop margins. */
    if (state->committed.mode == 1) {
        geometry->x += state->committed.crop[3];
        geometry->y += state->committed.crop[0];
        geometry->width -= state->committed.crop[1] + state->committed.crop[3];
        geometry->height -= state->committed.crop[0] + state->committed.crop[2];
    }
    return geometry->width > 0 && geometry->height > 0;
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
    MetaWindow* window = d->surface ? meta_wayland_surface_get_window(d->surface) : NULL;

    if (d->surface && d->surface->role &&
        g_object_get_data(G_OBJECT(d->surface->role), "gnoblin-decoration") == d)
        g_object_set_data(G_OBJECT(d->surface->role), "gnoblin-decoration", NULL);
    if (d->surface && g_object_get_data(G_OBJECT(d->surface), "gnoblin-decoration") == d)
        g_object_set_data(G_OBJECT(d->surface), "gnoblin-decoration", NULL);
    if (d->has_toplevel_listener)
        wl_list_remove(&d->toplevel_destroy.link);
    if (d->has_surface_listener)
        wl_list_remove(&d->surface_destroy.link);
    request_configuration(window);
    g_clear_object(&d->surface);
    g_free(d);
}

static void toplevel_destroyed(struct wl_listener* listener, void* data) {
    Decoration* d = wl_container_of(listener, d, toplevel_destroy);

    if (d->protocol == DECORATION_XDG)
        wl_resource_post_error(d->resource, ZXDG_TOPLEVEL_DECORATION_V1_ERROR_ORPHANED,
                               "Destroy the decoration before its toplevel");
    wl_resource_destroy(d->resource);
}

static void surface_destroyed(struct wl_listener* listener, void* data) {
    Decoration* d = wl_container_of(listener, d, surface_destroy);

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

static const struct zxdg_toplevel_decoration_v1_interface decoration_impl = {
    .destroy = destroy_resource,
    .set_mode = set_mode,
    .unset_mode = unset_mode,
};

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
    d->protocol = DECORATION_XDG;
    d->resource = wl_resource_create(client, &zxdg_toplevel_decoration_v1_interface, 1, id);
    if (!d->resource) {
        g_free(d);
        wl_client_post_no_memory(client);
        return;
    }
    d->surface = g_object_ref(surface);
    d->toplevel_destroy.notify = toplevel_destroyed;
    wl_resource_add_destroy_listener(toplevel_resource, &d->toplevel_destroy);
    d->has_toplevel_listener = TRUE;
    d->surface_destroy.notify = surface_destroyed;
    wl_resource_add_destroy_listener(surface->resource, &d->surface_destroy);
    d->has_surface_listener = TRUE;
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

static void kde_request_mode(struct wl_client* client, struct wl_resource* resource,
                             uint32_t mode) {
    Decoration* d = wl_resource_get_user_data(resource);
    const int applied_mode = !d->role_assigned || d->is_toplevel
                                 ? ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_SERVER
                                 : ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_NONE;

    if (mode > ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_SERVER) {
        wl_resource_post_error(resource, WL_DISPLAY_ERROR_INVALID_OBJECT,
                               "Invalid server decoration mode %u", mode);
        return;
    }

    /* The KDE protocol reports the applied mode. Gnoblin keeps its compositor
     * frame authoritative even when a client asks to switch back to CSD. */
    d->preference = applied_mode;
    org_kde_kwin_server_decoration_send_mode(resource, applied_mode);
    d->sent_mode = applied_mode;
    if (d->is_toplevel)
        request_configuration(meta_wayland_surface_get_window(d->surface));
}

static const struct org_kde_kwin_server_decoration_interface kde_decoration_impl = {
    .release = destroy_resource,
    .request_mode = kde_request_mode,
};

static void kde_manager_create(struct wl_client* client, struct wl_resource* manager, uint32_t id,
                               struct wl_resource* surface_resource) {
    MetaWaylandSurface* surface = wl_resource_get_user_data(surface_resource);
    Decoration* d;

    if (!surface) {
        wl_resource_post_error(manager, WL_DISPLAY_ERROR_INVALID_OBJECT,
                               "Server decoration requires a live wl_surface");
        return;
    }

    if (surface->role && META_IS_WAYLAND_XDG_TOPLEVEL(surface->role) &&
        g_object_get_data(G_OBJECT(surface->role), "gnoblin-decoration")) {
        wl_resource_post_error(manager, WL_DISPLAY_ERROR_INVALID_OBJECT,
                               "Surface already has a decoration object");
        return;
    }

    d = g_new0(Decoration, 1);
    d->protocol = DECORATION_KDE;
    d->surface = g_object_ref(surface);
    d->resource = wl_resource_create(client, &org_kde_kwin_server_decoration_interface, 1, id);
    if (!d->resource) {
        g_clear_object(&d->surface);
        g_free(d);
        wl_client_post_no_memory(client);
        return;
    }

    d->surface_destroy.notify = surface_destroyed;
    wl_resource_add_destroy_listener(surface_resource, &d->surface_destroy);
    d->has_surface_listener = TRUE;
    d->preference = ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_SERVER;
    d->sent_mode = -1;
    wl_resource_set_implementation(d->resource, &kde_decoration_impl, d, decoration_free);

    if (surface->role) {
        d->role_assigned = TRUE;
        if (META_IS_WAYLAND_XDG_TOPLEVEL(surface->role))
            kde_decoration_attach_toplevel(d, META_WAYLAND_XDG_TOPLEVEL(surface->role));
        else
            kde_decoration_set_mode(d, ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_NONE);
    } else {
        g_object_set_data(G_OBJECT(surface), "gnoblin-decoration", d);
        kde_decoration_set_mode(d, ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_SERVER);
    }
}

static const struct org_kde_kwin_server_decoration_manager_interface kde_manager_impl = {
    .create = kde_manager_create,
};

static void bind_kde_manager(struct wl_client* client, void* data, uint32_t version, uint32_t id) {
    struct wl_resource* resource =
        wl_resource_create(client, &org_kde_kwin_server_decoration_manager_interface, 1, id);
    if (!resource) {
        wl_client_post_no_memory(client);
        return;
    }
    wl_resource_set_implementation(resource, &kde_manager_impl, NULL, NULL);
    org_kde_kwin_server_decoration_manager_send_default_mode(
        resource, ORG_KDE_KWIN_SERVER_DECORATION_MANAGER_MODE_SERVER);
}

void meta_gnoblin_window_frame_init(MetaWaylandCompositor* compositor) {
    meta_gnoblin_frame_renderer_init(compositor);
    g_signal_new("gnoblin-frame-changed", META_TYPE_WINDOW, G_SIGNAL_RUN_LAST, 0, NULL, NULL, NULL,
                 G_TYPE_NONE, 0);
    if (gnoblin_native_control_protocol_enabled("xdg-decoration"))
        wl_global_create(compositor->wayland_display, &zxdg_decoration_manager_v1_interface, 1,
                         NULL, bind_manager);
    if (gnoblin_native_control_protocol_enabled("kde-server-decoration"))
        wl_global_create(compositor->wayland_display,
                         &org_kde_kwin_server_decoration_manager_interface, 1, NULL,
                         bind_kde_manager);
}
