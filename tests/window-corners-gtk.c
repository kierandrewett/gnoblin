#include <adwaita.h>

static GtkCssProvider* css_provider;
static char* colour_file;
static char* last_colour;

static void load_css(const char* background, const char* foreground) {
    g_autofree char* css = g_strdup_printf(
        "window.background, headerbar { background: %s; color: %s; }", background, foreground);

    gtk_css_provider_load_from_string(css_provider, css);
}

static gboolean reload_colour(gpointer user_data) {
    g_autofree char* value = NULL;
    GdkRGBA colour;

    (void)user_data;

    if (!g_file_get_contents(colour_file, &value, NULL, NULL))
        return G_SOURCE_CONTINUE;

    g_strstrip(value);
    if (g_str_equal(value, last_colour))
        return G_SOURCE_CONTINUE;

    if (!gdk_rgba_parse(&colour, value))
        return G_SOURCE_CONTINUE;

    g_autofree char* background = gdk_rgba_to_string(&colour);
    load_css(background, "white");
    g_free(last_colour);
    last_colour = g_strdup(value);

    return G_SOURCE_CONTINUE;
}

static void activate(GtkApplication* application, gpointer user_data) {
    GdkDisplay* display = gdk_display_get_default();
    GtkWidget* window = adw_application_window_new(application);
    GtkWidget* view = adw_toolbar_view_new();
    GtkWidget* header = adw_header_bar_new();

    (void)user_data;

    css_provider = gtk_css_provider_new();
    load_css("white", "black");
    gtk_style_context_add_provider_for_display(display, GTK_STYLE_PROVIDER(css_provider),
                                               GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);

    colour_file = g_strdup(g_getenv("GNOBLIN_CSD_COLOUR_FILE"));
    if (colour_file && *colour_file)
        g_timeout_add(16, reload_colour, NULL);

    gtk_window_set_title(GTK_WINDOW(window), "Corner fixture");
    gtk_window_set_default_size(GTK_WINDOW(window), 320, 240);
    adw_toolbar_view_add_top_bar(ADW_TOOLBAR_VIEW(view), header);
    adw_toolbar_view_set_content(ADW_TOOLBAR_VIEW(view), gtk_box_new(GTK_ORIENTATION_VERTICAL, 0));
    adw_application_window_set_content(ADW_APPLICATION_WINDOW(window), view);
    gtk_window_present(GTK_WINDOW(window));
}

int main(int argc, char** argv) {
    g_autoptr(GtkApplication) application =
        gtk_application_new("org.gnoblin.CornerFixture", G_APPLICATION_DEFAULT_FLAGS);

    g_signal_connect(application, "activate", G_CALLBACK(activate), NULL);
    return g_application_run(G_APPLICATION(application), argc, argv);
}
