#define _POSIX_C_SOURCE 200809L

/* Small Wayland fixture and shell-activation driver for input-source E2E. */
#include "wlr-foreign-toplevel-management-unstable-v1-client-protocol.h"
#include "xdg-shell-client-protocol.h"

#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>

enum { WINDOW_WIDTH = 320, WINDOW_HEIGHT = 240, ACTIVATION_TIMEOUT_MS = 5000 };

struct app;

struct window {
    struct app* app;
    struct wl_surface* surface;
    struct wl_buffer* buffer;
    struct xdg_surface* xdg_surface;
    struct xdg_toplevel* toplevel;
    bool mapped;
    bool closed;
};

struct foreign_handle {
    struct app* app;
    struct zwlr_foreign_toplevel_handle_v1* handle;
    struct foreign_handle* next;
    char* title;
    bool activated;
    bool activation_requested;
};

struct app {
    struct wl_display* display;
    struct wl_registry* registry;
    struct wl_compositor* compositor;
    struct wl_shm* shm;
    struct wl_seat* seat;
    struct xdg_wm_base* wm_base;
    struct zwlr_foreign_toplevel_manager_v1* manager;
    struct window window;
    struct foreign_handle* handles;
    const char* requested_title;
    bool activation_confirmed;
};

static volatile sig_atomic_t keep_running = 1;

static void stop_running(int signal_number) {
    (void)signal_number;
    keep_running = 0;
}

static int64_t monotonic_milliseconds(void) {
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0)
        return -1;
    return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static void wm_base_ping(void* data, struct xdg_wm_base* wm_base, uint32_t serial) {
    (void)data;
    xdg_wm_base_pong(wm_base, serial);
}

static const struct xdg_wm_base_listener wm_base_listener = {.ping = wm_base_ping};

static void create_window_buffer(struct window* window) {
    struct app* app = window->app;
    const int stride = WINDOW_WIDTH * 4;
    const int size = stride * WINDOW_HEIGHT;
    const char* runtime_dir = getenv("XDG_RUNTIME_DIR");
    char path[4096];
    int fd;
    uint32_t* pixels;
    struct wl_shm_pool* pool;

    if (window->buffer || !runtime_dir ||
        snprintf(path, sizeof(path), "%s/gnoblin-input-source-%ld", runtime_dir, (long)getpid()) >=
            (int)sizeof(path))
        return;

    fd = open(path, O_CREAT | O_EXCL | O_RDWR | O_CLOEXEC, 0600);
    if (fd < 0 || unlink(path) != 0 || ftruncate(fd, size) != 0) {
        if (fd >= 0)
            close(fd);
        return;
    }
    pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (pixels == MAP_FAILED) {
        close(fd);
        return;
    }
    for (int i = 0; i < WINDOW_WIDTH * WINDOW_HEIGHT; i++)
        pixels[i] = 0xff4b6178;
    munmap(pixels, size);

    pool = wl_shm_create_pool(app->shm, fd, size);
    window->buffer = wl_shm_pool_create_buffer(pool, 0, WINDOW_WIDTH, WINDOW_HEIGHT, stride,
                                               WL_SHM_FORMAT_XRGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    if (!window->buffer)
        return;

    wl_surface_attach(window->surface, window->buffer, 0, 0);
    wl_surface_damage(window->surface, 0, 0, WINDOW_WIDTH, WINDOW_HEIGHT);
    wl_surface_commit(window->surface);
    window->mapped = true;
}

static void xdg_surface_configure(void* data, struct xdg_surface* surface, uint32_t serial) {
    struct window* window = data;
    xdg_surface_ack_configure(surface, serial);
    create_window_buffer(window);
}

static const struct xdg_surface_listener xdg_surface_listener = {
    .configure = xdg_surface_configure,
};

static void xdg_toplevel_configure(void* data, struct xdg_toplevel* toplevel, int32_t width,
                                   int32_t height, struct wl_array* states) {
    (void)data;
    (void)toplevel;
    (void)width;
    (void)height;
    (void)states;
}

static void xdg_toplevel_close(void* data, struct xdg_toplevel* toplevel) {
    struct window* window = data;
    (void)toplevel;
    window->closed = true;
}

static void xdg_toplevel_configure_bounds(void* data, struct xdg_toplevel* toplevel, int32_t width,
                                          int32_t height) {
    (void)data;
    (void)toplevel;
    (void)width;
    (void)height;
}

static void xdg_toplevel_wm_capabilities(void* data, struct xdg_toplevel* toplevel,
                                         struct wl_array* capabilities) {
    (void)data;
    (void)toplevel;
    (void)capabilities;
}

static const struct xdg_toplevel_listener xdg_toplevel_listener = {
    .configure = xdg_toplevel_configure,
    .close = xdg_toplevel_close,
    .configure_bounds = xdg_toplevel_configure_bounds,
    .wm_capabilities = xdg_toplevel_wm_capabilities,
};

static void request_activation(struct foreign_handle* target) {
    struct app* app = target->app;
    if (target->activation_requested || !app->seat)
        return;
    target->activation_requested = true;
    zwlr_foreign_toplevel_handle_v1_activate(target->handle, app->seat);
    wl_display_flush(app->display);
}

static void foreign_handle_title(void* data, struct zwlr_foreign_toplevel_handle_v1* handle,
                                 const char* title) {
    struct foreign_handle* state = data;
    (void)handle;
    free(state->title);
    state->title = strdup(title);
    if (state->title && state->app->requested_title &&
        strcmp(state->title, state->app->requested_title) == 0)
        request_activation(state);
}

static void foreign_handle_app_id(void* data, struct zwlr_foreign_toplevel_handle_v1* handle,
                                  const char* app_id) {
    (void)data;
    (void)handle;
    (void)app_id;
}

static void foreign_handle_output(void* data, struct zwlr_foreign_toplevel_handle_v1* handle,
                                  struct wl_output* output) {
    (void)data;
    (void)handle;
    (void)output;
}

static void foreign_handle_state(void* data, struct zwlr_foreign_toplevel_handle_v1* handle,
                                 struct wl_array* states) {
    struct foreign_handle* state = data;
    uint32_t* value;
    (void)handle;

    state->activated = false;
    wl_array_for_each(value, states) {
        if (*value == ZWLR_FOREIGN_TOPLEVEL_HANDLE_V1_STATE_ACTIVATED)
            state->activated = true;
    }
}

static void foreign_handle_done(void* data, struct zwlr_foreign_toplevel_handle_v1* handle) {
    struct foreign_handle* state = data;
    (void)handle;
    if (state->activation_requested && state->activated && state->title &&
        strcmp(state->title, state->app->requested_title) == 0)
        state->app->activation_confirmed = true;
}

static void foreign_handle_closed(void* data, struct zwlr_foreign_toplevel_handle_v1* handle) {
    (void)data;
    (void)handle;
}

static void foreign_handle_parent(void* data, struct zwlr_foreign_toplevel_handle_v1* handle,
                                  struct zwlr_foreign_toplevel_handle_v1* parent) {
    (void)data;
    (void)handle;
    (void)parent;
}

static const struct zwlr_foreign_toplevel_handle_v1_listener foreign_handle_listener = {
    .title = foreign_handle_title,
    .app_id = foreign_handle_app_id,
    .output_enter = foreign_handle_output,
    .output_leave = foreign_handle_output,
    .state = foreign_handle_state,
    .done = foreign_handle_done,
    .closed = foreign_handle_closed,
    .parent = foreign_handle_parent,
};

static void foreign_toplevel(void* data, struct zwlr_foreign_toplevel_manager_v1* manager,
                             struct zwlr_foreign_toplevel_handle_v1* handle) {
    struct app* app = data;
    struct foreign_handle* state = calloc(1, sizeof(*state));
    (void)manager;
    if (!state) {
        wl_proxy_destroy((struct wl_proxy*)handle);
        return;
    }
    state->app = app;
    state->handle = handle;
    state->next = app->handles;
    app->handles = state;
    zwlr_foreign_toplevel_handle_v1_add_listener(handle, &foreign_handle_listener, state);
}

static void foreign_manager_finished(void* data, struct zwlr_foreign_toplevel_manager_v1* manager) {
    (void)data;
    (void)manager;
}

static const struct zwlr_foreign_toplevel_manager_v1_listener foreign_manager_listener = {
    .toplevel = foreign_toplevel,
    .finished = foreign_manager_finished,
};

static void registry_global(void* data, struct wl_registry* registry, uint32_t name,
                            const char* interface, uint32_t version) {
    struct app* app = data;
    if (strcmp(interface, wl_compositor_interface.name) == 0) {
        app->compositor =
            wl_registry_bind(registry, name, &wl_compositor_interface, version < 4 ? version : 4);
    } else if (strcmp(interface, wl_shm_interface.name) == 0) {
        app->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    } else if (strcmp(interface, xdg_wm_base_interface.name) == 0) {
        app->wm_base =
            wl_registry_bind(registry, name, &xdg_wm_base_interface, version < 6 ? version : 6);
        xdg_wm_base_add_listener(app->wm_base, &wm_base_listener, app);
    } else if (strcmp(interface, wl_seat_interface.name) == 0 && !app->seat) {
        app->seat = wl_registry_bind(registry, name, &wl_seat_interface, 1);
    } else if (strcmp(interface, zwlr_foreign_toplevel_manager_v1_interface.name) == 0 &&
               !app->manager) {
        app->manager = wl_registry_bind(registry, name, &zwlr_foreign_toplevel_manager_v1_interface,
                                        version < 3 ? version : 3);
        zwlr_foreign_toplevel_manager_v1_add_listener(app->manager, &foreign_manager_listener, app);
    }
}

static void registry_remove(void* data, struct wl_registry* registry, uint32_t name) {
    (void)data;
    (void)registry;
    (void)name;
}

static const struct wl_registry_listener registry_listener = {
    .global = registry_global,
    .global_remove = registry_remove,
};

static bool connect_display(struct app* app) {
    app->display = wl_display_connect(NULL);
    if (!app->display)
        return false;
    app->registry = wl_display_get_registry(app->display);
    wl_registry_add_listener(app->registry, &registry_listener, app);
    if (wl_display_roundtrip(app->display) < 0)
        return false;
    return true;
}

static void report_display_error(struct app* app, const char* operation, const char* title) {
    int saved_errno = errno;
    int error = wl_display_get_error(app->display);
    if (error == EPROTO) {
        const struct wl_interface* interface = NULL;
        uint32_t object_id = 0;
        uint32_t code = wl_display_get_protocol_error(app->display, &interface, &object_id);
        fprintf(stderr, "Wayland %s failed for window '%s': protocol error %u on %s object %u\n",
                operation, title, code, interface ? interface->name : "unknown interface",
                object_id);
        return;
    }

    if (error == 0)
        error = saved_errno;
    fprintf(stderr, "Wayland %s failed for window '%s': error %d (%s)\n", operation, title, error,
            error ? strerror(error) : "no error detail available");
}

static void report_activation_timeout(const char* title, int64_t started) {
    int64_t now = monotonic_milliseconds();
    int64_t elapsed = now >= started ? now - started : ACTIVATION_TIMEOUT_MS;
    fprintf(stderr, "window activation was not confirmed for '%s' after %lld ms (timeout %d ms)\n",
            title, (long long)elapsed, ACTIVATION_TIMEOUT_MS);
}

static void create_test_window(struct app* app, const char* title) {
    app->window.app = app;
    app->window.surface = wl_compositor_create_surface(app->compositor);
    app->window.xdg_surface = xdg_wm_base_get_xdg_surface(app->wm_base, app->window.surface);
    xdg_surface_add_listener(app->window.xdg_surface, &xdg_surface_listener, &app->window);
    app->window.toplevel = xdg_surface_get_toplevel(app->window.xdg_surface);
    xdg_toplevel_add_listener(app->window.toplevel, &xdg_toplevel_listener, &app->window);
    xdg_toplevel_set_title(app->window.toplevel, title);
    xdg_toplevel_set_app_id(app->window.toplevel, "org.gnoblin.InputSourceTest");
    wl_surface_commit(app->window.surface);
}

static int run_window(struct app* app, const char* title) {
    if (!app->compositor || !app->shm || !app->wm_base) {
        fprintf(stderr, "required Wayland globals are unavailable\n");
        return 1;
    }
    create_test_window(app, title);
    if (wl_display_roundtrip(app->display) < 0 || !app->window.mapped) {
        fprintf(stderr, "test toplevel did not map\n");
        return 1;
    }

    while (keep_running && !app->window.closed) {
        struct pollfd fd = {.fd = wl_display_get_fd(app->display), .events = POLLIN};
        if (wl_display_dispatch_pending(app->display) < 0)
            return 1;
        if (poll(&fd, 1, 100) < 0 && errno != EINTR)
            return 1;
        if (fd.revents & POLLIN && wl_display_dispatch(app->display) < 0)
            return 1;
    }
    return 0;
}

static int run_activation(struct app* app, const char* title) {
    if (!app->manager || !app->seat) {
        fprintf(stderr, "foreign-toplevel manager or seat is unavailable\n");
        return 1;
    }
    app->requested_title = title;
    int64_t started = monotonic_milliseconds();
    int64_t deadline = started + ACTIVATION_TIMEOUT_MS;
    while (!app->activation_confirmed) {
        struct pollfd fd = {.fd = wl_display_get_fd(app->display), .events = POLLIN};
        int64_t remaining = deadline - monotonic_milliseconds();
        if (remaining <= 0) {
            report_activation_timeout(title, started);
            return 1;
        }
        if (wl_display_dispatch_pending(app->display) < 0) {
            report_display_error(app, "dispatch pending events", title);
            return 1;
        }
        if (app->activation_confirmed)
            break;
        if (wl_display_flush(app->display) < 0 && errno != EAGAIN) {
            report_display_error(app, "flush requests", title);
            return 1;
        }
        int result = poll(&fd, 1, (int)remaining);
        if (result < 0 && errno == EINTR)
            continue;
        if (result == 0) {
            report_activation_timeout(title, started);
            return 1;
        }
        if (result < 0) {
            int error = errno;
            fprintf(stderr, "poll for window activation '%s' failed: error %d (%s)\n", title, error,
                    strerror(error));
            return 1;
        }
        if (wl_display_dispatch(app->display) < 0) {
            report_display_error(app, "dispatch events", title);
            return 1;
        }
    }
    printf("ACTIVATED:%s\n", title);
    return 0;
}

static void cleanup(struct app* app) {
    struct foreign_handle* handle = app->handles;
    while (handle) {
        struct foreign_handle* next = handle->next;
        wl_proxy_destroy((struct wl_proxy*)handle->handle);
        free(handle->title);
        free(handle);
        handle = next;
    }
    if (app->window.buffer)
        wl_buffer_destroy(app->window.buffer);
    if (app->window.toplevel)
        xdg_toplevel_destroy(app->window.toplevel);
    if (app->window.xdg_surface)
        xdg_surface_destroy(app->window.xdg_surface);
    if (app->window.surface)
        wl_surface_destroy(app->window.surface);
    if (app->manager)
        wl_proxy_destroy((struct wl_proxy*)app->manager);
    if (app->seat)
        wl_seat_destroy(app->seat);
    if (app->wm_base)
        xdg_wm_base_destroy(app->wm_base);
    if (app->shm)
        wl_shm_destroy(app->shm);
    if (app->compositor)
        wl_compositor_destroy(app->compositor);
    if (app->registry)
        wl_registry_destroy(app->registry);
    if (app->display)
        wl_display_disconnect(app->display);
}

int main(int argc, char** argv) {
    struct app app = {0};
    struct sigaction action = {.sa_handler = stop_running};
    int result;

    if (argc != 3 || (strcmp(argv[1], "window") != 0 && strcmp(argv[1], "activate") != 0)) {
        fprintf(stderr, "usage: %s window TITLE | activate TITLE\n", argv[0]);
        return 2;
    }
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    sigaction(SIGUSR1, &action, NULL);
    if (strcmp(argv[1], "activate") == 0)
        app.requested_title = argv[2];

    if (!connect_display(&app)) {
        fprintf(stderr, "cannot connect to the nested Wayland display\n");
        cleanup(&app);
        return 1;
    }
    result =
        strcmp(argv[1], "window") == 0 ? run_window(&app, argv[2]) : run_activation(&app, argv[2]);
    cleanup(&app);
    return result;
}
