#include <assert.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-client.h>
#include "blur-fade-client.h"

static struct wl_compositor *compositor;
static struct gnoblin_blur_fade_manager_v1 *manager;

static void
global (void *data, struct wl_registry *registry, uint32_t name,
        const char *interface, uint32_t version)
{
    if (strcmp (interface, "wl_compositor") == 0)
        compositor = wl_registry_bind (registry, name, &wl_compositor_interface, 1);
    if (strcmp (interface, "gnoblin_blur_fade_manager_v1") == 0)
        manager = wl_registry_bind (registry, name, &gnoblin_blur_fade_manager_v1_interface, 1);
}

static void removed (void *data, struct wl_registry *registry, uint32_t name) {}
static const struct wl_registry_listener listener = { global, removed };

int
main (int argc, char **argv)
{
    struct wl_display *display = wl_display_connect (NULL);
    assert (display && argc == 2);
    struct wl_registry *registry = wl_display_get_registry (display);
    wl_registry_add_listener (registry, &listener, NULL);
    assert (wl_display_roundtrip (display) >= 0);
    if (strcmp (argv[1], "absent") == 0)
    {
        assert (!manager);
        puts ("PASS: blur fade protocol is absent outside Gnoblin");
        wl_display_disconnect (display);
        return 0;
    }
    assert (manager && compositor);
    struct wl_surface *surface = wl_compositor_create_surface (compositor);
    int32_t values[33 * 5] = {0, 0, 100 * 256, 100 * 256, 128};
    size_t length = 5 * sizeof (int32_t);
    int invalid = strcmp (argv[1], "valid") != 0;
    if (strcmp (argv[1], "opacity") == 0) values[4] = 257;
    if (strcmp (argv[1], "size") == 0) values[2] = -1;
    if (strcmp (argv[1], "coordinate") == 0) values[0] = INT32_MAX;
    if (strcmp (argv[1], "length") == 0) length = 1;
    if (strcmp (argv[1], "count") == 0) length = sizeof (values);
    struct wl_array array = { length, 0, values };
    gnoblin_blur_fade_manager_v1_set_fades (manager, surface, &array);
    wl_surface_commit (surface);
    int result = wl_display_roundtrip (display);
    if (invalid)
    {
        assert (result == -1 && wl_display_get_error (display) == EPROTO);
        const struct wl_interface *interface;
        uint32_t id;
        assert (wl_display_get_protocol_error (display, &interface, &id) ==
                GNOBLIN_BLUR_FADE_MANAGER_V1_ERROR_INVALID_FADES);
        assert (interface == &gnoblin_blur_fade_manager_v1_interface);
    }
    else
    {
        assert (result >= 0);
        array.size = 0;
        gnoblin_blur_fade_manager_v1_set_fades (manager, surface, &array);
        wl_surface_commit (surface);
        assert (wl_display_roundtrip (display) >= 0);
        wl_surface_destroy (surface);
        gnoblin_blur_fade_manager_v1_destroy (manager);
    }
    printf ("PASS: blur fade protocol %s\n", argv[1]);
    wl_display_disconnect (display);
    return 0;
}
