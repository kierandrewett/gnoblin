/*
 * Copyright © 2019 Red Hat, Inc
 *
 * This program is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library. If not, see <http://www.gnu.org/licenses/>.
 *
 * Authors:
 *       Felipe Borges <feborges@redhat.com>
 */

#define _GNU_SOURCE 1

#include "config.h"

#include <string.h>

#include <adwaita.h>
#include <gdk-pixbuf/gdk-pixbuf.h>
#ifdef HAVE_GLYCIN
#include <glycin.h>
#endif
#include <gio/gio.h>
#include <glib/gi18n.h>
#include <gtk/gtk.h>

#include "wallpaperpreview.h"

struct _WallpaperPreview {
    GtkBox parent;

    GtkStack* stack;
    GtkWidget* loading;
    GtkWidget* desktop_preview;
    GtkWidget* animated_background_icon;
    GtkLabel* desktop_clock_label;
    GtkWidget* drawing_area;

    GdkPixbuf* image;

    GSettings* desktop_settings;
    gboolean is_24h_format;
    GDateTime* previous_time;
    guint clock_time_timeout_id;
};

struct _WallpaperPreviewClass {
    GtkBoxClass parent_class;
};

G_DEFINE_TYPE(WallpaperPreview, wallpaper_preview, GTK_TYPE_BOX)

static void draw_preview_func(GtkDrawingArea* drawing_area, cairo_t* cr, int width, int height,
                              gpointer data) {
    WallpaperPreview* self = WALLPAPER_PREVIEW(data);
    double scale;
    double image_width;
    double image_height;

    if (!self->image || width <= 0 || height <= 0)
        return;

    image_width = gdk_pixbuf_get_width(self->image);
    image_height = gdk_pixbuf_get_height(self->image);
    scale = MAX(width / image_width, height / image_height);
    cairo_save(cr);
    cairo_translate(cr, (width - image_width * scale) / 2, (height - image_height * scale) / 2);
    cairo_scale(cr, scale, scale);
    gdk_cairo_set_source_pixbuf(cr, self->image, 0, 0);
    cairo_paint(cr);
    cairo_restore(cr);
}

typedef struct {
    GString* path_text;
    gchar* first_path;
    const gchar* capture_element;
    guint static_count;
    gboolean background;
    gboolean animated;
} SlideshowPreview;

static void slideshow_start_element(GMarkupParseContext* context, const gchar* element_name,
                                    const gchar** attribute_names, const gchar** attribute_values,
                                    gpointer user_data, GError** error) {
    SlideshowPreview* preview = user_data;

    if (g_str_equal(element_name, "background"))
        preview->background = TRUE;
    else if (g_str_equal(element_name, "transition"))
        preview->animated = TRUE;
    else if (g_str_equal(element_name, "static"))
        preview->static_count++;

    if (!preview->first_path && !preview->capture_element &&
        (g_str_equal(element_name, "file") || g_str_equal(element_name, "from") ||
         g_str_equal(element_name, "to"))) {
        preview->capture_element = g_intern_string(element_name);
        g_string_truncate(preview->path_text, 0);
    }
}

static void slideshow_text(GMarkupParseContext* context, const gchar* text, gsize length,
                           gpointer user_data, GError** error) {
    SlideshowPreview* preview = user_data;

    if (preview->capture_element)
        g_string_append_len(preview->path_text, text, length);
}

static void slideshow_end_element(GMarkupParseContext* context, const gchar* element_name,
                                  gpointer user_data, GError** error) {
    SlideshowPreview* preview = user_data;

    if (g_strcmp0(element_name, preview->capture_element) == 0) {
        preview->first_path = g_strdup(g_strstrip(preview->path_text->str));
        preview->capture_element = NULL;
    }
}

static gchar* first_slideshow_image(const gchar* path, gboolean* animated) {
    static const GMarkupParser parser = {
        slideshow_start_element, slideshow_end_element, slideshow_text, NULL, NULL,
    };
    SlideshowPreview preview = {.path_text = g_string_new(NULL)};
    g_autoptr(GMarkupParseContext) context = NULL;
    g_autoptr(GFile) file = NULL;
    g_autoptr(GFileInputStream) stream = NULL;
    g_autofree gchar* contents = NULL;
    gchar header[4096] = {0};
    gssize header_length;
    gsize length = 0;
    gchar* image = NULL;

    file = g_file_new_for_path(path);
    stream = g_file_read(file, NULL, NULL);
    if (!stream)
        goto out;
    header_length =
        g_input_stream_read(G_INPUT_STREAM(stream), header, sizeof header - 1, NULL, NULL);
    if (header_length <= 0 || !g_strstr_len(header, header_length, "<background"))
        goto out;

    if (!g_file_get_contents(path, &contents, &length, NULL))
        goto out;

    context = g_markup_parse_context_new(&parser, 0, &preview, NULL);
    if (g_markup_parse_context_parse(context, contents, length, NULL) &&
        g_markup_parse_context_end_parse(context, NULL) && preview.background &&
        preview.first_path && *preview.first_path) {
        image = g_steal_pointer(&preview.first_path);
        *animated = preview.animated || preview.static_count > 1;
    }

out:
    g_free(preview.first_path);
    g_string_free(preview.path_text, TRUE);
    return image;
}

static GdkPixbuf* fit_preview_size(GdkPixbuf* decoded) {
    int width = gdk_pixbuf_get_width(decoded);
    int height = gdk_pixbuf_get_height(decoded);

    if (width <= 1024 && height <= 1024)
        return g_object_ref(decoded);

    double scale = MIN(1024.0 / width, 1024.0 / height);
    return gdk_pixbuf_scale_simple(decoded, MAX(1, (int)(width * scale)),
                                   MAX(1, (int)(height * scale)), GDK_INTERP_BILINEAR);
}

#ifdef HAVE_GLYCIN
static GdkPixbuf* load_preview_image(const gchar* path, gboolean* animated, GError** error) {
    g_autoptr(GFile) file = g_file_new_for_path(path);
    g_autoptr(GlyLoader) loader = gly_loader_new(file);
    g_autoptr(GlyImage) image = NULL;
    g_autoptr(GlyFrame) frame = NULL;
    g_autoptr(GdkPixbuf) decoded = NULL;
    g_autoptr(GError) glycin_error = NULL;
    gboolean has_alpha;

    gly_loader_set_accepted_memory_formats(loader, GLY_MEMORY_SELECTION_R8G8B8A8 |
                                                       GLY_MEMORY_SELECTION_R8G8B8);
    image = gly_loader_load(loader, &glycin_error);
    if (image)
        frame = gly_image_next_frame(image, &glycin_error);
    if (!frame)
        return gdk_pixbuf_new_from_file_at_scale(path, 1024, 1024, TRUE, error);

    *animated |= gly_frame_get_delay(frame) > 0;
    has_alpha = gly_frame_get_memory_format(frame) == GLY_MEMORY_R8G8B8A8;
    decoded = gdk_pixbuf_new_from_bytes(gly_frame_get_buf_bytes(frame), GDK_COLORSPACE_RGB,
                                        has_alpha, 8, gly_frame_get_width(frame),
                                        gly_frame_get_height(frame), gly_frame_get_stride(frame));
    return fit_preview_size(decoded);
}
#else
static GdkPixbuf* load_preview_image(const gchar* path, gboolean* animated, GError** error) {
    g_autoptr(GdkPixbufAnimation) animation = NULL;

    animation = gdk_pixbuf_animation_new_from_file(path, error);
    if (!animation)
        return NULL;

    *animated |= !gdk_pixbuf_animation_is_static_image(animation);
    return fit_preview_size(gdk_pixbuf_animation_get_static_image(animation));
}
#endif

static void update_clock_label(WallpaperPreview* self, gboolean force) {
    g_autoptr(GDateTime) now = NULL;
    g_autofree gchar* label = NULL;

    now = g_date_time_new_now_local();

    if (!force && self->previous_time &&
        g_date_time_get_hour(now) == g_date_time_get_hour(self->previous_time) &&
        g_date_time_get_minute(now) == g_date_time_get_minute(self->previous_time)) {
        return;
    }

    if (self->is_24h_format)
        label = g_date_time_format(now, "%R");
    else
        label = g_date_time_format(now, "%I:%M %p");

    gtk_label_set_label(self->desktop_clock_label, label);

    g_clear_pointer(&self->previous_time, g_date_time_unref);
    self->previous_time = g_steal_pointer(&now);
}

static void update_clock_format(WallpaperPreview* self) {
    g_autofree gchar* clock_format = NULL;
    gboolean is_24h_format;

    clock_format = g_settings_get_string(self->desktop_settings, "clock-format");
    is_24h_format = g_strcmp0(clock_format, "24h") == 0;

    if (is_24h_format != self->is_24h_format) {
        self->is_24h_format = is_24h_format;
        update_clock_label(self, TRUE);
    }
}

static gboolean update_clock_cb(gpointer data) {
    WallpaperPreview* self = WALLPAPER_PREVIEW(data);

    update_clock_label(self, FALSE);

    return G_SOURCE_CONTINUE;
}

static void wallpaper_preview_finalize(GObject* object) {
    WallpaperPreview* self = WALLPAPER_PREVIEW(object);

    g_clear_object(&self->desktop_settings);
    g_clear_object(&self->image);

    g_clear_pointer(&self->previous_time, g_date_time_unref);

    g_clear_handle_id(&self->clock_time_timeout_id, g_source_remove);

    G_OBJECT_CLASS(wallpaper_preview_parent_class)->finalize(object);
}

static void wallpaper_preview_init(WallpaperPreview* self) {
    GtkWidget* spinner;

    gtk_widget_init_template(GTK_WIDGET(self));

#if ADW_CHECK_VERSION(1, 6, 0)
    spinner = adw_spinner_new();
#else
    /* libadwaita 1.5 has no AdwSpinner. */
    spinner = gtk_spinner_new();
    gtk_spinner_set_spinning(GTK_SPINNER(spinner), TRUE);
    gtk_widget_set_size_request(spinner, 32, 32);
    gtk_widget_set_halign(spinner, GTK_ALIGN_CENTER);
    gtk_widget_set_valign(spinner, GTK_ALIGN_CENTER);
#endif
    adw_bin_set_child(ADW_BIN(self->loading), spinner);

    self->desktop_settings = g_settings_new("org.gnome.desktop.interface");

    g_signal_connect_object(self->desktop_settings, "changed::clock-format",
                            G_CALLBACK(update_clock_format), self, G_CONNECT_SWAPPED);
    update_clock_format(self);

    self->clock_time_timeout_id = g_timeout_add_seconds(1, update_clock_cb, self);
}

static void wallpaper_preview_map(GtkWidget* widget) {
    static GtkCssProvider* provider;

    GTK_WIDGET_CLASS(wallpaper_preview_parent_class)->map(widget);

    if (provider == NULL) {
        provider = gtk_css_provider_new();
        gtk_css_provider_load_from_resource(
            provider, "/org/freedesktop/portal/desktop/gnome/wallpaperpreview.css");
        gtk_style_context_add_provider_for_display(gtk_widget_get_display(widget),
                                                   GTK_STYLE_PROVIDER(provider),
                                                   GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

static void wallpaper_preview_class_init(WallpaperPreviewClass* klass) {
    GObjectClass* object_class = G_OBJECT_CLASS(klass);
    GtkWidgetClass* widget_class = GTK_WIDGET_CLASS(klass);

    object_class->finalize = wallpaper_preview_finalize;
    widget_class->map = wallpaper_preview_map;

    gtk_widget_class_set_template_from_resource(
        widget_class, "/org/freedesktop/portal/desktop/gnome/wallpaperpreview.ui");

    gtk_widget_class_bind_template_child(widget_class, WallpaperPreview, stack);
    gtk_widget_class_bind_template_child(widget_class, WallpaperPreview, loading);
    gtk_widget_class_bind_template_child(widget_class, WallpaperPreview, desktop_preview);
    gtk_widget_class_bind_template_child(widget_class, WallpaperPreview, animated_background_icon);
    gtk_widget_class_bind_template_child(widget_class, WallpaperPreview, drawing_area);
    gtk_widget_class_bind_template_child(widget_class, WallpaperPreview, desktop_clock_label);
}

WallpaperPreview* wallpaper_preview_new() {
    return g_object_new(wallpaper_preview_get_type(), NULL);
}

void wallpaper_preview_set_image(WallpaperPreview* self, const gchar* image_uri) {
    g_autofree char* path = NULL;
    g_autofree char* first_image = NULL;
    g_autoptr(GFile) image_file = NULL;
    g_autoptr(GError) error = NULL;
    gboolean animated = FALSE;

    image_file = g_file_new_for_uri(image_uri);
    path = g_file_get_path(image_file);
    if (!path)
        return;
    first_image = first_slideshow_image(path, &animated);
    g_clear_object(&self->image);
    self->image = load_preview_image(first_image ?: path, &animated, &error);
    if (!self->image)
        g_warning("Could not preview wallpaper %s: %s", path, error->message);

    gtk_widget_set_visible(self->animated_background_icon, animated);
    gtk_stack_set_visible_child(GTK_STACK(self->stack), self->desktop_preview);

    gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(self->drawing_area), draw_preview_func, self,
                                   NULL);
    gtk_widget_queue_draw(self->drawing_area);
}
