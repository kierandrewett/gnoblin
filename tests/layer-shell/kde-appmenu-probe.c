#define _GNU_SOURCE

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

#include "kde-appmenu-client-protocol.h"
#include "xdg-shell-client-protocol.h"

struct app
{
    struct wl_display *display;
    struct wl_registry *registry;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct xdg_wm_base *wm_base;
    struct org_kde_kwin_appmenu_manager *appmenu_manager;
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
    fprintf(stderr, "kde-appmenu-probe: %s\n", message);
    exit(2);
}

static int
roundtrip_ok(struct app *app, const char *label)
{
    if (wl_display_roundtrip(app->display) >= 0)
        return 0;

    fprintf(stderr, "%s: display error %d\n", label, wl_display_get_error(app->display));
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

    snprintf(path, sizeof path, "%s/gnoblin-kde-appmenu-probe-XXXXXX", runtime_dir);

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
handle_wm_base_ping(void *data, struct xdg_wm_base *wm_base, uint32_t serial)
{
    (void)data;
    xdg_wm_base_pong(wm_base, serial);
}

static const struct xdg_wm_base_listener wm_base_listener = {
    .ping = handle_wm_base_ping,
};

static void
handle_xdg_surface_configure(void *data, struct xdg_surface *surface, uint32_t serial)
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
handle_toplevel_configure(void *data,
                          struct xdg_toplevel *toplevel,
                          int32_t width,
                          int32_t height,
                          struct wl_array *states)
{
    (void)data;
    (void)toplevel;
    (void)width;
    (void)height;
    (void)states;
}

static void
handle_toplevel_close(void *data, struct xdg_toplevel *toplevel)
{
    (void)data;
    (void)toplevel;
}

static void
handle_toplevel_configure_bounds(void *data,
                                 struct xdg_toplevel *toplevel,
                                 int32_t width,
                                 int32_t height)
{
    (void)data;
    (void)toplevel;
    (void)width;
    (void)height;
}

static void
handle_toplevel_wm_capabilities(void *data,
                                struct xdg_toplevel *toplevel,
                                struct wl_array *capabilities)
{
    (void)data;
    (void)toplevel;
    (void)capabilities;
}

static const struct xdg_toplevel_listener toplevel_listener = {
    .configure = handle_toplevel_configure,
    .close = handle_toplevel_close,
    .configure_bounds = handle_toplevel_configure_bounds,
    .wm_capabilities = handle_toplevel_wm_capabilities,
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
        app->compositor = wl_registry_bind(registry, name, &wl_compositor_interface,
                                           min_u32(version, 4));
    }
    else if (strcmp(interface, wl_shm_interface.name) == 0)
    {
        app->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    }
    else if (strcmp(interface, xdg_wm_base_interface.name) == 0)
    {
        app->wm_base = wl_registry_bind(registry, name, &xdg_wm_base_interface,
                                        min_u32(version, 7));
        xdg_wm_base_add_listener(app->wm_base, &wm_base_listener, app);
    }
    else if (strcmp(interface, org_kde_kwin_appmenu_manager_interface.name) == 0)
    {
        app->appmenu_manager =
            wl_registry_bind(registry, name, &org_kde_kwin_appmenu_manager_interface,
                             min_u32(version, 2));
    }
}

static void
handle_global_remove(void *data, struct wl_registry *registry, uint32_t name)
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
    if (!app->wm_base)
        die("xdg_wm_base is missing");
    if (!app->appmenu_manager)
        die("org_kde_kwin_appmenu_manager is missing");
}

int
main(int argc, char **argv)
{
    struct app app = {0};
    struct xdg_state xdg_state = {0};
    const char *bus = argc > 1 ? argv[1] : "org.gnoblin.TestAppMenu";
    const char *path = argc > 2 ? argv[2] : "/org/gnoblin/TestAppMenu";
    struct wl_surface *surface;
    struct xdg_surface *xdg_surface;
    struct xdg_toplevel *toplevel;
    struct wl_buffer *buffer;
    struct org_kde_kwin_appmenu *appmenu;

    connect_app(&app);

    surface = wl_compositor_create_surface(app.compositor);
    xdg_surface = xdg_wm_base_get_xdg_surface(app.wm_base, surface);
    xdg_surface_add_listener(xdg_surface, &xdg_surface_listener, &xdg_state);
    toplevel = xdg_surface_get_toplevel(xdg_surface);
    xdg_toplevel_add_listener(toplevel, &toplevel_listener, NULL);
    xdg_toplevel_set_title(toplevel, "KDE AppMenu Probe");
    xdg_toplevel_set_app_id(toplevel, "gnoblin-kde-appmenu-probe");

    wl_surface_commit(surface);
    for (int i = 0; i < 30 && !xdg_state.configured; i++)
    {
        if (roundtrip_ok(&app, "xdg configure") != 0)
            return 1;
    }
    if (!xdg_state.configured)
    {
        fprintf(stderr, "timed out waiting for xdg configure\n");
        return 1;
    }

    xdg_surface_ack_configure(xdg_surface, xdg_state.serial);
    buffer = create_buffer(&app, 240, 120, 0xff315d8a);
    if (!buffer)
        die("could not create shm buffer");

    wl_surface_attach(surface, buffer, 0, 0);
    wl_surface_damage_buffer(surface, 0, 0, INT32_MAX, INT32_MAX);
    wl_surface_commit(surface);

    if (roundtrip_ok(&app, "map toplevel") != 0)
        return 1;

    appmenu = org_kde_kwin_appmenu_manager_create(app.appmenu_manager, surface);
    org_kde_kwin_appmenu_set_address(appmenu, bus, path);
    if (roundtrip_ok(&app, "publish appmenu address") != 0)
        return 1;

    printf("READY bus=%s path=%s\n", bus, path);
    fflush(stdout);

    while (wl_display_dispatch(app.display) >= 0)
        ;

    return 0;
}
