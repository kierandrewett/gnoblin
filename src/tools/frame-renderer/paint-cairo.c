/* SPDX-License-Identifier: MIT */
#include "paint.h"
#include <cairo.h>
#include <pango/pangocairo.h>
#include <string.h>
void frame_paint_init(int* argc, char*** argv) {}
void frame_paint(void* pixels, const struct FramePaint* m) {
    cairo_surface_t* surface = cairo_image_surface_create_for_data(
        pixels, CAIRO_FORMAT_ARGB32, m->width, m->height, m->width * 4);
    cairo_t* cr = cairo_create(surface);
    cairo_set_source_rgb(cr, ((m->color >> 16) & 255) / 255., ((m->color >> 8) & 255) / 255.,
                         (m->color & 255) / 255.);
    /* Intentionally paint the whole canvas: the compositor, not the renderer,
       must enforce the content exclusion region. */
    cairo_paint(cr);
    cairo_set_source_rgb(cr, .94, .94, .94);
    PangoLayout* layout = pango_cairo_create_layout(cr);
    PangoFontDescription* font = pango_font_description_from_string("Sans 10");
    pango_layout_set_font_description(layout, font);
    pango_layout_set_width(layout,
                           MAX(0, m->width - m->left - m->right - 3 * m->top - 20) * PANGO_SCALE);
    pango_layout_set_ellipsize(layout, PANGO_ELLIPSIZE_END);
    pango_layout_set_text(layout, m->title, -1);
    cairo_move_to(cr, m->left + 10, MAX(0, (m->top - 18) / 2));
    pango_cairo_show_layout(cr, layout);
    pango_layout_set_width(layout, -1);
    for (int i = 0; i < 3; i++) {
        pango_layout_set_text(layout, (const char*[]){"×", "□", "−"}[i], -1);
        cairo_move_to(cr, m->width - m->right - (i + 1) * m->top + (m->top - 12) / 2,
                      MAX(0, (m->top - 18) / 2));
        pango_cairo_show_layout(cr, layout);
    }
    pango_font_description_free(font);
    g_object_unref(layout);
    cairo_destroy(cr);
    cairo_surface_destroy(surface);
}
