#define _GNU_SOURCE

/* Minimal private frame-renderer fixture: acknowledge, but never paint.
 * Build with a client header/code generated from gnoblin-window-frame-v1.xml. */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>

#include "gnoblin-window-frame-v1-client-protocol.h"

static struct wl_compositor* compositor;
static struct gnoblin_window_frame_manager_v1* manager;
static const char* ready_path;

static void write_ready(void) {
    int fd = open(ready_path, O_CREAT | O_WRONLY | O_TRUNC | O_CLOEXEC, 0600);
    if (fd < 0 || write(fd, "configured\n", 11) != 11)
        exit(3);
    close(fd);
}

static void frame_configure(void* data, struct gnoblin_window_frame_v1* frame, uint32_t serial,
                            int32_t width, int32_t height, int32_t top, int32_t right,
                            int32_t bottom, int32_t left, uint32_t state, const char* title,
                            const char* app_id, const char* style) {
    gnoblin_window_frame_v1_ack_configure(frame, serial);
    write_ready();
}

static void frame_interaction(void* data, struct gnoblin_window_frame_v1* frame, uint32_t action,
                              uint32_t pressed) {}

static void frame_closed(void* data, struct gnoblin_window_frame_v1* frame) {
    gnoblin_window_frame_v1_destroy(frame);
}

static const struct gnoblin_window_frame_v1_listener frame_listener = {
    .configure = frame_configure,
    .interaction = frame_interaction,
    .closed = frame_closed,
};

static void manager_frame(void* data, struct gnoblin_window_frame_manager_v1* manager,
                          struct gnoblin_window_frame_v1* frame) {
    struct wl_surface* surface = wl_compositor_create_surface(compositor);
    if (!surface)
        exit(2);
    gnoblin_window_frame_v1_add_listener(frame, &frame_listener, NULL);
    gnoblin_window_frame_v1_attach_surface(frame, surface);
    /* No wl_surface.attach or wl_surface.commit follows: this is the test. */
}

static const struct gnoblin_window_frame_manager_v1_listener manager_listener = {
    .frame = manager_frame,
};

static void registry_global(void* data, struct wl_registry* registry, uint32_t name,
                            const char* interface, uint32_t version) {
    if (strcmp(interface, wl_compositor_interface.name) == 0)
        compositor = wl_registry_bind(registry, name, &wl_compositor_interface, 4);
    else if (strcmp(interface, gnoblin_window_frame_manager_v1_interface.name) == 0) {
        manager = wl_registry_bind(registry, name, &gnoblin_window_frame_manager_v1_interface, 1);
        gnoblin_window_frame_manager_v1_add_listener(manager, &manager_listener, NULL);
    }
}

static void registry_global_remove(void* data, struct wl_registry* registry, uint32_t name) {}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_global_remove,
};

int main(int argc, char** argv) {
    if (argc != 3 || strcmp(argv[1], "--ready") != 0) {
        fprintf(stderr, "usage: %s --ready PATH\n", argv[0]);
        return 2;
    }
    ready_path = argv[2];
    struct wl_display* display = wl_display_connect(NULL);
    if (!display)
        return 1;
    struct wl_registry* registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !compositor || !manager)
        return 1;
    while (wl_display_dispatch(display) >= 0)
        ;
    return 0;
}
