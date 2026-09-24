/* Small GTK4 Wayland toplevel used by the seeded lifecycle fuzzer. */
#include <gtk/gtk.h>
#include <glib-unix.h>
#include <signal.h>
#include <unistd.h>

static GMainLoop *main_loop;
static GtkApplication *application;
static GtkWindow *window;

static gboolean close_from_signal(gpointer data) {
    if (window)
        gtk_window_close(window);
    return G_SOURCE_REMOVE;
}

static gboolean close_requested(GtkWindow *closed_window, gpointer data) {
    g_main_loop_quit(main_loop);
    return FALSE;
}

static void activate(GtkApplication *app, gpointer data) {
    window = GTK_WINDOW(gtk_application_window_new(app));
    gtk_window_set_title(window, data);
    gtk_window_set_default_size(window, 480, 320);
    gtk_window_set_child(window, gtk_label_new(data));
    g_signal_connect(window, "close-request", G_CALLBACK(close_requested), NULL);
    gtk_window_present(window);
}

int main(int argc, char **argv) {
    if (argc != 2) {
        g_printerr("usage: %s TITLE\n", argv[0]);
        return 2;
    }

    g_autofree char *app_id = g_strdup_printf("org.gnoblin.LifecycleFuzz.p%d", getpid());
    application = gtk_application_new(app_id, G_APPLICATION_NON_UNIQUE);
    g_signal_connect(application, "activate", G_CALLBACK(activate), argv[1]);
    g_autoptr(GError) error = NULL;
    if (!g_application_register(G_APPLICATION(application), NULL, &error)) {
        g_printerr("cannot register fixture application: %s\n", error->message);
        return 1;
    }

    main_loop = g_main_loop_new(NULL, FALSE);
    g_unix_signal_add(SIGUSR1, close_from_signal, NULL);
    g_application_activate(G_APPLICATION(application));
    g_main_loop_run(main_loop);
    g_clear_object(&application);
    g_main_loop_unref(main_loop);
    return 0;
}
