/* SPDX-License-Identifier: GPL-2.0-or-later */
/* ext-background-effect-v1: commit-synchronised, surface-local blur regions. */
#include "config.h"
#include "wayland/meta-wayland-background-effect.h"
#include "wayland/meta-wayland-private.h"
#include "wayland/meta-wayland-region.h"
#include "wayland/meta-wayland-surface-private.h"
#include "wayland/gnoblin-config.h"
#include "meta/meta-context.h"
#include "meta/display.h"
#include "meta/util.h"
#include "ext-background-effect-v1-server-protocol.h"

#define OBJECT_KEY "ext-background-effect-object"
#define REGION_KEY "ext-background-effect-region"

typedef struct {
    MetaWaylandSurface* surface;
    gulong destroy_handler;
} BackgroundSurface;

static void set_pending_region(MetaWaylandSurface* surface, MtkRegion* region) {
    MetaWaylandSurfaceState* pending = meta_wayland_surface_get_pending_state(surface);
    /* An empty region is an explicit change; NULL qdata means no new request. */
    g_object_set_data_full(G_OBJECT(pending), REGION_KEY,
                           region ? mtk_region_copy(region) : mtk_region_create(),
                           (GDestroyNotify)mtk_region_unref);
}

static void destroy_resource(struct wl_client* client, struct wl_resource* resource) {
    wl_resource_destroy(resource);
}

static void surface_destroyed(MetaWaylandSurface* surface, BackgroundSurface* effect) {
    effect->surface = NULL;
}

static void free_effect(struct wl_resource* resource) {
    BackgroundSurface* effect = wl_resource_get_user_data(resource);
    if (effect->surface) {
        set_pending_region(effect->surface, NULL);
        g_object_set_data(G_OBJECT(effect->surface), OBJECT_KEY, NULL);
        g_signal_handler_disconnect(effect->surface, effect->destroy_handler);
    }
    g_free(effect);
}

static void set_blur_region(struct wl_client* client, struct wl_resource* resource,
                            struct wl_resource* region_resource) {
    BackgroundSurface* effect = wl_resource_get_user_data(resource);
    if (!effect->surface) {
        wl_resource_post_error(resource, EXT_BACKGROUND_EFFECT_SURFACE_V1_ERROR_SURFACE_DESTROYED,
                               "Surface destroyed");
        return;
    }
    MetaWaylandRegion* region = region_resource ? wl_resource_get_user_data(region_resource) : NULL;
    set_pending_region(effect->surface, region ? meta_wayland_region_peek_region(region) : NULL);
}

static const struct ext_background_effect_surface_v1_interface surface_impl = {
    .destroy = destroy_resource,
    .set_blur_region = set_blur_region,
};

static void get_background_effect(struct wl_client* client, struct wl_resource* resource,
                                  uint32_t id, struct wl_resource* surface_resource) {
    MetaWaylandSurface* surface = wl_resource_get_user_data(surface_resource);
    if (g_object_get_data(G_OBJECT(surface), OBJECT_KEY)) {
        wl_resource_post_error(resource,
                               EXT_BACKGROUND_EFFECT_MANAGER_V1_ERROR_BACKGROUND_EFFECT_EXISTS,
                               "Surface already has a background effect object");
        return;
    }
    struct wl_resource* object =
        wl_resource_create(client, &ext_background_effect_surface_v1_interface, 1, id);
    if (!object) {
        wl_client_post_no_memory(client);
        return;
    }
    BackgroundSurface* effect = g_new0(BackgroundSurface, 1);
    effect->surface = surface;
    effect->destroy_handler =
        g_signal_connect(surface, "destroy", G_CALLBACK(surface_destroyed), effect);
    g_object_set_data(G_OBJECT(surface), OBJECT_KEY, effect);
    set_pending_region(surface, NULL);
    wl_resource_set_implementation(object, &surface_impl, effect, free_effect);
}

static const struct ext_background_effect_manager_v1_interface manager_impl = {
    .destroy = destroy_resource,
    .get_background_effect = get_background_effect,
};

static void bind_manager(struct wl_client* client, void* data, uint32_t version, uint32_t id) {
    struct wl_resource* resource =
        wl_resource_create(client, &ext_background_effect_manager_v1_interface, 1, id);
    if (!resource) {
        wl_client_post_no_memory(client);
        return;
    }
    wl_resource_set_implementation(resource, &manager_impl, NULL, NULL);
    ext_background_effect_manager_v1_send_capabilities(
        resource, EXT_BACKGROUND_EFFECT_MANAGER_V1_CAPABILITY_BLUR);
}

void meta_wayland_init_background_effect(MetaWaylandCompositor* compositor) {
    if (!gnoblin_config_protocol_enabled("ext-background-effect-v1"))
        return;
    g_signal_new("gnoblin-background-effect-changed", META_TYPE_DISPLAY, G_SIGNAL_RUN_LAST, 0, NULL,
                 NULL, NULL, G_TYPE_NONE, 1, CLUTTER_TYPE_ACTOR);
    if (!wl_global_create(compositor->wayland_display, &ext_background_effect_manager_v1_interface,
                          1, NULL, bind_manager))
        g_error("Failed to register ext-background-effect-v1");
}

void meta_wayland_background_effect_merge_state(MetaWaylandSurfaceState* from,
                                                MetaWaylandSurfaceState* to) {
    MtkRegion* region = g_object_steal_data(G_OBJECT(from), REGION_KEY);
    if (region)
        g_object_set_data_full(G_OBJECT(to), REGION_KEY, region, (GDestroyNotify)mtk_region_unref);
}

void meta_wayland_background_effect_apply_state(MetaWaylandSurface* surface,
                                                MetaWaylandSurfaceState* state) {
    MtkRegion* region = g_object_steal_data(G_OBJECT(state), REGION_KEY);
    if (region)
        g_object_set_data_full(G_OBJECT(surface), REGION_KEY, region,
                               (GDestroyNotify)mtk_region_unref);
    region = g_object_get_data(G_OBJECT(surface), REGION_KEY);
    ClutterActor* actor = CLUTTER_ACTOR(meta_wayland_surface_get_actor(surface));
    if (!actor || !region)
        return;
    MtkRegion* previous = g_object_get_data(G_OBJECT(actor), REGION_KEY);
    if (previous && mtk_region_equal(region, previous))
        return;
    g_object_set_data_full(G_OBJECT(actor), REGION_KEY, mtk_region_ref(region),
                           (GDestroyNotify)mtk_region_unref);
    MetaDisplay* display = meta_context_get_display(surface->compositor->context);
    g_signal_emit_by_name(display, "gnoblin-background-effect-changed", actor);
    clutter_actor_queue_redraw(actor);
}

/**
 * meta_gnoblin_background_effect_get_region:
 * @actor: a surface actor
 * Returns: (transfer full) (nullable): committed region in surface-local coordinates;
 *   an empty region explicitly disables blur, NULL means no standard request
 */
MtkRegion* meta_gnoblin_background_effect_get_region(ClutterActor* actor) {
    MtkRegion* region = g_object_get_data(G_OBJECT(actor), REGION_KEY);
    return region ? mtk_region_copy(region) : NULL;
}
