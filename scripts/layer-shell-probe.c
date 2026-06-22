#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include <wayland-client.h>

#include "wlr-layer-shell-unstable-v1-client-protocol.h"
#include "xdg-shell-client-protocol.h"

struct app
{
    struct wl_display *display;
    struct wl_registry *registry;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct wl_output *output;
    struct zwlr_layer_shell_v1 *layer_shell;
    struct xdg_wm_base *wm_base;
    uint32_t layer_shell_version;
};

struct layer_state
{
    bool configured;
    bool closed;
    uint32_t serial;
    uint32_t width;
    uint32_t height;
};

struct layer_client
{
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer_surface;
    struct wl_buffer *buffer;
    struct layer_state state;
};

struct xdg_state
{
    bool configured;
    uint32_t serial;
};

static uint32_t
min_u32(uint32_t a, uint32_t b)
{
    return a < b ? a : b;
}

static void
die(const char *message)
{
    fprintf(stderr, "layer-shell-probe: %s\n", message);
    exit(2);
}

static int
roundtrip_ok(struct app *app, const char *label)
{
    if (wl_display_roundtrip(app->display) >= 0)
        return 0;

    fprintf(stderr, "%s: display error %d\n",
            label, wl_display_get_error(app->display));
    return 1;
}

static int
expect_protocol_error(struct app *app, const char *label)
{
    for (int i = 0; i < 8; i++)
    {
        if (wl_display_roundtrip(app->display) < 0)
        {
            int error = wl_display_get_error(app->display);

            if (error == EPROTO)
            {
                fprintf(stderr, "%s: observed expected protocol error\n",
                        label);
                return 0;
            }

            fprintf(stderr, "%s: expected EPROTO, got display error %d\n",
                    label, error);
            return 1;
        }
    }

    fprintf(stderr, "%s: expected protocol error did not occur\n", label);
    return 1;
}

static int
create_anonymous_file(size_t size)
{
    const char *runtime_dir = getenv("XDG_RUNTIME_DIR");
    char path[PATH_MAX];
    int fd;

    if (!runtime_dir || runtime_dir[0] == '\0')
        runtime_dir = "/tmp";

    snprintf(path, sizeof path, "%s/gnoblin-layer-probe-XXXXXX", runtime_dir);

    fd = mkstemp(path);
    if (fd < 0)
        return -1;

    unlink(path);

    if (ftruncate(fd, (off_t)size) < 0)
    {
        close(fd);
        return -1;
    }

    return fd;
}

static struct wl_buffer *
create_buffer(struct app *app, int width, int height, uint32_t color)
{
    int stride = width * 4;
    size_t size = (size_t)stride * (size_t)height;
    int fd;
    uint32_t *data;
    struct wl_shm_pool *pool;
    struct wl_buffer *buffer;

    fd = create_anonymous_file(size);
    if (fd < 0)
        return NULL;

    data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (data == MAP_FAILED)
    {
        close(fd);
        return NULL;
    }

    for (size_t i = 0; i < size / 4; i++)
        data[i] = color;

    pool = wl_shm_create_pool(app->shm, fd, (int32_t)size);
    buffer = wl_shm_pool_create_buffer(pool, 0, width, height, stride,
                                       WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    munmap(data, size);
    close(fd);

    return buffer;
}

static void
handle_layer_configure(void *data,
                       struct zwlr_layer_surface_v1 *surface,
                       uint32_t serial,
                       uint32_t width,
                       uint32_t height)
{
    struct layer_state *state = data;

    (void)surface;

    state->configured = true;
    state->serial = serial;
    state->width = width;
    state->height = height;
}

static void
handle_layer_closed(void *data,
                    struct zwlr_layer_surface_v1 *surface)
{
    struct layer_state *state = data;

    (void)surface;

    state->closed = true;
}

static const struct zwlr_layer_surface_v1_listener layer_surface_listener = {
    .configure = handle_layer_configure,
    .closed = handle_layer_closed,
};

static void
handle_xdg_wm_base_ping(void *data,
                        struct xdg_wm_base *wm_base,
                        uint32_t serial)
{
    (void)data;

    xdg_wm_base_pong(wm_base, serial);
}

static const struct xdg_wm_base_listener wm_base_listener = {
    .ping = handle_xdg_wm_base_ping,
};

static void
handle_xdg_surface_configure(void *data,
                             struct xdg_surface *surface,
                             uint32_t serial)
{
    struct xdg_state *state = data;

    (void)surface;

    state->configured = true;
    state->serial = serial;
}

static const struct xdg_surface_listener xdg_surface_listener = {
    .configure = handle_xdg_surface_configure,
};

static void
handle_xdg_popup_configure(void *data,
                           struct xdg_popup *popup,
                           int32_t x,
                           int32_t y,
                           int32_t width,
                           int32_t height)
{
    (void)data;
    (void)popup;
    (void)x;
    (void)y;
    (void)width;
    (void)height;
}

static void
handle_xdg_popup_done(void *data,
                      struct xdg_popup *popup)
{
    (void)data;
    (void)popup;
}

static void
handle_xdg_popup_repositioned(void *data,
                              struct xdg_popup *popup,
                              uint32_t token)
{
    (void)data;
    (void)popup;
    (void)token;
}

static const struct xdg_popup_listener xdg_popup_listener = {
    .configure = handle_xdg_popup_configure,
    .popup_done = handle_xdg_popup_done,
    .repositioned = handle_xdg_popup_repositioned,
};

static void
handle_global(void *data,
              struct wl_registry *registry,
              uint32_t name,
              const char *interface,
              uint32_t version)
{
    struct app *app = data;

    if (strcmp(interface, wl_compositor_interface.name) == 0)
    {
        app->compositor =
            wl_registry_bind(registry, name, &wl_compositor_interface,
                             min_u32(version, 4));
    }
    else if (strcmp(interface, wl_shm_interface.name) == 0)
    {
        app->shm =
            wl_registry_bind(registry, name, &wl_shm_interface, 1);
    }
    else if (strcmp(interface, wl_output_interface.name) == 0 && !app->output)
    {
        app->output =
            wl_registry_bind(registry, name, &wl_output_interface, 1);
    }
    else if (strcmp(interface, zwlr_layer_shell_v1_interface.name) == 0)
    {
        app->layer_shell_version = min_u32(version, 5);
        app->layer_shell =
            wl_registry_bind(registry, name, &zwlr_layer_shell_v1_interface,
                             app->layer_shell_version);
    }
    else if (strcmp(interface, xdg_wm_base_interface.name) == 0)
    {
        app->wm_base =
            wl_registry_bind(registry, name, &xdg_wm_base_interface,
                             min_u32(version, 7));
        xdg_wm_base_add_listener(app->wm_base, &wm_base_listener, app);
    }
}

static void
handle_global_remove(void *data,
                     struct wl_registry *registry,
                     uint32_t name)
{
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {
    .global = handle_global,
    .global_remove = handle_global_remove,
};

static void
connect_app(struct app *app)
{
    app->display = wl_display_connect(NULL);
    if (!app->display)
        die("could not connect to WAYLAND_DISPLAY");

    app->registry = wl_display_get_registry(app->display);
    wl_registry_add_listener(app->registry, &registry_listener, app);

    if (roundtrip_ok(app, "registry roundtrip") != 0)
        exit(2);

    if (!app->compositor)
        die("wl_compositor is missing");
    if (!app->shm)
        die("wl_shm is missing");
    if (!app->layer_shell)
        die("zwlr_layer_shell_v1 is missing");
}

static void
disconnect_app(struct app *app)
{
    if (app->wm_base)
        xdg_wm_base_destroy(app->wm_base);
    if (app->layer_shell)
        zwlr_layer_shell_v1_destroy(app->layer_shell);
    if (app->output)
        wl_output_destroy(app->output);
    if (app->shm)
        wl_shm_destroy(app->shm);
    if (app->compositor)
        wl_compositor_destroy(app->compositor);
    if (app->registry)
        wl_registry_destroy(app->registry);
    if (app->display)
        wl_display_disconnect(app->display);
}

static void
create_layer_client_on_layer(struct app *app,
                             struct layer_client *client,
                             const char *namespace,
                             uint32_t layer)
{
    memset(client, 0, sizeof *client);

    client->surface = wl_compositor_create_surface(app->compositor);
    client->layer_surface =
        zwlr_layer_shell_v1_get_layer_surface(app->layer_shell,
                                              client->surface,
                                              app->output,
                                              layer,
                                              namespace);
    zwlr_layer_surface_v1_add_listener(client->layer_surface,
                                       &layer_surface_listener,
                                       &client->state);
}

static void
create_layer_client(struct app *app,
                    struct layer_client *client,
                    const char *namespace)
{
    create_layer_client_on_layer(app, client, namespace,
                                 ZWLR_LAYER_SHELL_V1_LAYER_TOP);
}

static void
set_bar_state(struct app *app, struct layer_client *client)
{
    zwlr_layer_surface_v1_set_size(client->layer_surface, 0, 28);
    zwlr_layer_surface_v1_set_anchor(client->layer_surface,
                                     ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                                         ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
                                         ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
    zwlr_layer_surface_v1_set_exclusive_zone(client->layer_surface, 28);
    zwlr_layer_surface_v1_set_keyboard_interactivity(
        client->layer_surface,
        ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE);

    if (app->layer_shell_version >= 5)
        zwlr_layer_surface_v1_set_exclusive_edge(
            client->layer_surface,
            ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP);
}

static void
set_center_overlay_state(struct layer_client *client)
{
    zwlr_layer_surface_v1_set_size(client->layer_surface, 520, 120);
    zwlr_layer_surface_v1_set_anchor(client->layer_surface, 0);
    zwlr_layer_surface_v1_set_keyboard_interactivity(
        client->layer_surface,
        ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE);
}

static void
set_background_state(struct layer_client *client)
{
    zwlr_layer_surface_v1_set_size(client->layer_surface, 0, 0);
    zwlr_layer_surface_v1_set_anchor(client->layer_surface,
                                     ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                                         ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM |
                                         ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
                                         ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
    zwlr_layer_surface_v1_set_exclusive_zone(client->layer_surface, -1);
    zwlr_layer_surface_v1_set_keyboard_interactivity(
        client->layer_surface,
        ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE);
}

static int
wait_layer_configure(struct app *app,
                     struct layer_client *client,
                     const char *label)
{
    for (int i = 0; i < 20; i++)
    {
        if (roundtrip_ok(app, label) != 0)
            return 1;
        if (client->state.configured)
            return 0;
    }

    fprintf(stderr, "%s: timed out waiting for layer configure\n", label);
    return 1;
}

static int
map_layer_client(struct app *app,
                 struct layer_client *client,
                 uint32_t color)
{
    int width;
    int height;

    client->state.configured = false;
    wl_surface_commit(client->surface);

    if (wait_layer_configure(app, client, "layer configure") != 0)
        return 1;

    zwlr_layer_surface_v1_ack_configure(client->layer_surface,
                                        client->state.serial);

    width = client->state.width ? (int)client->state.width : 320;
    height = client->state.height ? (int)client->state.height : 28;
    client->buffer = create_buffer(app, width, height, color);
    if (!client->buffer)
        die("could not create shm buffer");

    wl_surface_attach(client->surface, client->buffer, 0, 0);
    wl_surface_damage_buffer(client->surface, 0, 0, INT32_MAX, INT32_MAX);
    wl_surface_commit(client->surface);

    return roundtrip_ok(app, "map layer surface");
}

static void
destroy_layer_client(struct layer_client *client)
{
    if (client->buffer)
        wl_buffer_destroy(client->buffer);
    if (client->layer_surface)
        zwlr_layer_surface_v1_destroy(client->layer_surface);
    if (client->surface)
        wl_surface_destroy(client->surface);
}

static int
run_map(struct app *app)
{
    struct layer_client client;

    create_layer_client(app, &client, "gnoblin-probe-map");
    set_bar_state(app, &client);

    if (map_layer_client(app, &client, 0xff2f6fed) != 0)
        return 1;

    zwlr_layer_surface_v1_set_layer(client.layer_surface,
                                    ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY);
    wl_surface_commit(client.surface);
    if (roundtrip_ok(app, "set layer") != 0)
        return 1;

    wl_surface_attach(client.surface, NULL, 0, 0);
    wl_surface_commit(client.surface);
    if (roundtrip_ok(app, "unmap layer surface") != 0)
        return 1;

    if (client.buffer)
    {
        wl_buffer_destroy(client.buffer);
        client.buffer = NULL;
    }

    set_bar_state(app, &client);
    if (map_layer_client(app, &client, 0xff4b9b5f) != 0)
        return 1;

    destroy_layer_client(&client);
    return roundtrip_ok(app, "destroy mapped layer surface");
}

static int
run_background(struct app *app)
{
    struct layer_client client;

    create_layer_client_on_layer(app, &client, "gnoblin-probe-background",
                                 ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND);
    set_background_state(&client);

    if (map_layer_client(app, &client, 0xffcc2d6f) != 0)
        return 1;

    if (client.state.width < 320 || client.state.height < 240)
    {
        fprintf(stderr,
                "background-layer: expected output-sized configure, got %ux%u\n",
                client.state.width, client.state.height);
        destroy_layer_client(&client);
        return 1;
    }

    destroy_layer_client(&client);
    return roundtrip_ok(app, "destroy background layer surface");
}

static int
run_popup(struct app *app)
{
    struct layer_client parent;
    struct wl_surface *popup_surface;
    struct xdg_surface *xdg_surface;
    struct xdg_positioner *positioner;
    struct xdg_popup *popup;
    struct wl_buffer *popup_buffer;
    struct xdg_state xdg_state = {0};

    if (!app->wm_base)
        die("xdg_wm_base is missing");

    create_layer_client(app, &parent, "gnoblin-probe-popup-parent");
    set_bar_state(app, &parent);
    if (map_layer_client(app, &parent, 0xff7054c2) != 0)
        return 1;

    popup_surface = wl_compositor_create_surface(app->compositor);
    xdg_surface = xdg_wm_base_get_xdg_surface(app->wm_base, popup_surface);
    xdg_surface_add_listener(xdg_surface, &xdg_surface_listener, &xdg_state);

    positioner = xdg_wm_base_create_positioner(app->wm_base);
    xdg_positioner_set_size(positioner, 96, 40);
    xdg_positioner_set_anchor_rect(positioner, 0, 0, 96, 28);
    xdg_positioner_set_anchor(positioner, XDG_POSITIONER_ANCHOR_BOTTOM_LEFT);
    xdg_positioner_set_gravity(positioner, XDG_POSITIONER_GRAVITY_BOTTOM_LEFT);

    popup = xdg_surface_get_popup(xdg_surface, NULL, positioner);
    xdg_popup_add_listener(popup, &xdg_popup_listener, NULL);
    zwlr_layer_surface_v1_get_popup(parent.layer_surface, popup);
    xdg_positioner_destroy(positioner);

    wl_surface_commit(popup_surface);
    for (int i = 0; i < 20 && !xdg_state.configured; i++)
    {
        if (roundtrip_ok(app, "popup configure") != 0)
            return 1;
    }

    if (!xdg_state.configured)
    {
        fprintf(stderr, "popup: timed out waiting for xdg configure\n");
        return 1;
    }

    xdg_surface_ack_configure(xdg_surface, xdg_state.serial);
    popup_buffer = create_buffer(app, 96, 40, 0xffd97706);
    if (!popup_buffer)
        die("could not create popup shm buffer");

    wl_surface_attach(popup_surface, popup_buffer, 0, 0);
    wl_surface_damage_buffer(popup_surface, 0, 0, INT32_MAX, INT32_MAX);
    wl_surface_commit(popup_surface);

    if (roundtrip_ok(app, "map popup") != 0)
        return 1;

    wl_buffer_destroy(popup_buffer);
    xdg_popup_destroy(popup);
    xdg_surface_destroy(xdg_surface);
    wl_surface_destroy(popup_surface);
    destroy_layer_client(&parent);

    return roundtrip_ok(app, "destroy popup");
}

static struct zwlr_layer_surface_v1 *
new_invalid_layer_surface(struct app *app,
                          struct wl_surface **surface_out,
                          const char *namespace)
{
    struct wl_surface *surface = wl_compositor_create_surface(app->compositor);
    struct zwlr_layer_surface_v1 *layer_surface =
        zwlr_layer_shell_v1_get_layer_surface(app->layer_shell,
                                              surface,
                                              app->output,
                                              ZWLR_LAYER_SHELL_V1_LAYER_TOP,
                                              namespace);

    *surface_out = surface;
    return layer_surface;
}

static int
run_invalid_size(struct app *app)
{
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer_surface =
        new_invalid_layer_surface(app, &surface, "gnoblin-probe-invalid-size");

    (void)layer_surface;

    wl_surface_commit(surface);
    return expect_protocol_error(app, "invalid-size");
}

static int
run_invalid_anchor(struct app *app)
{
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer_surface =
        new_invalid_layer_surface(app, &surface, "gnoblin-probe-invalid-anchor");

    zwlr_layer_surface_v1_set_anchor(layer_surface, 1u << 12);
    return expect_protocol_error(app, "invalid-anchor");
}

static int
run_invalid_keyboard(struct app *app)
{
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer_surface =
        new_invalid_layer_surface(app, &surface, "gnoblin-probe-invalid-keyboard");

    zwlr_layer_surface_v1_set_keyboard_interactivity(layer_surface, 99);
    return expect_protocol_error(app, "invalid-keyboard");
}

static int
run_invalid_exclusive_edge(struct app *app)
{
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer_surface =
        new_invalid_layer_surface(app, &surface,
                                  "gnoblin-probe-invalid-exclusive-edge");

    zwlr_layer_surface_v1_set_size(layer_surface, 96, 28);
    zwlr_layer_surface_v1_set_anchor(layer_surface,
                                     ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP);
    zwlr_layer_surface_v1_set_exclusive_edge(
        layer_surface,
        ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM);
    wl_surface_commit(surface);

    return expect_protocol_error(app, "invalid-exclusive-edge");
}

static int
run_buffer_before_configure(struct app *app)
{
    struct wl_surface *surface;
    struct wl_buffer *buffer;
    struct zwlr_layer_surface_v1 *layer_surface =
        new_invalid_layer_surface(app, &surface,
                                  "gnoblin-probe-buffer-before-configure");

    zwlr_layer_surface_v1_set_size(layer_surface, 96, 28);
    zwlr_layer_surface_v1_set_anchor(layer_surface,
                                     ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                                         ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
                                         ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);

    buffer = create_buffer(app, 96, 28, 0xffcc3344);
    if (!buffer)
        die("could not create shm buffer");

    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_commit(surface);

    return expect_protocol_error(app, "buffer-before-configure");
}

static int
run_ack_before_configure(struct app *app)
{
    struct wl_surface *surface;
    struct zwlr_layer_surface_v1 *layer_surface =
        new_invalid_layer_surface(app, &surface,
                                  "gnoblin-probe-ack-before-configure");

    zwlr_layer_surface_v1_ack_configure(layer_surface, 1);
    return expect_protocol_error(app, "ack-before-configure");
}

static int
run_bad_serial_high(struct app *app)
{
    struct layer_client client;
    uint32_t serial;

    create_layer_client(app, &client, "gnoblin-probe-bad-serial-high");
    set_bar_state(app, &client);
    client.state.configured = false;
    wl_surface_commit(client.surface);

    if (wait_layer_configure(app, &client, "bad-serial-high configure") != 0)
        return 1;

    serial = client.state.serial + 1;
    zwlr_layer_surface_v1_ack_configure(client.layer_surface, serial);

    return expect_protocol_error(app, "bad-serial-high");
}

static int
run_unknown_serial_low(struct app *app)
{
    struct layer_client client;
    uint32_t serial;

    create_layer_client(app, &client, "gnoblin-probe-unknown-serial-low");
    set_bar_state(app, &client);
    client.state.configured = false;
    wl_surface_commit(client.surface);

    if (wait_layer_configure(app, &client, "unknown-serial-low configure") != 0)
        return 1;

    serial = 0;
    zwlr_layer_surface_v1_ack_configure(client.layer_surface, serial);

    return expect_protocol_error(app, "unknown-serial-low");
}

static int
run_commit_after_later_configure(struct app *app)
{
    struct layer_client client;
    struct wl_buffer *replacement;
    int width;
    int height;

    create_layer_client(app, &client, "gnoblin-probe-commit-after-later-configure");
    set_bar_state(app, &client);
    client.state.configured = false;
    wl_surface_commit(client.surface);

    if (wait_layer_configure(app, &client, "commit-after-later-configure initial") != 0)
        return 1;

    zwlr_layer_surface_v1_ack_configure(client.layer_surface,
                                        client.state.serial);

    width = client.state.width ? (int)client.state.width : 320;
    height = client.state.height ? (int)client.state.height : 28;
    client.buffer = create_buffer(app, width, height, 0xff4b9b5f);
    if (!client.buffer)
        die("could not create first shm buffer");

    wl_surface_attach(client.surface, client.buffer, 0, 0);
    wl_surface_damage_buffer(client.surface, 0, 0, INT32_MAX, INT32_MAX);
    wl_surface_commit(client.surface);
    if (roundtrip_ok(app, "commit-after-later-configure initial map") != 0)
        return 1;

    /* Trigger a new configure, but intentionally do not ack it before
     * committing the already-acked response. A client can receive a newer
     * configure while still rendering the buffer for an older acked configure;
     * the older ack remains a valid response for this commit. */
    client.state.configured = false;
    zwlr_layer_surface_v1_set_size(client.layer_surface, 0, 40);
    wl_surface_commit(client.surface);
    if (wait_layer_configure(app, &client, "commit-after-later-configure second") != 0)
        return 1;

    replacement = create_buffer(app, width, height, 0xffd97706);
    if (!replacement)
        die("could not create replacement shm buffer");

    wl_surface_attach(client.surface, replacement, 0, 0);
    wl_surface_damage_buffer(client.surface, 0, 0, INT32_MAX, INT32_MAX);
    wl_surface_commit(client.surface);
    if (roundtrip_ok(app, "commit-after-later-configure old ack commit") != 0)
        return 1;

    wl_buffer_destroy(replacement);
    destroy_layer_client(&client);
    return roundtrip_ok(app, "destroy commit-after-later-configure");
}

static int
run_state_change_configure(struct app *app)
{
    struct layer_client client;
    uint32_t initial_serial;

    create_layer_client(app, &client, "gnoblin-probe-state-change-configure");
    set_bar_state(app, &client);

    if (map_layer_client(app, &client, 0xff22c55e) != 0)
        return 1;

    initial_serial = client.state.serial;
    client.state.configured = false;

    zwlr_layer_surface_v1_set_margin(client.layer_surface, 12, 0, 0, 0);
    wl_surface_commit(client.surface);

    if (wait_layer_configure(app, &client, "state-change-configure") != 0)
        return 1;

    if (client.state.serial == initial_serial)
    {
        fprintf(stderr, "state-change-configure: configure serial did not change\n");
        return 1;
    }

    zwlr_layer_surface_v1_ack_configure(client.layer_surface,
                                        client.state.serial);
    wl_surface_commit(client.surface);

    if (roundtrip_ok(app, "state-change-configure ack commit") != 0)
        return 1;

    destroy_layer_client(&client);
    return roundtrip_ok(app, "destroy state-change-configure");
}

static int
run_null_before_first_map_resets(struct app *app)
{
    struct layer_client client;
    struct wl_buffer *buffer;
    int width;
    int height;

    create_layer_client(app, &client, "gnoblin-probe-null-before-first-map-resets");
    set_bar_state(app, &client);
    client.state.configured = false;
    wl_surface_commit(client.surface);

    if (wait_layer_configure(app, &client, "null-before-first-map initial") != 0)
        return 1;

    zwlr_layer_surface_v1_ack_configure(client.layer_surface,
                                        client.state.serial);

    wl_surface_attach(client.surface, NULL, 0, 0);
    wl_surface_commit(client.surface);
    if (roundtrip_ok(app, "null-before-first-map null commit") != 0)
        return 1;

    width = client.state.width ? (int)client.state.width : 320;
    height = client.state.height ? (int)client.state.height : 28;
    buffer = create_buffer(app, width, height, 0xffcc3344);
    if (!buffer)
        die("could not create shm buffer");

    /* If the null attach reset the layer-surface as the protocol describes,
     * this direct buffer commit is invalid: the client must first perform a
     * new bufferless commit, receive configure, and ack it. */
    wl_surface_attach(client.surface, buffer, 0, 0);
    wl_surface_damage_buffer(client.surface, 0, 0, INT32_MAX, INT32_MAX);
    wl_surface_commit(client.surface);

    return expect_protocol_error(app, "null-before-first-map-resets");
}

static int
run_wait_closed(struct app *app)
{
    struct layer_client client;
    const char *timeout_env = getenv("PROBE_WAIT_CLOSED_SECONDS");
    int timeout_seconds = 8;
    int width;
    int height;

    if (!app->output)
        die("wl_output is missing");

    if (timeout_env && timeout_env[0] != '\0')
    {
        char *end = NULL;
        long parsed = strtol(timeout_env, &end, 10);

        if (end && *end == '\0' && parsed > 0 && parsed < INT_MAX)
            timeout_seconds = (int)parsed;
    }

    create_layer_client_on_layer(app, &client, "gnoblin-probe-wait-closed",
                                 ZWLR_LAYER_SHELL_V1_LAYER_TOP);
    set_bar_state(app, &client);
    client.state.configured = false;
    wl_surface_commit(client.surface);

    if (wait_layer_configure(app, &client, "wait-closed configure") != 0)
        return 1;

    zwlr_layer_surface_v1_ack_configure(client.layer_surface,
                                        client.state.serial);

    width = client.state.width ? (int)client.state.width : 320;
    height = client.state.height ? (int)client.state.height : 28;
    client.buffer = create_buffer(app, width, height, 0xff3b82f6);
    if (!client.buffer)
        die("could not create wait-closed shm buffer");

    wl_surface_attach(client.surface, client.buffer, 0, 0);
    wl_surface_damage_buffer(client.surface, 0, 0, INT32_MAX, INT32_MAX);
    wl_surface_commit(client.surface);

    if (roundtrip_ok(app, "wait-closed map") != 0)
        return 1;

    fprintf(stderr, "wait-closed: mapped\n");
    fflush(stderr);

    for (int i = 0; i < timeout_seconds * 10; i++)
    {
        if (roundtrip_ok(app, "wait-closed poll") != 0)
            return 1;
        if (client.state.closed)
        {
            fprintf(stderr, "wait-closed: observed closed event\n");
            destroy_layer_client(&client);
            return roundtrip_ok(app, "wait-closed destroy");
        }
        usleep(100000);
    }

    fprintf(stderr, "wait-closed: timed out waiting for closed event\n");
    destroy_layer_client(&client);
    return 1;
}

static int
run_closed_ignores_requests(struct app *app)
{
    struct layer_client client;
    const char *timeout_env = getenv("PROBE_WAIT_CLOSED_SECONDS");
    int timeout_seconds = 8;

    if (!app->output)
        die("wl_output is missing");

    if (timeout_env && timeout_env[0] != '\0')
    {
        char *end = NULL;
        long parsed = strtol(timeout_env, &end, 10);

        if (end && *end == '\0' && parsed > 0 && parsed < INT_MAX)
            timeout_seconds = (int)parsed;
    }

    create_layer_client_on_layer(app, &client, "gnoblin-probe-closed-ignore",
                                 ZWLR_LAYER_SHELL_V1_LAYER_TOP);
    set_bar_state(app, &client);

    if (map_layer_client(app, &client, 0xff2563eb) != 0)
        return 1;

    fprintf(stderr, "closed-ignore: mapped\n");
    fflush(stderr);

    for (int i = 0; i < timeout_seconds * 10; i++)
    {
        if (roundtrip_ok(app, "closed-ignore poll") != 0)
            return 1;
        if (client.state.closed)
        {
            fprintf(stderr, "closed-ignore: observed closed event\n");

            zwlr_layer_surface_v1_ack_configure(client.layer_surface,
                                                client.state.serial);
            zwlr_layer_surface_v1_set_size(client.layer_surface, 0, 40);
            zwlr_layer_surface_v1_set_anchor(
                client.layer_surface,
                ZWLR_LAYER_SURFACE_V1_ANCHOR_BOTTOM |
                    ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT |
                    ZWLR_LAYER_SURFACE_V1_ANCHOR_RIGHT);
            wl_surface_damage_buffer(client.surface, 0, 0,
                                     INT32_MAX, INT32_MAX);
            wl_surface_commit(client.surface);

            if (roundtrip_ok(app, "closed-ignore post-close requests") != 0)
                return 1;

            fprintf(stderr, "closed-ignore: post-close requests ignored\n");
            destroy_layer_client(&client);
            return roundtrip_ok(app, "closed-ignore destroy");
        }
        usleep(100000);
    }

    fprintf(stderr, "closed-ignore: timed out waiting for closed event\n");
    destroy_layer_client(&client);
    return 1;
}

static int
run_invalid_layer(struct app *app)
{
    struct wl_surface *surface = wl_compositor_create_surface(app->compositor);

    zwlr_layer_shell_v1_get_layer_surface(app->layer_shell,
                                          surface,
                                          app->output,
                                          99,
                                          "gnoblin-probe-invalid-layer");

    return expect_protocol_error(app, "invalid-layer");
}

static int
run_pending_buffer_before_get_layer_surface(struct app *app)
{
    struct wl_surface *surface = wl_compositor_create_surface(app->compositor);
    struct wl_buffer *buffer;

    buffer = create_buffer(app, 96, 28, 0xffcc3344);
    if (!buffer)
        die("could not create shm buffer");

    wl_surface_attach(surface, buffer, 0, 0);
    zwlr_layer_shell_v1_get_layer_surface(
        app->layer_shell,
        surface,
        app->output,
        ZWLR_LAYER_SHELL_V1_LAYER_TOP,
        "gnoblin-probe-pending-buffer-before-get-layer-surface");

    return expect_protocol_error(app, "pending-buffer-before-get-layer-surface");
}

static int
run_committed_before_get_layer_surface(struct app *app)
{
    struct wl_surface *surface = wl_compositor_create_surface(app->compositor);

    wl_surface_commit(surface);
    zwlr_layer_shell_v1_get_layer_surface(
        app->layer_shell,
        surface,
        app->output,
        ZWLR_LAYER_SHELL_V1_LAYER_TOP,
        "gnoblin-probe-committed-before-get-layer-surface");

    return expect_protocol_error(app, "committed-before-get-layer-surface");
}

static int
run_hold_overlay(struct app *app)
{
    struct layer_client client;
    const char *hold_seconds_env = getenv("PROBE_HOLD_SECONDS");
    int hold_seconds = 300;

    if (hold_seconds_env && hold_seconds_env[0] != '\0')
    {
        char *end = NULL;
        long parsed = strtol(hold_seconds_env, &end, 10);

        if (end && *end == '\0' && parsed > 0 && parsed < INT_MAX)
            hold_seconds = (int)parsed;
    }

    create_layer_client_on_layer(app, &client, "gnoblin-visible-overlay",
                                 ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY);
    set_center_overlay_state(&client);

    if (map_layer_client(app, &client, 0xffff00ff) != 0)
        return 1;

    fprintf(stderr, "hold-overlay: mapped bright overlay for %d seconds\n",
            hold_seconds);

    for (int i = 0; i < hold_seconds * 10; i++)
    {
        if (wl_display_dispatch_pending(app->display) < 0)
            return 1;
        if (client.state.closed)
            break;
        wl_display_flush(app->display);
        usleep(100000);
    }

    destroy_layer_client(&client);
    return roundtrip_ok(app, "destroy hold overlay");
}

static int
run_mode(struct app *app, const char *mode)
{
    if (strcmp(mode, "map") == 0)
        return run_map(app);
    if (strcmp(mode, "background") == 0)
        return run_background(app);
    if (strcmp(mode, "popup") == 0)
        return run_popup(app);
    if (strcmp(mode, "invalid-size") == 0)
        return run_invalid_size(app);
    if (strcmp(mode, "invalid-anchor") == 0)
        return run_invalid_anchor(app);
    if (strcmp(mode, "invalid-keyboard") == 0)
        return run_invalid_keyboard(app);
    if (strcmp(mode, "invalid-exclusive-edge") == 0)
        return run_invalid_exclusive_edge(app);
    if (strcmp(mode, "buffer-before-configure") == 0)
        return run_buffer_before_configure(app);
    if (strcmp(mode, "ack-before-configure") == 0)
        return run_ack_before_configure(app);
    if (strcmp(mode, "bad-serial-high") == 0)
        return run_bad_serial_high(app);
    if (strcmp(mode, "unknown-serial-low") == 0)
        return run_unknown_serial_low(app);
    if (strcmp(mode, "commit-after-later-configure") == 0)
        return run_commit_after_later_configure(app);
    if (strcmp(mode, "state-change-configure") == 0)
        return run_state_change_configure(app);
    if (strcmp(mode, "null-before-first-map-resets") == 0)
        return run_null_before_first_map_resets(app);
    if (strcmp(mode, "wait-closed") == 0)
        return run_wait_closed(app);
    if (strcmp(mode, "closed-ignores-requests") == 0)
        return run_closed_ignores_requests(app);
    if (strcmp(mode, "invalid-layer") == 0)
        return run_invalid_layer(app);
    if (strcmp(mode, "pending-buffer-before-get-layer-surface") == 0)
        return run_pending_buffer_before_get_layer_surface(app);
    if (strcmp(mode, "committed-before-get-layer-surface") == 0)
        return run_committed_before_get_layer_surface(app);
    if (strcmp(mode, "hold-overlay") == 0)
        return run_hold_overlay(app);

    fprintf(stderr, "unknown probe mode: %s\n", mode);
    return 2;
}

int main(int argc, char **argv)
{
    struct app app = {0};
    int status;

    if (argc != 2)
    {
        fprintf(stderr, "usage: %s MODE\n", argv[0]);
        return 2;
    }

    connect_app(&app);
    status = run_mode(&app, argv[1]);
    disconnect_app(&app);

    return status;
}
