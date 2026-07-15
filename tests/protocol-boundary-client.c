/* Black-box client harness for Gnoblin-owned Wayland protocol boundaries. */
#include "wlr-foreign-toplevel-management-unstable-v1-client-protocol.h"
#include "wlr-layer-shell-unstable-v1-client-protocol.h"
#include "wlr-screencopy-unstable-v1-client-protocol.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <wayland-client.h>

struct protocols
{
  struct zwlr_foreign_toplevel_manager_v1 *foreign_toplevel;
  struct zwlr_layer_shell_v1 *layer_shell;
  struct zwlr_screencopy_manager_v1 *screencopy;
};

static uint32_t
supported_version (uint32_t advertised, uint32_t supported)
{
  return advertised < supported ? advertised : supported;
}

static void
registry_global (void *data,
                 struct wl_registry *registry,
                 uint32_t name,
                 const char *interface,
                 uint32_t version)
{
  struct protocols *protocols = data;

  if (strcmp (interface, zwlr_foreign_toplevel_manager_v1_interface.name) == 0)
    protocols->foreign_toplevel =
      wl_registry_bind (registry, name,
                        &zwlr_foreign_toplevel_manager_v1_interface,
                        supported_version (version,
                                           zwlr_foreign_toplevel_manager_v1_interface.version));
  else if (strcmp (interface, zwlr_layer_shell_v1_interface.name) == 0)
    protocols->layer_shell =
      wl_registry_bind (registry, name,
                        &zwlr_layer_shell_v1_interface,
                        supported_version (version, zwlr_layer_shell_v1_interface.version));
  else if (strcmp (interface, zwlr_screencopy_manager_v1_interface.name) == 0)
    protocols->screencopy =
      wl_registry_bind (registry, name,
                        &zwlr_screencopy_manager_v1_interface,
                        supported_version (version,
                                           zwlr_screencopy_manager_v1_interface.version));
}

static void
registry_global_remove (void *data, struct wl_registry *registry, uint32_t name)
{
  (void) data;
  (void) registry;
  (void) name;
}

static const struct wl_registry_listener registry_listener = {
  .global = registry_global,
  .global_remove = registry_global_remove,
};

int
main (void)
{
  struct protocols protocols = { 0 };
  struct wl_display *display = wl_display_connect (NULL);
  struct wl_registry *registry;

  if (!display)
    {
      fprintf (stderr, "FAIL: could not connect to Wayland display\n");
      return 1;
    }

  registry = wl_display_get_registry (display);
  wl_registry_add_listener (registry, &registry_listener, &protocols);
  if (wl_display_roundtrip (display) < 0)
    {
      fprintf (stderr, "FAIL: registry round-trip failed\n");
      return 1;
    }

  if (!protocols.foreign_toplevel || !protocols.layer_shell || !protocols.screencopy)
    {
      fprintf (stderr,
               "FAIL: missing owned protocol: foreign-toplevel=%d layer-shell=%d screencopy=%d\n",
               protocols.foreign_toplevel != NULL,
               protocols.layer_shell != NULL,
               protocols.screencopy != NULL);
      return 1;
    }

  /* The first sync discovers globals; the second proves the server created
   * every resource queued from those registry callbacks. */
  if (wl_display_roundtrip (display) < 0)
    {
      fprintf (stderr, "FAIL: protocol bind round-trip failed\n");
      return 1;
    }

  wl_display_disconnect (display);
  printf ("PASS: owned protocol client connected and disconnected cleanly\n");
  return 0;
}
