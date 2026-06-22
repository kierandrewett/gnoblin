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

struct app
{
  struct wl_display *display;
  struct wl_registry *registry;
  struct wl_compositor *compositor;
  struct wl_shm *shm;
  struct wl_output *output;
  struct wl_seat *seat;
  struct wl_pointer *pointer;
  struct wl_keyboard *keyboard;
  struct zwlr_layer_shell_v1 *layer_shell;
  struct wl_surface *surface;
  struct zwlr_layer_surface_v1 *layer_surface;
  struct wl_buffer *buffer;
  bool mapped;
  uint32_t serial;
  uint32_t keys;
};

static uint32_t
min_u32 (uint32_t a,
         uint32_t b)
{
  return a < b ? a : b;
}

static void
die (const char *message)
{
  fprintf (stderr, "layer-keyboard-focus-client: %s\n", message);
  exit (2);
}

static int
create_anonymous_file (size_t size)
{
  const char *runtime_dir = getenv ("XDG_RUNTIME_DIR");
  char path[PATH_MAX];
  int fd;

  if (!runtime_dir || runtime_dir[0] == '\0')
    runtime_dir = "/tmp";

  snprintf (path, sizeof path, "%s/gnoblin-layer-key-focus-XXXXXX",
            runtime_dir);

  fd = mkstemp (path);
  if (fd < 0)
    return -1;

  unlink (path);

  if (ftruncate (fd, (off_t) size) < 0)
    {
      close (fd);
      return -1;
    }

  return fd;
}

static struct wl_buffer *
create_buffer (struct app *app,
               int         width,
               int         height,
               uint32_t    color)
{
  int stride = width * 4;
  size_t size = (size_t) stride * (size_t) height;
  int fd;
  uint32_t *data;
  struct wl_shm_pool *pool;
  struct wl_buffer *buffer;

  fd = create_anonymous_file (size);
  if (fd < 0)
    return NULL;

  data = mmap (NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (data == MAP_FAILED)
    {
      close (fd);
      return NULL;
    }

  for (size_t i = 0; i < size / 4; i++)
    data[i] = color;

  pool = wl_shm_create_pool (app->shm, fd, (int32_t) size);
  buffer = wl_shm_pool_create_buffer (pool, 0, width, height, stride,
                                      WL_SHM_FORMAT_ARGB8888);
  wl_shm_pool_destroy (pool);
  munmap (data, size);
  close (fd);

  return buffer;
}

static void
map_surface (struct app *app)
{
  zwlr_layer_surface_v1_ack_configure (app->layer_surface, app->serial);
  app->buffer = create_buffer (app, 180, 90, 0xff60a5fa);
  if (!app->buffer)
    die ("could not create shm buffer");

  wl_surface_attach (app->surface, app->buffer, 0, 0);
  wl_surface_damage_buffer (app->surface, 0, 0, INT32_MAX, INT32_MAX);
  wl_surface_commit (app->surface);
  app->mapped = true;
  fprintf (stdout, "MAPPED\n");
  fflush (stdout);
}

static void
handle_layer_configure (void                         *data,
                        struct zwlr_layer_surface_v1 *surface,
                        uint32_t                      serial,
                        uint32_t                      width,
                        uint32_t                      height)
{
  struct app *app = data;

  (void) surface;
  (void) width;
  (void) height;

  app->serial = serial;
  if (!app->mapped)
    map_surface (app);
  else
    {
      zwlr_layer_surface_v1_ack_configure (app->layer_surface, serial);
      wl_surface_commit (app->surface);
    }
}

static void
handle_layer_closed (void                         *data,
                     struct zwlr_layer_surface_v1 *surface)
{
  (void) surface;

  ((struct app *) data)->mapped = false;
}

static const struct zwlr_layer_surface_v1_listener layer_surface_listener = {
  .configure = handle_layer_configure,
  .closed = handle_layer_closed,
};

static void
pointer_enter (void              *data,
               struct wl_pointer *pointer,
               uint32_t           serial,
               struct wl_surface *surface,
               wl_fixed_t         sx,
               wl_fixed_t         sy)
{
  (void) data;
  (void) pointer;
  (void) serial;
  (void) surface;

  fprintf (stdout, "POINTER_ENTER %d %d\n",
           wl_fixed_to_int (sx), wl_fixed_to_int (sy));
  fflush (stdout);
}

static void
pointer_leave (void              *data,
               struct wl_pointer *pointer,
               uint32_t           serial,
               struct wl_surface *surface)
{
  (void) data;
  (void) pointer;
  (void) serial;
  (void) surface;

  fprintf (stdout, "POINTER_LEAVE\n");
  fflush (stdout);
}

static void
pointer_motion (void              *data,
                struct wl_pointer *pointer,
                uint32_t           time,
                wl_fixed_t         sx,
                wl_fixed_t         sy)
{
  (void) data;
  (void) pointer;
  (void) time;

  fprintf (stdout, "POINTER_MOTION %d %d\n",
           wl_fixed_to_int (sx), wl_fixed_to_int (sy));
  fflush (stdout);
}

static void
pointer_button (void              *data,
                struct wl_pointer *pointer,
                uint32_t           serial,
                uint32_t           time,
                uint32_t           button,
                uint32_t           state)
{
  (void) data;
  (void) pointer;
  (void) serial;
  (void) time;

  fprintf (stdout, "POINTER_BUTTON %u %u\n", button, state);
  fflush (stdout);
}

static void
pointer_axis (void              *data,
              struct wl_pointer *pointer,
              uint32_t           time,
              uint32_t           axis,
              wl_fixed_t         value)
{
  (void) data;
  (void) pointer;
  (void) time;
  (void) axis;
  (void) value;
}

static const struct wl_pointer_listener pointer_listener = {
  .enter = pointer_enter,
  .leave = pointer_leave,
  .motion = pointer_motion,
  .button = pointer_button,
  .axis = pointer_axis,
};

static void
keyboard_keymap (void               *data,
                 struct wl_keyboard *keyboard,
                 uint32_t            format,
                 int32_t             fd,
                 uint32_t            size)
{
  (void) data;
  (void) keyboard;
  (void) format;
  (void) size;

  close (fd);
}

static void
keyboard_enter (void               *data,
                struct wl_keyboard *keyboard,
                uint32_t            serial,
                struct wl_surface  *surface,
                struct wl_array    *keys)
{
  (void) data;
  (void) keyboard;
  (void) serial;
  (void) surface;
  (void) keys;

  fprintf (stdout, "KEYBOARD_ENTER\n");
  fflush (stdout);
}

static void
keyboard_leave (void               *data,
                struct wl_keyboard *keyboard,
                uint32_t            serial,
                struct wl_surface  *surface)
{
  (void) data;
  (void) keyboard;
  (void) serial;
  (void) surface;
}

static void
keyboard_key (void               *data,
              struct wl_keyboard *keyboard,
              uint32_t            serial,
              uint32_t            time,
              uint32_t            key,
              uint32_t            state)
{
  struct app *app = data;

  (void) keyboard;
  (void) serial;
  (void) time;
  (void) key;

  if (state != WL_KEYBOARD_KEY_STATE_PRESSED)
    return;

  app->keys++;
  fprintf (stdout, "KEY %u\n", app->keys);
  fflush (stdout);
}

static void
keyboard_modifiers (void               *data,
                    struct wl_keyboard *keyboard,
                    uint32_t            serial,
                    uint32_t            mods_depressed,
                    uint32_t            mods_latched,
                    uint32_t            mods_locked,
                    uint32_t            group)
{
  (void) data;
  (void) keyboard;
  (void) serial;
  (void) mods_depressed;
  (void) mods_latched;
  (void) mods_locked;
  (void) group;
}

static void
keyboard_repeat_info (void               *data,
                      struct wl_keyboard *keyboard,
                      int32_t             rate,
                      int32_t             delay)
{
  (void) data;
  (void) keyboard;
  (void) rate;
  (void) delay;
}

static const struct wl_keyboard_listener keyboard_listener = {
  .keymap = keyboard_keymap,
  .enter = keyboard_enter,
  .leave = keyboard_leave,
  .key = keyboard_key,
  .modifiers = keyboard_modifiers,
  .repeat_info = keyboard_repeat_info,
};

static void
seat_capabilities (void           *data,
                   struct wl_seat *seat,
                   uint32_t        capabilities)
{
  struct app *app = data;

  if ((capabilities & WL_SEAT_CAPABILITY_POINTER) && !app->pointer)
    {
      app->pointer = wl_seat_get_pointer (seat);
      wl_pointer_add_listener (app->pointer, &pointer_listener, app);
    }
  if ((capabilities & WL_SEAT_CAPABILITY_KEYBOARD) && !app->keyboard)
    {
      app->keyboard = wl_seat_get_keyboard (seat);
      wl_keyboard_add_listener (app->keyboard, &keyboard_listener, app);
    }
}

static void
seat_name (void           *data,
           struct wl_seat *seat,
           const char     *name)
{
  (void) data;
  (void) seat;
  (void) name;
}

static const struct wl_seat_listener seat_listener = {
  .capabilities = seat_capabilities,
  .name = seat_name,
};

static void
handle_global (void               *data,
               struct wl_registry *registry,
               uint32_t            name,
               const char         *interface,
               uint32_t            version)
{
  struct app *app = data;

  if (strcmp (interface, wl_compositor_interface.name) == 0)
    app->compositor =
      wl_registry_bind (registry, name, &wl_compositor_interface,
                        min_u32 (version, 4));
  else if (strcmp (interface, wl_shm_interface.name) == 0)
    app->shm =
      wl_registry_bind (registry, name, &wl_shm_interface, 1);
  else if (strcmp (interface, wl_output_interface.name) == 0 && !app->output)
    app->output =
      wl_registry_bind (registry, name, &wl_output_interface, 1);
  else if (strcmp (interface, wl_seat_interface.name) == 0 && !app->seat)
    {
      app->seat =
        wl_registry_bind (registry, name, &wl_seat_interface,
                          min_u32 (version, 4));
      wl_seat_add_listener (app->seat, &seat_listener, app);
    }
  else if (strcmp (interface, zwlr_layer_shell_v1_interface.name) == 0)
    app->layer_shell =
      wl_registry_bind (registry, name, &zwlr_layer_shell_v1_interface,
                        min_u32 (version, 5));
}

static void
handle_global_remove (void               *data,
                      struct wl_registry *registry,
                      uint32_t            name)
{
  (void) data;
  (void) registry;
  (void) name;
}

static const struct wl_registry_listener registry_listener = {
  .global = handle_global,
  .global_remove = handle_global_remove,
};

int
main (int argc,
      char **argv)
{
  struct app app = { 0 };
  const char *mode;
  uint32_t keyboard_interactivity =
    ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_ON_DEMAND;

  (void) argc;
  (void) argv;

  mode = getenv ("GNOBLIN_LAYER_KEYBOARD_MODE");
  if (mode && strcmp (mode, "exclusive") == 0)
    keyboard_interactivity =
      ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE;
  else if (mode && strcmp (mode, "none") == 0)
    keyboard_interactivity =
      ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_NONE;

  app.display = wl_display_connect (NULL);
  if (!app.display)
    die ("could not connect to WAYLAND_DISPLAY");

  app.registry = wl_display_get_registry (app.display);
  wl_registry_add_listener (app.registry, &registry_listener, &app);
  if (wl_display_roundtrip (app.display) < 0)
    die ("registry roundtrip failed");
  if (wl_display_roundtrip (app.display) < 0)
    die ("seat roundtrip failed");

  if (!app.compositor || !app.shm || !app.layer_shell)
    die ("missing required Wayland globals");

  app.surface = wl_compositor_create_surface (app.compositor);
  app.layer_surface =
    zwlr_layer_shell_v1_get_layer_surface (app.layer_shell,
                                           app.surface,
                                           app.output,
                                           ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,
                                           "gnoblin-layer-keyboard-focus");
  zwlr_layer_surface_v1_add_listener (app.layer_surface,
                                      &layer_surface_listener,
                                      &app);
  zwlr_layer_surface_v1_set_size (app.layer_surface, 180, 90);
  zwlr_layer_surface_v1_set_anchor (app.layer_surface,
                                    ZWLR_LAYER_SURFACE_V1_ANCHOR_TOP |
                                    ZWLR_LAYER_SURFACE_V1_ANCHOR_LEFT);
  zwlr_layer_surface_v1_set_keyboard_interactivity (
    app.layer_surface,
    keyboard_interactivity);
  wl_surface_commit (app.surface);

  while (wl_display_dispatch (app.display) >= 0)
    {
    }

  return 0;
}
