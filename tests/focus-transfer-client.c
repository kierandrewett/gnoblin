#define _GNU_SOURCE

/* Minimal Wayland client that requests activation without input context. */
#include <wayland-client.h>

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include "xdg-activation-v1-client-protocol.h"
#include "xdg-shell-client-protocol.h"

typedef struct {
    struct wl_display* display;
    struct wl_registry* registry;
    struct wl_compositor* compositor;
    struct wl_shm* shm;
    struct xdg_wm_base* wm_base;
    struct xdg_activation_v1* activation;
    struct wl_surface* surface;
    struct wl_buffer* buffer;
    struct xdg_surface* xdg_surface;
    struct xdg_toplevel* toplevel;
    const char* request_path;
    int running;
    int activation_requested;
} Client;

static void draw_surface(Client* client) {
    const int width = 400;
    const int height = 240;
    const int stride = width * 4;
    const int size = height * stride;
    const char* runtime_dir = getenv("XDG_RUNTIME_DIR");
    char path[4096];
    int fd;
    uint32_t* pixels;
    struct wl_shm_pool* pool;

    if (client->buffer)
        return;
    if (!runtime_dir || snprintf(path, sizeof(path), "%s/gnoblin-focus-%ld", runtime_dir,
                                 (long)getpid()) >= (int)sizeof(path))
        exit(20);

    fd = open(path, O_CREAT | O_EXCL | O_RDWR | O_CLOEXEC, 0600);
    if (fd < 0 || unlink(path) != 0 || ftruncate(fd, size) != 0)
        exit(21);
    pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (pixels == MAP_FAILED)
        exit(22);
    for (int i = 0; i < width * height; i++)
        pixels[i] = 0xff303840;
    munmap(pixels, size);

    pool = wl_shm_create_pool(client->shm, fd, size);
    client->buffer =
        wl_shm_pool_create_buffer(pool, 0, width, height, stride, WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);

    wl_surface_attach(client->surface, client->buffer, 0, 0);
    wl_surface_damage(client->surface, 0, 0, width, height);
    wl_surface_commit(client->surface);
}

static void xdg_surface_configure(void* data, struct xdg_surface* surface, uint32_t serial) {
    Client* client = data;
    xdg_surface_ack_configure(surface, serial);
    draw_surface(client);
}

static const struct xdg_surface_listener xdg_surface_listener = {
    .configure = xdg_surface_configure,
};

static void toplevel_configure(void* data, struct xdg_toplevel* toplevel, int32_t width,
                               int32_t height, struct wl_array* states) {}

static void toplevel_close(void* data, struct xdg_toplevel* toplevel) {
    Client* client = data;
    client->running = 0;
}

static void toplevel_configure_bounds(void* data, struct xdg_toplevel* toplevel, int32_t width,
                                      int32_t height) {}

static void toplevel_wm_capabilities(void* data, struct xdg_toplevel* toplevel,
                                     struct wl_array* capabilities) {}

static const struct xdg_toplevel_listener toplevel_listener = {
    .configure = toplevel_configure,
    .close = toplevel_close,
    .configure_bounds = toplevel_configure_bounds,
    .wm_capabilities = toplevel_wm_capabilities,
};

static void wm_base_ping(void* data, struct xdg_wm_base* wm_base, uint32_t serial) {
    xdg_wm_base_pong(wm_base, serial);
}

static const struct xdg_wm_base_listener wm_base_listener = {
    .ping = wm_base_ping,
};

static void token_done(void* data, struct xdg_activation_token_v1* token, const char* value) {
    Client* client = data;
    if (value && *value) {
        xdg_activation_v1_activate(client->activation, value, client->surface);
        wl_display_flush(client->display);
    }
    xdg_activation_token_v1_destroy(token);
}

static const struct xdg_activation_token_v1_listener token_listener = {
    .done = token_done,
};

static void registry_global(void* data, struct wl_registry* registry, uint32_t name,
                            const char* interface, uint32_t version) {
    Client* client = data;

    if (strcmp(interface, wl_compositor_interface.name) == 0) {
        client->compositor =
            wl_registry_bind(registry, name, &wl_compositor_interface, version < 4 ? version : 4);
    } else if (strcmp(interface, wl_shm_interface.name) == 0) {
        client->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    } else if (strcmp(interface, xdg_wm_base_interface.name) == 0) {
        client->wm_base =
            wl_registry_bind(registry, name, &xdg_wm_base_interface, version < 6 ? version : 6);
        xdg_wm_base_add_listener(client->wm_base, &wm_base_listener, client);
    } else if (strcmp(interface, xdg_activation_v1_interface.name) == 0) {
        client->activation = wl_registry_bind(registry, name, &xdg_activation_v1_interface, 1);
    }
}

static void registry_remove(void* data, struct wl_registry* registry, uint32_t name) {}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_remove,
};

static void request_activation(Client* client) {
    struct xdg_activation_token_v1* token;

    client->activation_requested = 1;
    unlink(client->request_path);
    token = xdg_activation_v1_get_activation_token(client->activation);
    xdg_activation_token_v1_add_listener(token, &token_listener, client);
    /* Deliberately omit a surface, serial, and app ID. */
    xdg_activation_token_v1_commit(token);
    wl_display_flush(client->display);
}

static int initialize(Client* client) {
    client->display = wl_display_connect(NULL);
    if (!client->display)
        return 10;
    client->registry = wl_display_get_registry(client->display);
    wl_registry_add_listener(client->registry, &registry_listener, client);
    if (wl_display_roundtrip(client->display) < 0 || !client->compositor || !client->shm ||
        !client->wm_base || !client->activation)
        return 11;

    client->surface = wl_compositor_create_surface(client->compositor);
    client->xdg_surface = xdg_wm_base_get_xdg_surface(client->wm_base, client->surface);
    xdg_surface_add_listener(client->xdg_surface, &xdg_surface_listener, client);
    client->toplevel = xdg_surface_get_toplevel(client->xdg_surface);
    xdg_toplevel_add_listener(client->toplevel, &toplevel_listener, client);
    xdg_toplevel_set_title(client->toplevel, "Untrusted Activation Target");
    xdg_toplevel_set_app_id(client->toplevel, "org.gnoblin.FocusTransferTest");
    wl_surface_commit(client->surface);
    if (wl_display_roundtrip(client->display) < 0 || !client->buffer)
        return 12;
    return 0;
}

static void cleanup(Client* client) {
    if (client->buffer)
        wl_buffer_destroy(client->buffer);
    if (client->toplevel)
        xdg_toplevel_destroy(client->toplevel);
    if (client->xdg_surface)
        xdg_surface_destroy(client->xdg_surface);
    if (client->surface)
        wl_surface_destroy(client->surface);
    if (client->activation)
        xdg_activation_v1_destroy(client->activation);
    if (client->wm_base)
        xdg_wm_base_destroy(client->wm_base);
    if (client->shm)
        wl_shm_destroy(client->shm);
    if (client->compositor)
        wl_compositor_destroy(client->compositor);
    if (client->registry)
        wl_registry_destroy(client->registry);
    if (client->display)
        wl_display_disconnect(client->display);
}

int main(int argc, char** argv) {
    Client client = {0};
    struct pollfd display_fd;
    int result;

    if (argc != 2)
        return 2;
    client.request_path = argv[1];
    client.running = 1;
    result = initialize(&client);
    if (result != 0) {
        cleanup(&client);
        return result;
    }

    display_fd.fd = wl_display_get_fd(client.display);
    display_fd.events = POLLIN;
    while (client.running) {
        if (!client.activation_requested && access(client.request_path, F_OK) == 0)
            request_activation(&client);
        if (wl_display_flush(client.display) < 0 && errno != EAGAIN)
            break;
        display_fd.revents = 0;
        result = poll(&display_fd, 1, 25);
        if (result < 0 && errno == EINTR)
            continue;
        if (result < 0 || (display_fd.revents & (POLLERR | POLLHUP | POLLNVAL)))
            break;
        if ((display_fd.revents & POLLIN) && wl_display_dispatch(client.display) < 0)
            break;
    }

    cleanup(&client);
    return 0;
}
