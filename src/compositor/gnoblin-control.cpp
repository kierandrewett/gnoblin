/*
 * gnoblin-shell: config keybindings + D-Bus/IPC -> the action dispatcher.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-control.h"

#include <gio/gio.h>
#include <string.h>

extern "C" {
#include <math.h>
#include <clutter/clutter.h>
#include <cairo.h>
#include <cogl/cogl.h>
#include <meta/compositor.h>
#include <meta/display.h>
#include <meta/meta-backend.h>
#include <meta/meta-context.h>
#include <meta/meta-multi-texture.h>
#include <meta/meta-shaped-texture.h>
#include <meta/meta-window-actor.h>
#include <meta/meta-workspace-manager.h>
#include <meta/prefs.h>
#include <meta/window.h>
}

extern "C" {
#include "gnoblin-shadow-spec.h"
}

#include "gnoblin-actions.h"
#include "gnoblin-config.h"
#include "gnoblin-roles.h"
#include "gnoblin-rules.h"

#define GNOBLIN_DBUS_NAME "dev.gnoblin.Shell"
#define GNOBLIN_DBUS_PATH "/dev/gnoblin/Shell"


typedef struct {
    char* action;
    char* arg;
} Binding;

static MetaDisplay* the_display;
static GHashTable* bindings; /* accelerator action id (guint) -> Binding* */

/* ---- keybindings ---- */

/* mutter processes its own built-in keybindings (gsettings) before our grabbed
 * accelerators, so e.g. Super+Left is taken. Since gnoblin IS the window
 * manager, make its config authoritative: disable mutter's keybinding schemas
 * so any key is free to grab. Gated by `take-over-keybindings` (default true).
 * Set it false to keep mutter's defaults (and avoid touching your dconf). */
static void disable_schema_keybindings(const char* schema_id) {
    GSettingsSchemaSource* source = g_settings_schema_source_get_default();
    g_autoptr(GSettingsSchema) schema = NULL;
    g_autoptr(GSettings) settings = NULL;
    g_auto(GStrv) keys = NULL;
    const char* empty[] = {NULL};
    int i;

    if (!source)
        return;
    schema = g_settings_schema_source_lookup(source, schema_id, TRUE);
    if (!schema)
        return;

    settings = g_settings_new_full(schema, NULL, NULL);
    keys = g_settings_schema_list_keys(schema);
    for (i = 0; keys[i]; i++) {
        g_autoptr(GSettingsSchemaKey) key = g_settings_schema_get_key(schema, keys[i]);

        if (g_variant_type_equal(g_settings_schema_key_get_value_type(key),
                                 G_VARIANT_TYPE_STRING_ARRAY))
            g_settings_set_strv(settings, keys[i], empty);
    }
    g_settings_sync();
}

void gnoblin_control_take_over_keybindings(void) {
    if (!gnoblin_config_get_bool("input", "take-over-keybindings", TRUE))
        return;

    disable_schema_keybindings("org.gnome.desktop.wm.keybindings");
    disable_schema_keybindings("org.gnome.mutter.keybindings");
    disable_schema_keybindings("org.gnome.mutter.wayland.keybindings");
}

static void binding_free(gpointer data) {
    Binding* b = (Binding*)data;

    g_free(b->action);
    g_free(b->arg);
    g_free(b);
}

/* "Super+Shift+Q" -> "<Super><Shift>q"; "Super+Left" -> "<Super>Left". */
static char* to_mutter_accelerator(const char* spec) {
    g_auto(GStrv) parts = g_strsplit(spec, "+", -1);
    GString* out = g_string_new(NULL);
    int i;

    for (i = 0; parts[i]; i++) {
        g_autofree char* tok = g_strdup(g_strstrip(parts[i]));

        if (tok[0] == '\0')
            continue;

        if (!g_ascii_strcasecmp(tok, "super") || !g_ascii_strcasecmp(tok, "meta") ||
            !g_ascii_strcasecmp(tok, "mod4") || !g_ascii_strcasecmp(tok, "win"))
            g_string_append(out, "<Super>");
        else if (!g_ascii_strcasecmp(tok, "ctrl") || !g_ascii_strcasecmp(tok, "control"))
            g_string_append(out, "<Control>");
        else if (!g_ascii_strcasecmp(tok, "alt") || !g_ascii_strcasecmp(tok, "mod1"))
            g_string_append(out, "<Alt>");
        else if (!g_ascii_strcasecmp(tok, "shift"))
            g_string_append(out, "<Shift>");
        else if (strlen(tok) == 1) /* a letter/digit key — lowercase */
            g_string_append_c(out, g_ascii_tolower(tok[0]));
        else /* a named key: Left, Up, Return, space, F1 … */
            g_string_append(out, tok);
    }

    return g_string_free(out, FALSE);
}

/* While the session is locked, config keybindings are suppressed — otherwise
 * `Super+Space` (launcher), `spawn`, `close`, etc. would bypass the lock. */
static gboolean locked;

void gnoblin_control_set_locked(gboolean is_locked) {
    locked = is_locked;
}

gboolean gnoblin_control_is_locked(void) {
    return locked;
}

static void on_accelerator_activated(MetaDisplay* display, guint action_id,
                                     ClutterInputDevice* device, guint timestamp,
                                     gpointer user_data) {
    Binding* b = (Binding*)g_hash_table_lookup(bindings, GUINT_TO_POINTER(action_id));

    if (locked)
        return;
    if (b)
        gnoblin_actions_dispatch(display, b->action, b->arg, NULL, timestamp);
}

static void ungrab_all(void) {
    GHashTableIter iter;
    gpointer key;

    if (!bindings)
        return;

    g_hash_table_iter_init(&iter, bindings);
    while (g_hash_table_iter_next(&iter, &key, NULL))
        meta_display_ungrab_accelerator(the_display, GPOINTER_TO_UINT(key));

    g_hash_table_remove_all(bindings);
}

void gnoblin_control_reload_keybindings(MetaDisplay* display) {
    /* Each line in the config's [bind] section is `Accelerator = action [arg]`,
     * so the key is the accelerator and its value is the command. */
    g_auto(GStrv) accels = gnoblin_config_get_keys("bind");
    g_autoptr(GHashTable) seen = g_hash_table_new(g_str_hash, g_str_equal);
    int i;

    the_display = display;
    if (!bindings)
        bindings = g_hash_table_new_full(NULL, NULL, NULL, binding_free);

    ungrab_all();

    if (!accels)
        return;

    for (i = 0; accels[i]; i++) {
        g_autofree char* value = NULL;
        g_autofree char* accel = NULL;
        g_auto(GStrv) cmd = NULL;
        guint action_id;
        Binding* b;

        if (g_hash_table_contains(seen, accels[i]))
            continue;
        g_hash_table_add(seen, accels[i]);

        value = gnoblin_config_get_string("bind", accels[i]);
        if (!value || value[0] == '\0')
            continue;

        accel = to_mutter_accelerator(accels[i]);
        cmd = g_strsplit(g_strstrip(value), " ", 2);

        action_id = meta_display_grab_accelerator(display, accel, META_KEY_BINDING_NONE);
        if (action_id == 0) {
            g_warning("gnoblin: could not grab '%s = %s' (%s)", accels[i], value, accel);
            continue;
        }

        b = g_new0(Binding, 1);
        b->action = g_strdup(cmd[0]);
        b->arg = cmd[1] ? g_strdup(g_strstrip(cmd[1])) : NULL;
        g_hash_table_insert(bindings, GUINT_TO_POINTER(action_id), b);
    }
}

/* ---- D-Bus / IPC ---- */

static const char introspection_xml[] =
    "<node>"
    "  <interface name='dev.gnoblin.Shell'>"
    "    <method name='Dispatch'>"
    "      <arg type='s' name='action' direction='in'/>"
    "      <arg type='s' name='arg' direction='in'/>"
    "    </method>"
    "    <method name='ListActions'>"
    "      <arg type='as' name='actions' direction='out'/>"
    "    </method>"
    "    <method name='WorkspaceState'>"
    "      <arg type='u' name='active' direction='out'/>"
    "      <arg type='u' name='count' direction='out'/>"
    "    </method>"
    "    <method name='WorkspaceNames'>"
    "      <arg type='as' name='names' direction='out'/>"
    "    </method>"
    "    <method name='ListWindows'>"
    "      <arg type='a(tssbb)' name='windows' direction='out'/>"
    "    </method>"
    "    <method name='ListWindowFrames'>"
    "      <arg type='a(tiiii)' name='frames' direction='out'/>"
    "    </method>"
    "    <method name='InspectScene'>"
    "      <arg type='s' name='json' direction='out'/>"
    "    </method>"
    "    <method name='ActivateWindow'>"
    "      <arg type='t' name='id' direction='in'/>"
    "    </method>"
    "    <method name='GetActiveWindowMenu'>"
    "      <arg type='s' name='kind' direction='out'/>"
    "      <arg type='s' name='bus_name' direction='out'/>"
    "      <arg type='s' name='app_object_path' direction='out'/>"
    "      <arg type='s' name='window_object_path' direction='out'/>"
    "      <arg type='s' name='menubar_object_path' direction='out'/>"
    "      <arg type='s' name='app_menu_object_path' direction='out'/>"
    "    </method>"
    "  </interface>"
    "</node>";

/* Per-window tuple for ListWindows: (id, title, app-id, focused, minimized).
 * `app-id` is the GTK application id, falling back to the WM class. Skips
 * non-normal and skip-taskbar windows (panels, docks, popups). */
static void build_window_list(MetaDisplay* display, GVariantBuilder* builder) {
    GList* windows = meta_display_list_all_windows(display);
    GList* l;

    for (l = windows; l; l = l->next) {
        MetaWindow* w = META_WINDOW(l->data);
        const char* title;
        const char* app;

        if (meta_window_get_window_type(w) != META_WINDOW_NORMAL || meta_window_is_skip_taskbar(w))
            continue;

        title = meta_window_get_title(w);
        app = meta_window_get_gtk_application_id(w);
        if (!app)
            app = meta_window_get_wm_class(w);

        g_variant_builder_add(
            builder, "(tssbb)", (guint64)meta_window_get_id(w), title ? title : "", app ? app : "",
            (gboolean)meta_window_has_focus(w), (gboolean)meta_window_is_hidden(w));
    }

    g_list_free(windows);
}

/* Geometry companion for tests and tooling. Keeps ListWindows stable for
 * existing clients while exposing the frame rect used by Mutter constraints. */
static void build_window_frame_list(MetaDisplay* display, GVariantBuilder* builder) {
    GList* windows = meta_display_list_all_windows(display);
    GList* l;

    for (l = windows; l; l = l->next) {
        MetaWindow* w = META_WINDOW(l->data);
        MtkRectangle frame;

        if (meta_window_get_window_type(w) != META_WINDOW_NORMAL || meta_window_is_skip_taskbar(w))
            continue;

        meta_window_get_frame_rect(w, &frame);
        g_variant_builder_add(builder, "(tiiii)", (guint64)meta_window_get_id(w), frame.x, frame.y,
                              frame.width, frame.height);
    }

    g_list_free(windows);
}


static void activate_window_by_id(MetaDisplay* display, guint64 id) {
    GList* windows = meta_display_list_all_windows(display);
    GList* l;

    for (l = windows; l; l = l->next) {
        MetaWindow* w = META_WINDOW(l->data);

        if (meta_window_get_id(w) == id) {
            meta_window_activate(w, meta_display_get_current_time_roundtrip(display));
            break;
        }
    }

    g_list_free(windows);
}

/* Append a JSON-escaped string (quotes included). */
static void json_str(GString* s, const char* v) {
    g_string_append_c(s, '"');
    for (const char* p = v ? v : ""; *p; p++) {
        if (*p == '"' || *p == '\\')
            g_string_append_c(s, '\\'), g_string_append_c(s, *p);
        else if ((unsigned char)*p < 0x20)
            g_string_append_printf(s, "\\u%04x", *p);
        else
            g_string_append_c(s, *p);
    }
    g_string_append_c(s, '"');
}

/* Append ,"key":[r,g,b,a]. */
static void json_color(GString* s, const char* key, const float c[4]) {
    g_string_append_printf(s, ",\"%s\":[%.3f,%.3f,%.3f,%.3f]", key, (double)c[0], (double)c[1],
                           (double)c[2], (double)c[3]);
}

/* Append ,"key":[v0,v1,...] — but emit `null` for any non-finite component.
 * printf prints "inf"/"-nan" for them (an UNALLOCATED actor's allocation box is
 * +/-inf and its transformed size NaN), which is INVALID JSON and silently breaks
 * the whole parse — exactly the kind of thing this inspector must never do. */
static void json_fvec(GString* s, const char* key, const float* v, int n) {
    g_string_append_printf(s, ",\"%s\":[", key);
    for (int i = 0; i < n; i++) {
        if (i)
            g_string_append_c(s, ',');
        if (isfinite((double)v[i]))
            g_string_append_printf(s, "%.0f", (double)v[i]);
        else
            g_string_append(s, "null");
    }
    g_string_append_c(s, ']');
}

/* Emit ,"fx":[...] listing the gnoblin effects attached to this actor (no public
 * Clutter API enumerates effects, so we probe our known names). */
static void json_attached_fx(GString* s, ClutterActor* a) {
    static const char* names[] = {"gnoblin-rounded", "gnoblin-blur", "gnoblin-shadow"};
    gboolean first = TRUE;

    g_string_append(s, ",\"fx\":[");
    for (guint i = 0; i < G_N_ELEMENTS(names); i++) {
        if (clutter_actor_get_effect(a, names[i])) {
            if (!first)
                g_string_append_c(s, ',');
            first = FALSE;
            json_str(s, names[i] + strlen("gnoblin-"));
        }
    }
    g_string_append_c(s, ']');
}

/* Recursively dump a Clutter actor subtree (the GTK-inspector object tree): each
 * node's runtime type, name, geometry, opacity, mapped state, attached gnoblin
 * effects and children, down to `maxdepth`. */
static void dump_actor_tree(GString* s, ClutterActor* a, int depth, int maxdepth) {
    float x = 0, y = 0, w = 0, h = 0;
    double csx = 1.0, csy = 1.0;
    const char* name = clutter_actor_get_name(a);

    clutter_actor_get_position(a, &x, &y);
    clutter_actor_get_size(a, &w, &h);
    clutter_actor_get_scale(a, &csx, &csy);

    g_string_append(s, "{\"gtype\":");
    json_str(s, G_OBJECT_TYPE_NAME(a));
    g_string_append(s, ",\"name\":");
    json_str(s, name ? name : "");
    {
        float pos[2] = {x, y}, size[2] = {w, h};
        json_fvec(s, "pos", pos, 2);
        json_fvec(s, "size", size, 2);
        g_string_append(s, ",\"scale\":[");
        if (isfinite(csx))
            g_string_append_printf(s, "%.2f", csx);
        else
            g_string_append(s, "null");
        g_string_append_c(s, ',');
        if (isfinite(csy))
            g_string_append_printf(s, "%.2f", csy);
        else
            g_string_append(s, "null");
        g_string_append_printf(s, "],\"opacity\":%d,\"mapped\":%s",
                               (int)clutter_actor_get_opacity(a),
                               clutter_actor_is_mapped(a) ? "true" : "false");
    }
    /* The shaped-texture content's preferred size reveals the buffer_scale mutter
     * applied: a 2560px buffer at buffer_scale 2 reports 1280 logical here; if it
     * reports 2560 the scale was dropped (the HiDPI 2× layer-surface bug). */
    {
        ClutterContent* content = clutter_actor_get_content(a);
        float cw = 0, ch = 0;
        if (content && clutter_content_get_preferred_size(content, &cw, &ch)) {
            float c[2] = {cw, ch};
            json_fvec(s, "content", c, 2);
        }
    }
    {
        /* An UNALLOCATED actor's box is +/-inf and its transformed size NaN —
         * json_fvec emits those as null rather than the JSON-breaking inf/-nan. */
        ClutterActorBox box;
        clutter_actor_get_allocation_box(a, &box);
        float ab[4] = {box.x1, box.y1, box.x2, box.y2};
        json_fvec(s, "alloc", ab, 4);
    }
    {
        /* Size after the FULL ancestor transform chain (stage coords). If this is
         * 2× the alloc, there's a hidden transform scale (not visible in
         * get_scale) — the layer-surface HiDPI 2× bug. */
        float tw = 0, th = 0;
        clutter_actor_get_transformed_size(a, &tw, &th);
        float ts[2] = {tw, th};
        json_fvec(s, "txsize", ts, 2);
    }
    {
        /* Offscreen redirect: an actor rendered to an intermediate FBO (effects,
         * forced redirect) can be composited at the wrong scale on HiDPI. The
         * actor's resource scale drives the FBO resolution. */
        ClutterOffscreenRedirect r = clutter_actor_get_offscreen_redirect(a);
        double rs = clutter_actor_get_resource_scale(a);
        g_string_append_printf(s, ",\"redirect\":%d", (int)r);
        if (isfinite(rs))
            g_string_append_printf(s, ",\"rscale\":%.2f", rs);
        else
            g_string_append(s, ",\"rscale\":null");
    }
    json_attached_fx(s, a);
    g_string_append(s, ",\"children\":[");
    if (depth < maxdepth) {
        GList* kids = clutter_actor_get_children(a);
        GList* k;
        gboolean first = TRUE;

        for (k = kids; k; k = k->next) {
            if (!first)
                g_string_append_c(s, ',');
            first = FALSE;
            dump_actor_tree(s, CLUTTER_ACTOR(k->data), depth + 1, maxdepth);
        }
        g_list_free(kids);
    }
    g_string_append(s, "]}");
}

/* Resolve a window's box-shadow spec the way maybe_add_shadow does: a
 * type-specific `shadow.<type>` overrides the global `shadow`; menu-like types
 * take only their explicit key. Newly-allocated string or NULL. */
static char* resolve_shadow_spec(MetaWindow* w) {
    const char* tn = NULL;
    gboolean menu_like = FALSE;
    char* spec = NULL;

    switch (meta_window_get_window_type(w)) {
    case META_WINDOW_NORMAL:
        tn = "normal";
        break;
    case META_WINDOW_DIALOG:
        tn = "dialog";
        break;
    case META_WINDOW_MODAL_DIALOG:
        tn = "modal-dialog";
        break;
    case META_WINDOW_MENU:
        tn = "menu", menu_like = TRUE;
        break;
    case META_WINDOW_DROPDOWN_MENU:
        tn = "dropdown-menu", menu_like = TRUE;
        break;
    case META_WINDOW_POPUP_MENU:
        tn = "popup-menu", menu_like = TRUE;
        break;
    case META_WINDOW_UTILITY:
        tn = "utility";
        break;
    case META_WINDOW_COMBO:
        tn = "combo", menu_like = TRUE;
        break;
    default:
        break;
    }
    if (tn) {
        g_autofree char* key = g_strdup_printf("shadow.%s", tn);
        spec = gnoblin_config_get_string("appearance", key);
    }
    if (!spec && !menu_like)
        spec = gnoblin_config_get_string("appearance", "shadow");
    return spec;
}

static const char* border_style_name(GnoblinBorderStyle st) {
    switch (st) {
    case GNOBLIN_BORDER_NONE:
        return "none";
    case GNOBLIN_BORDER_LINE:
        return "line";
    case GNOBLIN_BORDER_LIP:
        return "lip";
    case GNOBLIN_BORDER_RING:
        return "ring";
    default:
        return "?";
    }
}

/* Find the first descendant surface actor's shaped texture (its ClutterContent). */
static MetaShapedTexture* find_shaped_texture(ClutterActor* a) {
    ClutterContent* content = clutter_actor_get_content(a);
    if (content && META_IS_SHAPED_TEXTURE(content))
        return META_SHAPED_TEXTURE(content);
    GList* kids = clutter_actor_get_children(a);
    MetaShapedTexture* found = NULL;
    for (GList* k = kids; k && !found; k = k->next)
        found = find_shaped_texture(CLUTTER_ACTOR(k->data));
    g_list_free(kids);
    return found;
}

/* DIAGNOSTIC (GNOBLIN_DUMP_TEXTURE=<dir>): write each window's shaped-texture
 * content — what mutter actually holds, BEFORE compositing — to tex-<pid>.png.
 * Distinguishes a too-big received texture (Mesa/dmabuf import) from a correct
 * texture scaled up at composite time (the HiDPI EGL-layer 2× bug). */
static void dump_surface_textures(MetaDisplay* display) {
    const char* dir = g_getenv("GNOBLIN_DUMP_TEXTURE");
    if (!dir)
        return;
    MetaCompositor* compositor = meta_display_get_compositor(display);
    GList* actors = meta_compositor_get_window_actors(compositor);
    for (GList* l = actors; l; l = l->next) {
        MetaWindowActor* wa = META_WINDOW_ACTOR(l->data);
        MetaWindow* w = meta_window_actor_get_meta_window(wa);
        if (!w)
            continue;
        MetaShapedTexture* stex = find_shaped_texture(CLUTTER_ACTOR(wa));
        if (!stex)
            continue;
        /* Read the cogl texture DIRECTLY (meta_shaped_texture_get_image bails on
         * dmabuf/external textures via should_get_via_offscreen). cogl handles the
         * readback for external textures internally. This shows what mutter holds
         * for the EGL/dmabuf topbar — at PHYSICAL texture size. */
        MetaMultiTexture* mtex = meta_shaped_texture_get_texture(stex);
        if (!mtex)
            continue;
        CoglTexture* tex = meta_multi_texture_get_plane(mtex, 0);
        if (!tex)
            continue;
        int tw = cogl_texture_get_width(tex);
        int th = cogl_texture_get_height(tex);
        cairo_surface_t* img = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, tw, th);
        cogl_texture_get_data(tex, COGL_PIXEL_FORMAT_BGRA_8888_PRE,
                              cairo_image_surface_get_stride(img),
                              cairo_image_surface_get_data(img));
        cairo_surface_mark_dirty(img);
        char path[512];
        g_snprintf(path, sizeof(path), "%s/tex-%d-%dx%d.png", dir,
                   (int)meta_window_get_pid(w), tw, th);
        cairo_surface_write_to_png(img, path);
        cairo_surface_destroy(img);
    }
}

/* Dump the live scene as JSON — the gnoblin equivalent of the GTK inspector's
 * object tree + properties. For every window actor / layer surface: geometry
 * (frame vs buffer rect, actor allocation, CSD inset), live Clutter actor state
 * (position/size/opacity/scale/visibility/children), the resolved gnoblin effect
 * set with FULL parameters (rounding radius/algorithm/smoothing, the two-layer
 * ring border widths + every colour incl. focused variants, blur radius +
 * alpha-gate, shadow), which gnoblin effects are actually attached, and window
 * state (wm-class, app-id, pid, monitor, stacking layer, maximized/fullscreen/
 * focus, SSD-vs-CSD). Tooling reads this instead of guessing from pixels. */
static char* build_scene_json(MetaDisplay* display) {
    MetaCompositor* compositor = meta_display_get_compositor(display);
    GList* actors = meta_compositor_get_window_actors(compositor);
    GList* l;
    MetaBackend* backend = meta_context_get_backend(meta_display_get_context(display));
    ClutterActor* stage = meta_backend_get_stage(backend);
    float stage_w = 0, stage_h = 0;
    clutter_actor_get_size(stage, &stage_w, &stage_h);
    GString* s = g_string_new("{");
    /* Stage coord space is LOGICAL (==monitor logical size) when stage views are
     * scaled, PHYSICAL (==mode) otherwise. So stage_w == logical width means the
     * HiDPI "scaled framebuffer" path is active. */
    g_string_append_printf(s, "\"stage\":[%.0f,%.0f],", stage_w, stage_h);
    g_string_append(s, "\"surfaces\":[");
    gboolean first = TRUE;

    for (l = actors; l; l = l->next) {
        MetaWindowActor* wa = META_WINDOW_ACTOR(l->data);
        MetaWindow* w = meta_window_actor_get_meta_window(wa);
        ClutterActor* actor = CLUTTER_ACTOR(wa);
        MtkRectangle frame = {0, 0, 0, 0};
        MtkRectangle buffer = {0, 0, 0, 0};
        const char* ns;
        const char* title;
        const char* wmclass;
        const char* appid;
        float aw = 0, ah = 0, ax = 0, ay = 0;
        double sx = 1.0, sy = 1.0;
        GnoblinEffects fx;
        gboolean has_rounded, has_blur;
        ClutterEffect *rounded_fx, *blur_fx;
        /* CSD inset, clamped to >=0 the way the rounded shader applies it. */
        int il, it, ir, ib;

        if (!w)
            continue;

        meta_window_get_frame_rect(w, &frame);
        meta_window_get_buffer_rect(w, &buffer);
        clutter_actor_get_size(actor, &aw, &ah);
        clutter_actor_get_position(actor, &ax, &ay);
        clutter_actor_get_scale(actor, &sx, &sy);
        ns = gnoblin_rules_layer_namespace(w);
        title = meta_window_get_title(w);
        wmclass = meta_window_get_wm_class(w);
        appid = meta_window_get_gtk_application_id(w);
        rounded_fx = clutter_actor_get_effect(actor, "gnoblin-rounded");
        blur_fx = clutter_actor_get_effect(actor, "gnoblin-blur");
        has_rounded = rounded_fx != NULL;
        has_blur = blur_fx != NULL;
        gnoblin_rules_effects(w, &fx);
        il = MAX(0, frame.x - buffer.x);
        it = MAX(0, frame.y - buffer.y);
        ir = MAX(0, (buffer.x + buffer.width) - (frame.x + frame.width));
        ib = MAX(0, (buffer.y + buffer.height) - (frame.y + frame.height));

        if (!first)
            g_string_append_c(s, ',');
        first = FALSE;
        g_string_append_c(s, '{');
        g_string_append_printf(s, "\"id\":%" G_GUINT64_FORMAT, (guint64)meta_window_get_id(w));
        g_string_append(s, ",\"title\":");
        json_str(s, title ? title : "");
        g_string_append(s, ",\"layer_ns\":");
        json_str(s, ns ? ns : "");
        g_string_append(s, ",\"wm_class\":");
        json_str(s, wmclass ? wmclass : "");
        g_string_append(s, ",\"app_id\":");
        json_str(s, appid ? appid : "");
        g_string_append_printf(s, ",\"type\":%d", (int)meta_window_get_window_type(w));
        g_string_append_printf(s, ",\"pid\":%d", (int)meta_window_get_pid(w));
        g_string_append_printf(s, ",\"monitor\":%d", meta_window_get_monitor(w));
        g_string_append_printf(s, ",\"stack_layer\":%d", (int)meta_window_get_layer(w));

        /* Window state. */
        g_string_append_printf(
            s, ",\"state\":{\"focused\":%s,\"maximized\":%s,\"fullscreen\":%s,\"all_ws\":%s,\"ssd\":%s}",
            meta_window_has_focus(w) ? "true" : "false",
            meta_window_is_maximized(w) ? "true" : "false",
            meta_window_is_fullscreen(w) ? "true" : "false",
            meta_window_is_on_all_workspaces(w) ? "true" : "false",
            (il || it || ir || ib) ? "false" : "true");

        /* Geometry. */
        g_string_append_printf(s, ",\"frame\":[%d,%d,%d,%d]", frame.x, frame.y, frame.width,
                               frame.height);
        g_string_append_printf(s, ",\"buffer\":[%d,%d,%d,%d]", buffer.x, buffer.y, buffer.width,
                               buffer.height);
        g_string_append_printf(s, ",\"csd_inset\":[%d,%d,%d,%d]", il, it, ir, ib);

        /* Live Clutter actor state. pos/size/scale go through the same non-finite
         * guard as dump_actor_tree (an unmapped/mid-animation actor can report
         * inf/NaN, which would emit invalid JSON and break the whole parse). */
        {
            float apos[2] = {ax, ay}, asize[2] = {aw, ah};
            g_string_append_printf(s, ",\"actor\":{\"opacity\":%d",
                                   (int)clutter_actor_get_opacity(actor));
            json_fvec(s, "pos", apos, 2);
            json_fvec(s, "size", asize, 2);
            g_string_append(s, ",\"scale\":[");
            if (isfinite(sx))
                g_string_append_printf(s, "%.3f", sx);
            else
                g_string_append(s, "null");
            g_string_append_c(s, ',');
            if (isfinite(sy))
                g_string_append_printf(s, "%.3f", sy);
            else
                g_string_append(s, "null");
            g_string_append_printf(s,
                                   "],\"visible\":%s,\"mapped\":%s,\"reactive\":%s,\"z\":%.1f,"
                                   "\"clip\":%s,\"children\":%d}",
                                   clutter_actor_is_visible(actor) ? "true" : "false",
                                   clutter_actor_is_mapped(actor) ? "true" : "false",
                                   clutter_actor_get_reactive(actor) ? "true" : "false",
                                   (double)clutter_actor_get_z_position(actor),
                                   clutter_actor_has_clip(actor) ? "true" : "false",
                                   clutter_actor_get_n_children(actor));
        }

        /* Paint box: the actor's 2D bounding box in STAGE coords — what actually
         * gets painted (incl. SSD titlebar/decorations + effect margins), which is
         * what the rounding effect's offscreen TEXTURE is sized to. Comparing it to
         * the frame rect reveals where the window sits inside the texture — the key
         * to rounding the frame (not the padded texture) on SSD windows. */
        {
            ClutterActorBox pb;
            if (clutter_actor_get_paint_box(actor, &pb))
                g_string_append_printf(s, ",\"paint_box\":[%.0f,%.0f,%.0f,%.0f]", (double)pb.x1,
                                       (double)pb.y1, (double)(pb.x2 - pb.x1),
                                       (double)(pb.y2 - pb.y1));
        }

        /* Rounding + the two-layer ring border, FULL parameters. */
        g_string_append_printf(s,
                               ",\"rounding\":{\"enabled\":%s,\"radius\":%.1f,\"algorithm\":%d,"
                               "\"smoothing\":%.3f,\"applied_inset\":[%d,%d,%d,%d]",
                               fx.rounding_enabled ? "true" : "false", (double)fx.rounded.radius,
                               (int)fx.rounded.algorithm, (double)fx.rounded.smoothing, il, it, ir,
                               ib);
        /* The ACTUAL applied params read off the live effect: corner_fill +
         * adaptive are decided per-window in maybe_round_corners, NOT in the
         * resolved rules, so the inspector could not see them before. Plus the
         * offscreen FBO size the effect renders to (should equal the window paint
         * box x resource_scale; a mismatch is the HiDPI/SSD geometry bug). */
        if (has_rounded) {
            GnoblinRoundedParams ap;
            gboolean rfocused = FALSE;

            if (gnoblin_rounded_get_params(rounded_fx, &ap, &rfocused))
                g_string_append_printf(s,
                                       ",\"corner_fill\":%s,\"adaptive\":%s,\"adapt_shade\":%.3f,"
                                       "\"adapt_light\":%.3f,\"focused\":%s",
                                       ap.corner_fill ? "true" : "false",
                                       ap.adaptive ? "true" : "false", (double)ap.adapt_shade,
                                       (double)ap.adapt_light, rfocused ? "true" : "false");
            CoglTexture* rt =
                clutter_offscreen_effect_get_texture(CLUTTER_OFFSCREEN_EFFECT(rounded_fx));
            if (rt)
                g_string_append_printf(s, ",\"fbo\":[%d,%d]", cogl_texture_get_width(rt),
                                       cogl_texture_get_height(rt));
        }
        g_string_append_c(s, '}');
        g_string_append(s, ",\"border\":{");
        g_string_append_printf(s, "\"style\":");
        json_str(s, border_style_name(fx.rounded.border_style));
        g_string_append_printf(s, ",\"border_width\":%.2f,\"ring_width\":%.2f",
                               (double)fx.rounded.border_width, (double)fx.rounded.ring_width);
        json_color(s, "border_color", fx.rounded.border_color);
        json_color(s, "border_color_focused", fx.rounded.border_color_focused);
        json_color(s, "ring_color", fx.rounded.ring_color);
        json_color(s, "ring_color_focused", fx.rounded.ring_color_focused);
        g_string_append_c(s, '}');

        /* Blur + shadow. */
        g_string_append_printf(
            s, ",\"blur\":{\"enabled\":%s,\"radius\":%.1f,\"alpha_threshold\":%.3f}",
            fx.blur_enabled ? "true" : "false", (double)fx.blur_radius,
            (double)fx.blur_alpha_threshold);
        g_string_append_printf(s, ",\"shadow\":{\"enabled\":%s",
                               fx.shadow_enabled ? "true" : "false");
        {
            g_autofree char* spec = resolve_shadow_spec(w);

            if (spec && *spec) {
                GnoblinShadowLayer layers[GNOBLIN_SHADOW_MAX_LAYERS];
                int n = gnoblin_shadow_parse_box_shadow(spec, layers, GNOBLIN_SHADOW_MAX_LAYERS);
                int i;

                g_string_append(s, ",\"layers\":[");
                for (i = 0; i < n; i++) {
                    if (i)
                        g_string_append_c(s, ',');
                    g_string_append_printf(
                        s, "{\"offset\":[%.1f,%.1f],\"blur\":%.1f,\"spread\":%.1f", (double)layers[i].offset_x,
                        (double)layers[i].offset_y, (double)layers[i].blur, (double)layers[i].spread);
                    json_color(s, "color", layers[i].color);
                    g_string_append_c(s, '}');
                }
                g_string_append_c(s, ']');
            }
        }
        g_string_append_c(s, '}');

        /* Which gnoblin effects are attached AND whether they're currently enabled
         * — the effect stays attached but gets DISABLED while a window is
         * maximized/fullscreen (or mid-animation), so attached != active. */
        g_string_append_printf(
            s, ",\"attached\":{\"rounded\":%s,\"blur\":%s},\"enabled\":{\"rounded\":%s,\"blur\":%s}",
            has_rounded ? "true" : "false", has_blur ? "true" : "false",
            (has_rounded && clutter_actor_meta_get_enabled(CLUTTER_ACTOR_META(rounded_fx)))
                ? "true"
                : "false",
            (has_blur && clutter_actor_meta_get_enabled(CLUTTER_ACTOR_META(blur_fx))) ? "true"
                                                                                      : "false");

        /* The drop-shadow is a SIBLING actor pinned below the window (data key
         * "gnoblin-shadow"); report its geometry + that the shadow effect runs. */
        {
            ClutterActor* sh =
                (ClutterActor*)g_object_get_data(G_OBJECT(actor), "gnoblin-shadow");

            if (sh) {
                float sxp = 0, syp = 0, swp = 0, shp = 0;

                clutter_actor_get_position(sh, &sxp, &syp);
                clutter_actor_get_size(sh, &swp, &shp);
                g_string_append_printf(s,
                                       ",\"shadow_actor\":{\"pos\":[%.0f,%.0f],\"size\":[%.0f,%.0f],"
                                       "\"opacity\":%d,\"mapped\":%s}",
                                       (double)sxp, (double)syp, (double)swp, (double)shp,
                                       (int)clutter_actor_get_opacity(sh),
                                       clutter_actor_is_mapped(sh) ? "true" : "false");
            }
        }

        /* The Clutter actor subtree (object tree, like the GTK inspector). */
        g_string_append(s, ",\"tree\":");
        dump_actor_tree(s, actor, 0, 3);
        g_string_append_c(s, '}');
    }
    g_string_append(s, "]}");
    return g_string_free(s, FALSE);
}

static void handle_method_call(GDBusConnection* connection, const char* sender,
                               const char* object_path, const char* interface_name,
                               const char* method_name, GVariant* parameters,
                               GDBusMethodInvocation* invocation, gpointer user_data) {
    if (!g_strcmp0(method_name, "Dispatch")) {
        const char *action = NULL, *arg = NULL;

        g_variant_get(parameters, "(&s&s)", &action, &arg);
        if (!locked || !g_strcmp0(action, "lock"))
            gnoblin_actions_dispatch(the_display, action, (arg && arg[0]) ? arg : NULL, NULL, 0);
        g_dbus_method_invocation_return_value(invocation, NULL);
    } else if (!g_strcmp0(method_name, "ListActions")) {
        g_dbus_method_invocation_return_value(invocation,
                                              g_variant_new("(^as)", gnoblin_actions_list()));
    } else if (!g_strcmp0(method_name, "WorkspaceState")) {
        MetaWorkspaceManager* wm = meta_display_get_workspace_manager(the_display);
        guint active = (guint)meta_workspace_manager_get_active_workspace_index(wm);
        guint count = (guint)meta_workspace_manager_get_n_workspaces(wm);

        g_dbus_method_invocation_return_value(invocation, g_variant_new("(uu)", active, count));
    } else if (!g_strcmp0(method_name, "WorkspaceNames")) {
        /* The configured `[workspaces]` index->name labels (1-based keys), padded
         * to the live workspace count; an unnamed workspace yields "". The topbar
         * renders these instead of bare numbers. */
        MetaWorkspaceManager* wm = meta_display_get_workspace_manager(the_display);
        guint count = (guint)meta_workspace_manager_get_n_workspaces(wm);
        GVariantBuilder builder;
        guint i;

        g_variant_builder_init(&builder, G_VARIANT_TYPE("as"));
        for (i = 0; i < count; i++) {
            g_autofree char* key = g_strdup_printf("%u", i + 1);
            g_autofree char* name = gnoblin_config_get_string("workspaces", key);

            g_variant_builder_add(&builder, "s", name ? name : "");
        }
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(as)", &builder));
    } else if (!g_strcmp0(method_name, "ListWindows")) {
        GVariantBuilder builder;

        g_variant_builder_init(&builder, G_VARIANT_TYPE("a(tssbb)"));
        build_window_list(the_display, &builder);
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(a(tssbb))", &builder));
    } else if (!g_strcmp0(method_name, "ListWindowFrames")) {
        GVariantBuilder builder;

        g_variant_builder_init(&builder, G_VARIANT_TYPE("a(tiiii)"));
        build_window_frame_list(the_display, &builder);
        g_dbus_method_invocation_return_value(invocation, g_variant_new("(a(tiiii))", &builder));
    } else if (!g_strcmp0(method_name, "InspectScene")) {
        dump_surface_textures(the_display);
        g_autofree char* json = build_scene_json(the_display);

        g_dbus_method_invocation_return_value(invocation, g_variant_new("(s)", json));
    } else if (!g_strcmp0(method_name, "ActivateWindow")) {
        guint64 id = 0;

        g_variant_get(parameters, "(t)", &id);
        if (!locked)
            activate_window_by_id(the_display, id);
        g_dbus_method_invocation_return_value(invocation, NULL);
    } else if (!g_strcmp0(method_name, "GetActiveWindowMenu")) {
        /* The GTK menu export (org.gtk.Menus / org.gtk.Actions) of the focused
         * window, propagated to mutter via gtk-shell / X11 _GTK_* properties. KDE/Qt
         * windows can also publish a com.canonical.dbusmenu address via the KDE
         * appmenu Wayland protocol. Returns the backend kind + bus name + object
         * paths so the topbar can render a KDE-style global menu. All empty when
         * nothing is focused or the app exports no menu.
         *
         * The appmenu is a toggleable API — when [features] appmenu = off we report
         * no menu, so any client honouring the API hides its menu bar. */
        MetaWindow* w = (!locked && gnoblin_feature_enabled("appmenu", TRUE))
                            ? meta_display_get_focus_window(the_display)
                            : NULL;
        g_autofree char* backend = gnoblin_config_get_string("topbar", "appmenu-backend");
        const gboolean allow_gtk =
            !backend || backend[0] == '\0' || !g_ascii_strcasecmp(backend, "auto") ||
            !g_ascii_strcasecmp(backend, "both") || !g_ascii_strcasecmp(backend, "gtk");
        const gboolean allow_kde =
            !backend || backend[0] == '\0' || !g_ascii_strcasecmp(backend, "auto") ||
            !g_ascii_strcasecmp(backend, "both") || !g_ascii_strcasecmp(backend, "kde") ||
            !g_ascii_strcasecmp(backend, "dbusmenu");
        const gboolean disabled =
            backend && (!g_ascii_strcasecmp(backend, "off") || !g_ascii_strcasecmp(backend, "none"));
        const char* kind = "";
        const char* bus = NULL;
        const char* app = NULL;
        const char* win = NULL;
        const char* bar = NULL;
        const char* appmenu = NULL;

        if (w && !disabled && allow_kde) {
            bus = meta_window_get_kde_appmenu_service_name(w);
            bar = meta_window_get_kde_appmenu_object_path(w);
            if (bus && bus[0] && bar && bar[0])
                kind = "dbusmenu";
            else
                bus = bar = NULL;
        }

        if (w && !disabled && kind[0] == '\0' && allow_gtk) {
            bus = meta_window_get_gtk_unique_bus_name(w);
            app = meta_window_get_gtk_application_object_path(w);
            win = meta_window_get_gtk_window_object_path(w);
            bar = meta_window_get_gtk_menubar_object_path(w);
            appmenu = meta_window_get_gtk_app_menu_object_path(w);
            if (bus && bus[0] && bar && bar[0])
                kind = "gtk";
        }

        g_dbus_method_invocation_return_value(
            invocation, g_variant_new("(ssssss)", kind, bus ? bus : "", app ? app : "", win ? win : "",
                                      bar ? bar : "", appmenu ? appmenu : ""));
    } else {
        g_dbus_method_invocation_return_error(invocation, G_DBUS_ERROR, G_DBUS_ERROR_UNKNOWN_METHOD,
                                              "Unknown method %s", method_name);
    }
}

static const GDBusInterfaceVTable interface_vtable = {handle_method_call, NULL, NULL, {0}};

static void on_bus_acquired(GDBusConnection* connection, const char* name, gpointer user_data) {
    g_autoptr(GDBusNodeInfo) node = NULL;
    g_autoptr(GError) error = NULL;

    node = g_dbus_node_info_new_for_xml(introspection_xml, &error);
    if (!node) {
        g_warning("gnoblin: bad D-Bus introspection: %s", error->message);
        return;
    }

    if (g_dbus_connection_register_object(connection, GNOBLIN_DBUS_PATH, node->interfaces[0],
                                          &interface_vtable, NULL, NULL, &error) == 0)
        g_warning("gnoblin: failed to register %s: %s", GNOBLIN_DBUS_PATH, error->message);
}

void gnoblin_control_init(MetaDisplay* display) {
    the_display = display;

    g_signal_connect(display, "accelerator-activated", G_CALLBACK(on_accelerator_activated),
                     NULL);

    gnoblin_control_reload_keybindings(display);

    g_bus_own_name(G_BUS_TYPE_SESSION, GNOBLIN_DBUS_NAME, G_BUS_NAME_OWNER_FLAGS_REPLACE,
                   on_bus_acquired, NULL, NULL, NULL, NULL);
}
