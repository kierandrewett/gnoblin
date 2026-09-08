/* Native Wayland activation probe: request a token without an input serial. */
#include <gtk/gtk.h>
#include <glib/gstdio.h>
#include <gdk/gdkwayland.h>
#include <wayland-client.h>
#include <string.h>
#include "xdg-activation-v1-client-protocol.h"

static struct xdg_activation_v1 *activation;
static GtkWidget *window;
static const char *request_path;
static const char *input_path;

static void
registry_global (void *data, struct wl_registry *registry, uint32_t name,
                 const char *interface, uint32_t version)
{
  if (strcmp (interface, "xdg_activation_v1") == 0)
    activation = wl_registry_bind (registry, name, &xdg_activation_v1_interface, 1);
}

static void registry_remove (void *data, struct wl_registry *registry, uint32_t name) {}
static const struct wl_registry_listener registry_listener = { registry_global, registry_remove };

static void
token_done (void *data, struct xdg_activation_token_v1 *token, const char *value)
{
  struct wl_surface *surface = gdk_wayland_window_get_wl_surface (gtk_widget_get_window (window));
  xdg_activation_v1_activate (activation, value, surface);
  xdg_activation_token_v1_destroy (token);
}
static const struct xdg_activation_token_v1_listener token_listener = { token_done };

static gboolean
poll_request (gpointer data)
{
  if (g_file_test (request_path, G_FILE_TEST_EXISTS))
    {
      g_unlink (request_path);
      struct xdg_activation_token_v1 *token = xdg_activation_v1_get_activation_token (activation);
      xdg_activation_token_v1_add_listener (token, &token_listener, NULL);
      xdg_activation_token_v1_commit (token);
      wl_display_flush (gdk_wayland_display_get_wl_display (gdk_display_get_default ()));
    }
  return G_SOURCE_CONTINUE;
}

static gboolean
key_press (GtkWidget *widget, GdkEventKey *event, gpointer data)
{
  if (event->keyval == GDK_KEY_f)
    g_file_set_contents (input_path, "focused", -1, NULL);
  return FALSE;
}

int
main (int argc, char **argv)
{
  gtk_init (&argc, &argv);
  g_assert (argc == 3);
  request_path = argv[1];
  input_path = argv[2];
  struct wl_display *display = gdk_wayland_display_get_wl_display (gdk_display_get_default ());
  struct wl_registry *registry = wl_display_get_registry (display);
  wl_registry_add_listener (registry, &registry_listener, NULL);
  wl_display_roundtrip (display);
  g_assert (activation != NULL);
  window = gtk_window_new (GTK_WINDOW_TOPLEVEL);
  gtk_window_set_title (GTK_WINDOW (window), "Wayland Activation Target");
  gtk_window_set_default_size (GTK_WINDOW (window), 400, 300);
  gtk_container_add (GTK_CONTAINER (window), gtk_label_new ("Activation must deliver keyboard input here"));
  g_signal_connect (window, "key-press-event", G_CALLBACK (key_press), NULL);
  g_signal_connect (window, "destroy", G_CALLBACK (gtk_main_quit), NULL);
  gtk_widget_show_all (window);
  g_timeout_add (30, poll_request, NULL);
  gtk_main ();
  return 0;
}
