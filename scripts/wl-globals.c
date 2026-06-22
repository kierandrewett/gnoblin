/*
 * gnoblin: tiny Wayland registry dumper.
 *
 * Connects to $WAYLAND_DISPLAY and prints every advertised global as
 * "interface version". Used by the devkit to verify which protocols gnoblin's
 * compositor exposes, without depending on wayland-info being installed.
 *
 * Build: cc scripts/wl-globals.c $(pkg-config --cflags --libs wayland-client) -o wl-globals
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-client.h>

static void
handle_global (void *data, struct wl_registry *registry, uint32_t name,
               const char *interface, uint32_t version)
{
  const char *filter = data;

  if (filter && filter[0] && !strstr (interface, filter))
    return;

  printf ("%s %u\n", interface, version);
}

static void
handle_global_remove (void *data, struct wl_registry *registry, uint32_t name)
{
}

static const struct wl_registry_listener registry_listener = {
  handle_global,
  handle_global_remove,
};

int
main (int argc, char **argv)
{
  struct wl_display *display;
  struct wl_registry *registry;
  const char *filter = argc > 1 ? argv[1] : "";

  display = wl_display_connect (NULL);
  if (!display)
    {
      fprintf (stderr, "wl-globals: cannot connect to WAYLAND_DISPLAY=%s\n",
               getenv ("WAYLAND_DISPLAY"));
      return 1;
    }

  registry = wl_display_get_registry (display);
  wl_registry_add_listener (registry, &registry_listener, (void *) filter);
  wl_display_roundtrip (display);

  wl_registry_destroy (registry);
  wl_display_disconnect (display);
  return 0;
}
