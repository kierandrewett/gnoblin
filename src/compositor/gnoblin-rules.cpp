/*
 * gnoblin-shell: per-application window rules + resolved visual-effect set.
 * See gnoblin-rules.h.
 *
 * This program is free software; you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the Free
 * Software Foundation; either version 2 of the License, or (at your option)
 * any later version.
 */

#include "gnoblin-rules.h"

#include <stdlib.h>
#include <string.h>

extern "C" {
#include <meta/display.h>
}

#include "gnoblin-color-spec.h"
#include "gnoblin-config.h"
#include "gnoblin-rules-spec.h"

/* Resolved hints we cache on the window for the plugin (decoration + opacity +
 * the per-rule effect overrides). Geometry/workspace/state actions are applied
 * immediately and not stored. */
typedef struct {
    int opacity;          /* active opacity %, or -1 */
    int inactive_opacity; /* unfocused opacity %, or -1 */
    gboolean no_round;
    gboolean no_shadow;
    gboolean blur; /* legacy boolean blur toggle (kept for compatibility) */

    /* Per-rule effect overrides. Each *_set flag says "a rule touched this", so
     * the resolver knows to override the global default. */
    gboolean rounding_set;
    int rounding;
    gboolean algo_set;
    GnoblinRoundedAlgorithm algo;
    gboolean smoothing_set;
    float smoothing;
    gboolean border_set;
    float border_width;
    float border_color[4];
    gboolean border_style_set;
    GnoblinBorderStyle border_style;
    gboolean blur_set; /* rule explicitly enabled/disabled blur */
    gboolean blur_on;
    gboolean blur_radius_set;
    float blur_radius;
    gboolean blur_threshold_set;
    float blur_threshold;

    gboolean applied; /* gnoblin_rules_apply has run for this window */
} GnoblinRuleHints;

static GQuark hints_quark;

static GnoblinRuleHints* get_hints(MetaWindow* window) {
    GnoblinRuleHints* h;

    if (G_UNLIKELY(hints_quark == 0))
        hints_quark = g_quark_from_static_string("gnoblin-rule-hints");

    h = (GnoblinRuleHints*)g_object_get_qdata(G_OBJECT(window), hints_quark);
    if (!h) {
        h = g_new0(GnoblinRuleHints, 1);
        h->opacity = -1;
        h->inactive_opacity = -1;
        g_object_set_qdata_full(G_OBJECT(window), hints_quark, h, g_free);
    }
    return h;
}

const char* gnoblin_rules_layer_namespace(MetaWindow* window) {
    /* Set by the wlr-layer-shell overlay (meta-wayland-layer-shell.c) on the
     * MetaWindow backing a layer surface. NULL for ordinary toplevels (and for
     * layer surfaces when running against a libmutter built before that patch). */
    return (const char*)g_object_get_data(G_OBJECT(window), "gnoblin-layer-namespace");
}

/* Case-insensitive substring test; NULL haystack never matches. */
static gboolean contains_ci(const char* haystack, const char* needle) {
    g_autofree char* h = NULL;
    g_autofree char* n = NULL;

    if (!haystack || !needle)
        return FALSE;
    h = g_utf8_casefold(haystack, -1);
    n = g_utf8_casefold(needle, -1);
    return strstr(h, n) != NULL;
}

/* Case-insensitive regex search (GLib GRegex, PCRE syntax); NULL haystack never
 * matches. The pattern is an unanchored search — use ^/$ for whole-string. A
 * malformed pattern warns once and never matches. */
static gboolean matches_re(const char* haystack, const char* pattern) {
    g_autoptr(GError) err = NULL;
    g_autoptr(GRegex) re = NULL;

    if (!haystack || !pattern)
        return FALSE;
    re = g_regex_new(pattern, (GRegexCompileFlags)(G_REGEX_CASELESS | G_REGEX_OPTIMIZE),
                     (GRegexMatchFlags)0, &err);
    if (!re) {
        g_warning("gnoblin: invalid window-rule regex '%s': %s", pattern,
                  err ? err->message : "?");
        return FALSE;
    }
    return g_regex_match(re, haystack, (GRegexMatchFlags)0, NULL);
}

/* Test `key`'s window attribute(s) against `val`, either as a case-insensitive
 * substring (`is_re == FALSE`, the `=` operator) or a case-insensitive regex
 * (`is_re == TRUE`, the `~=` operator). */
static gboolean matcher_test(MetaWindow* window, const char* key, const char* val,
                             gboolean is_re) {
    /* A tiny helper macro: pick substring vs regex per the operator. */
#define HIT(hay) (is_re ? matches_re((hay), val) : contains_ci((hay), val))

    if (!g_strcmp0(key, "app-id"))
        /* "app id" in user terms: the GTK app id, or — for Wayland clients whose
         * xdg app_id mutter exposes as the class — the wm_class. */
        return HIT(meta_window_get_gtk_application_id(window)) ||
               HIT(meta_window_get_wm_class(window));
    if (!g_strcmp0(key, "class"))
        return HIT(meta_window_get_wm_class(window)) ||
               HIT(meta_window_get_wm_class_instance(window));
    if (!g_strcmp0(key, "title"))
        return HIT(meta_window_get_title(window));
    if (!g_strcmp0(key, "layer"))
        /* Match the wlr-layer-shell namespace (e.g. layer=gnoblin-dock). Only
         * layer surfaces carry one; toplevels never match a `layer=` matcher. */
        return HIT(gnoblin_rules_layer_namespace(window));

#undef HIT
    g_warning("gnoblin: unknown window-rule matcher '%s'", key);
    return FALSE;
}

/* Does one matcher hit this window? Two operators: `key=value` is a
 * case-insensitive substring test (backward compatible), `key~=pattern` is a
 * case-insensitive regex (GLib/PCRE) test — e.g. `class~=^firefox$`,
 * `layer~=^gnoblin-`. The `~=` form is checked first since it contains `=`. */
static gboolean matcher_hits(MetaWindow* window, const char* matcher) {
    const char* op = strstr(matcher, "~=");

    if (op) {
        g_autofree char* key = g_strndup(matcher, op - matcher);
        g_autofree char* val = g_strdup(op + 2);
        return matcher_test(window, g_strstrip(key), g_strstrip(val), TRUE);
    } else {
        g_auto(GStrv) kv = g_strsplit(matcher, "=", 2);
        if (!kv[0] || !kv[1])
            return FALSE;
        return matcher_test(window, g_strstrip(kv[0]), g_strstrip(kv[1]), FALSE);
    }
}

/* All comma-separated matchers in the part before `|` must hit. */
static gboolean rule_matches(MetaWindow* window, const char* matchers) {
    g_auto(GStrv) parts = g_strsplit(matchers, ",", -1);

    for (int i = 0; parts[i]; i++) {
        g_autofree char* m = g_strdup(g_strstrip(parts[i]));
        if (*m && !matcher_hits(window, m))
            return FALSE;
    }
    return TRUE;
}

/* Parse a corner-style token. Returns FALSE on an unknown name. */
static gboolean parse_corner_style(const char* s, GnoblinRoundedAlgorithm* out) {
    if (!g_ascii_strcasecmp(s, "circular") || !g_ascii_strcasecmp(s, "circle")) {
        *out = GNOBLIN_ROUNDED_CIRCLE;
        return TRUE;
    }
    if (!g_ascii_strcasecmp(s, "squircle") || !g_ascii_strcasecmp(s, "smooth") ||
        !g_ascii_strcasecmp(s, "continuous")) {
        *out = GNOBLIN_ROUNDED_SQUIRCLE;
        return TRUE;
    }
    return FALSE;
}

/* Parse a border-style token. */
static gboolean parse_border_style(const char* s, GnoblinBorderStyle* out) {
    if (!g_ascii_strcasecmp(s, "line") || !g_ascii_strcasecmp(s, "solid")) {
        *out = GNOBLIN_BORDER_LINE;
        return TRUE;
    }
    if (!g_ascii_strcasecmp(s, "lip") || !g_ascii_strcasecmp(s, "raised") ||
        !g_ascii_strcasecmp(s, "bevel")) {
        *out = GNOBLIN_BORDER_LIP;
        return TRUE;
    }
    if (!g_ascii_strcasecmp(s, "none") || !g_ascii_strcasecmp(s, "off")) {
        *out = GNOBLIN_BORDER_NONE;
        return TRUE;
    }
    if (!g_ascii_strcasecmp(s, "ring") || !g_ascii_strcasecmp(s, "pulse")) {
        *out = GNOBLIN_BORDER_RING;
        return TRUE;
    }
    return FALSE;
}

/* Live light/dark for the RING border colours: the dark-style toggle writes
 * "$XDG_RUNTIME_DIR/gnoblin-theme" ("dark"/"light") — same file the clients read.
 * Falls back to [appearance] theme; defaults to dark. */
static gboolean border_theme_is_dark(void) {
    const char* run = g_getenv("XDG_RUNTIME_DIR");
    if (run && *run) {
        g_autofree char* path = g_build_filename(run, "gnoblin-theme", NULL);
        g_autofree char* contents = NULL;
        if (g_file_get_contents(path, &contents, NULL, NULL) && contents) {
            g_strstrip(contents);
            return g_strcmp0(contents, "light") != 0;
        }
    }
    {
        g_autofree char* theme = gnoblin_config_get_string("appearance", "theme");
        if (theme && *theme)
            return g_ascii_strcasecmp(theme, "light") != 0;
    }
    return TRUE;
}

static void set_rgba(float c[4], float r, float g, float b, float a) {
    c[0] = r;
    c[1] = g;
    c[2] = b;
    c[3] = a;
}

/* The "ring"/"pulse" two-layer border: a light inner border + a darker outer
 * ring, both strengthening on focus; colours flip for light/dark. Widths default
 * to 1px each if a rule didn't set them. Mirrors Kieran's Tailwind spec. */
static void apply_ring_border_defaults(GnoblinRoundedParams* r, gboolean dark) {
    /* The spec is 1px each, but a 1px line washes out on this renderer; ~1.5px
     * inner border + 2px outer ring reads as the intended thin two-tone edge. */
    if (r->border_width <= 0.0f)
        r->border_width = 1.5f;
    if (r->ring_width <= 0.0f)
        r->ring_width = 2.0f;
    if (dark) {
        set_rgba(r->border_color, 1, 1, 1, 0.08f);
        set_rgba(r->border_color_focused, 1, 1, 1, 0.16f);
        set_rgba(r->ring_color, 0, 0, 0, 0.88f);
        set_rgba(r->ring_color_focused, 0, 0, 0, 0.96f);
    } else {
        set_rgba(r->border_color, 1, 1, 1, 1.00f);
        set_rgba(r->border_color_focused, 1, 1, 1, 1.00f);
        set_rgba(r->ring_color, 0, 0, 0, 0.08f);
        set_rgba(r->ring_color_focused, 0, 0, 0, 0.16f);
    }
}

/* #rrggbb[aa] -> 0..1 rgba. Tolerates a value wrapped in single/double quotes
 * (rule colours are written quoted in the config, e.g. `border 1 "#ffffff20"`,
 * and the quotes survive into the per-action token). Leaves `out` untouched on a
 * parse failure. */
static gboolean parse_color01(const char* s, float out[4]) {
    g_autofree char* copy = NULL;
    char* t;
    guint8 r, g, b, a;
    size_t len;

    if (!s)
        return FALSE;
    copy = g_strdup(s);
    t = g_strstrip(copy);
    len = strlen(t);
    if (len >= 2 && (t[0] == '"' || t[0] == '\'') && t[len - 1] == t[0]) {
        t[len - 1] = '\0';
        t++;
    }
    if (!gnoblin_color_parse_hex(t, &r, &g, &b, &a))
        return FALSE;
    out[0] = r / 255.0f;
    out[1] = g / 255.0f;
    out[2] = b / 255.0f;
    out[3] = a / 255.0f;
    return TRUE;
}

static void apply_action(MetaWindow* window, GnoblinRuleHints* hints, const char* action) {
    g_auto(GStrv) tok = g_strsplit(action, " ", 2);
    const char* verb = tok[0] ? g_strstrip(tok[0]) : "";
    const char* arg = tok[1] ? g_strstrip(tok[1]) : NULL;
    MtkRectangle r;

    if (!*verb)
        return;

    if (!g_strcmp0(verb, "float")) {
        /* gnoblin is floating already; reserved for suppressing future tiling. */
    } else if (!g_strcmp0(verb, "size") && arg) {
        int w = 0, h = 0;
        if (gnoblin_rules_parse_size(arg, &w, &h)) {
            meta_window_get_frame_rect(window, &r);
            meta_window_move_resize_frame(window, TRUE, r.x, r.y, w, h);
        }
    } else if ((!g_strcmp0(verb, "position") || !g_strcmp0(verb, "move")) && arg) {
        int x = 0, y = 0;
        if (gnoblin_rules_parse_position(arg, &x, &y)) {
            meta_window_get_frame_rect(window, &r);
            meta_window_move_resize_frame(window, TRUE, x, y, r.width, r.height);
        }
    } else if (!g_strcmp0(verb, "center")) {
        MtkRectangle wa;
        meta_window_get_frame_rect(window, &r);
        meta_window_get_work_area_current_monitor(window, &wa);
        meta_window_move_resize_frame(window, TRUE, wa.x + (wa.width - r.width) / 2,
                                      wa.y + (wa.height - r.height) / 2, r.width, r.height);
    } else if (!g_strcmp0(verb, "workspace") && arg) {
        int idx;
        if (gnoblin_rules_parse_workspace_index(arg, &idx))
            meta_window_change_workspace_by_index(window, idx, FALSE);
    } else if (!g_strcmp0(verb, "monitor") && arg) {
        int idx;
        if (gnoblin_rules_parse_monitor_index(arg, &idx))
            meta_window_move_to_monitor(window, idx);
    } else if (!g_strcmp0(verb, "sticky")) {
        meta_window_stick(window);
    } else if (!g_strcmp0(verb, "above") || !g_strcmp0(verb, "always-on-top")) {
        meta_window_make_above(window);
    } else if (!g_strcmp0(verb, "maximize")) {
        meta_window_maximize(window);
    } else if (!g_strcmp0(verb, "fullscreen")) {
        meta_window_make_fullscreen(window);
    } else if (!g_strcmp0(verb, "minimize")) {
        meta_window_minimize(window);
    } else if (!g_strcmp0(verb, "opacity") && arg) {
        int opacity;
        if (gnoblin_rules_parse_percent(arg, &opacity))
            hints->opacity = opacity;
    } else if (!g_strcmp0(verb, "inactive-opacity") && arg) {
        int opacity;
        if (gnoblin_rules_parse_percent(arg, &opacity))
            hints->inactive_opacity = opacity;
    } else if (!g_strcmp0(verb, "no-shadow")) {
        hints->no_shadow = TRUE;
    } else if (!g_strcmp0(verb, "no-round")) {
        hints->no_round = TRUE;
        hints->rounding_set = TRUE;
        hints->rounding = 0;
    } else if (!g_strcmp0(verb, "rounding") && arg) {
        int px;
        if (gnoblin_rules_parse_monitor_index(arg, &px)) { /* a non-negative int */
            hints->rounding_set = TRUE;
            hints->rounding = px;
            hints->no_round = (px <= 0);
        }
    } else if (!g_strcmp0(verb, "corner-style") && arg) {
        GnoblinRoundedAlgorithm a;
        if (parse_corner_style(arg, &a)) {
            hints->algo_set = TRUE;
            hints->algo = a;
        } else {
            g_warning("gnoblin: unknown corner-style '%s'", arg);
        }
    } else if (!g_strcmp0(verb, "smoothing") && arg) {
        double v = g_ascii_strtod(arg, NULL);
        hints->smoothing_set = TRUE;
        hints->smoothing = (float)CLAMP(v, 0.0, 1.0);
    } else if (!g_strcmp0(verb, "border") && arg) {
        /* `border <width> [#color]` — width in px, optional colour. */
        g_auto(GStrv) bt = g_strsplit(g_strstrip((char*)arg), " ", 2);
        int w = 0;
        if (bt[0] && gnoblin_rules_parse_monitor_index(g_strstrip(bt[0]), &w)) {
            hints->border_set = TRUE;
            hints->border_width = (float)w;
            /* default colour: faint white, like the chrome hairlines */
            hints->border_color[0] = hints->border_color[1] = hints->border_color[2] = 1.0f;
            hints->border_color[3] = 0.12f;
            if (bt[1] && *g_strstrip(bt[1]))
                parse_color01(g_strstrip(bt[1]), hints->border_color);
            if (!hints->border_style_set && w > 0) {
                hints->border_style_set = TRUE;
                hints->border_style = GNOBLIN_BORDER_LINE;
            }
        }
    } else if (!g_strcmp0(verb, "border-color") && arg) {
        if (parse_color01(g_strstrip((char*)arg), hints->border_color))
            hints->border_set = TRUE;
    } else if (!g_strcmp0(verb, "border-style") && arg) {
        GnoblinBorderStyle st;
        if (parse_border_style(arg, &st)) {
            hints->border_style_set = TRUE;
            hints->border_style = st;
        } else {
            g_warning("gnoblin: unknown border-style '%s'", arg);
        }
    } else if (!g_strcmp0(verb, "no-border")) {
        hints->border_style_set = TRUE;
        hints->border_style = GNOBLIN_BORDER_NONE;
        hints->border_set = TRUE;
        hints->border_width = 0.0f;
    } else if (!g_strcmp0(verb, "blur")) {
        hints->blur = TRUE;
        hints->blur_set = TRUE;
        hints->blur_on = TRUE;
        if (arg) {
            int px;
            if (gnoblin_rules_parse_monitor_index(arg, &px)) {
                hints->blur_radius_set = TRUE;
                hints->blur_radius = (float)px;
            }
        }
    } else if (!g_strcmp0(verb, "no-blur")) {
        hints->blur = FALSE;
        hints->blur_set = TRUE;
        hints->blur_on = FALSE;
    } else if ((!g_strcmp0(verb, "blur-threshold") || !g_strcmp0(verb, "blur-alpha-threshold")) &&
               arg) {
        double v = g_ascii_strtod(arg, NULL);
        hints->blur_threshold_set = TRUE;
        hints->blur_threshold = (float)CLAMP(v, 0.0, 1.0);
    } else {
        g_warning("gnoblin: unknown window-rule action '%s'", verb);
    }
}

void gnoblin_rules_apply(MetaWindow* window) {
    g_auto(GStrv) rules = gnoblin_config_get_list("window-rules", "rule");
    GnoblinRuleHints* hints = get_hints(window);

    hints->applied = TRUE;

    if (!rules)
        return;

    for (int i = 0; rules[i]; i++) {
        g_auto(GStrv) halves = g_strsplit(rules[i], "|", 2);
        if (!halves[0] || !halves[1])
            continue; /* a rule must have both a matcher and an action list */

        if (!rule_matches(window, halves[0]))
            continue;

        g_auto(GStrv) actions = g_strsplit(halves[1], ",", -1);
        for (int a = 0; actions[a]; a++) {
            g_autofree char* act = g_strdup(g_strstrip(actions[a]));
            if (*act)
                apply_action(window, hints, act);
        }
    }
}

/* ---- global defaults ([appearance] / [effects]) ---- */

/* Read a double from config (there is no double getter), with a fallback. */
static double config_get_double(const char* section, const char* key, double fallback) {
    g_autofree char* s = gnoblin_config_get_string(section, key);
    char* end = NULL;
    double v;

    if (!s || !*s)
        return fallback;
    v = g_ascii_strtod(s, &end);
    if (end == s)
        return fallback;
    return v;
}

/* Fill `out` with the global default effect set: the resolved `[appearance]` /
 * `[effects]` values before any per-rule override. Keeps `[appearance] rounding`
 * working as the global default radius (backward compatible). */
static void global_effects(GnoblinEffects* out) {
    g_autofree char* corner_style = NULL;
    g_autofree char* border_style = NULL;
    g_autofree char* border_color = NULL;
    int rounding;

    memset(out, 0, sizeof(*out));

    /* Rounding radius: [effects] rounding overrides [appearance] rounding if set;
     * otherwise the historical [appearance] rounding is the default. */
    rounding = gnoblin_config_get_int("effects", "rounding",
                                      gnoblin_config_get_int("appearance", "rounding", 0));
    out->rounded.radius = (float)MAX(rounding, 0);
    out->rounding_enabled = rounding > 0;

    /* Corner algorithm + smoothing (global). */
    out->rounded.algorithm = GNOBLIN_ROUNDED_CIRCLE;
    corner_style = gnoblin_config_get_string("effects", "corner-style");
    if (corner_style)
        parse_corner_style(g_strstrip(corner_style), &out->rounded.algorithm);
    out->rounded.smoothing =
        (float)CLAMP(config_get_double("effects", "corner-smoothing", 0.0), 0.0, 1.0);

    /* Border (global). */
    out->rounded.border_style = GNOBLIN_BORDER_NONE;
    out->rounded.border_width = (float)MAX(gnoblin_config_get_int("effects", "border-width", 0), 0);
    out->rounded.border_color[0] = out->rounded.border_color[1] = out->rounded.border_color[2] =
        1.0f;
    out->rounded.border_color[3] = 0.12f;
    border_color = gnoblin_config_get_string("effects", "border-color");
    if (border_color)
        parse_color01(g_strstrip(border_color), out->rounded.border_color);
    border_style = gnoblin_config_get_string("effects", "border-style");
    if (border_style)
        parse_border_style(g_strstrip(border_style), &out->rounded.border_style);
    else if (out->rounded.border_width > 0)
        out->rounded.border_style = GNOBLIN_BORDER_LINE;

    /* Blur (global). [effects] blur is the per-window blur radius; the historical
     * [appearance] blur drives the Overview backdrop and is left untouched. */
    out->blur_radius = (float)MAX(gnoblin_config_get_int("effects", "blur", 0), 0);
    out->blur_enabled = out->blur_radius > 0;

    /* Frost only the translucent parts of a surface: frost where the surface's
     * own alpha is below this cutoff. Default 1.0 leaves the upper-alpha gate
     * open; gnoblin-blur still requires solid-body coverage so client-side
     * shadow halos are not frosted. */
    out->blur_alpha_threshold =
        (float)CLAMP(config_get_double("effects", "blur-alpha-threshold", 1.0), 0.0, 1.0);

    /* Shadow on by default (matches today's behaviour: [appearance] shadow). */
    out->shadow_enabled = TRUE;

    /* Keep-effects-when-maximized toggles (off by default). */
    out->keep_rounded_for_maximized =
        gnoblin_config_get_bool("effects", "keep-rounded-for-maximized", FALSE);
    out->keep_rounded_for_fullscreen =
        gnoblin_config_get_bool("effects", "keep-rounded-for-fullscreen", FALSE);
}

/* gnoblin's own layer-shell chrome (topbar, dock, popouts, OSD, notifications,
 * launcher, power-menu — anything whose layer namespace matches `^gnoblin-`)
 * gets compositor blur + matching rounding by DEFAULT so it ships frosted. The
 * wallpaper/background layer is excluded: it is the bottom-most surface, so there
 * is nothing meaningful behind it to frost. This is only a default — a matching
 * `[window-rules]` rule (resolved into the per-rule hints below) overrides it,
 * and `[effects] gnoblin-chrome-blur = off` disables it wholesale. */
static gboolean is_gnoblin_chrome(const char* ns) {
    if (!ns)
        return FALSE;
    if (!g_str_has_prefix(ns, "gnoblin-"))
        return FALSE;
    /* The wallpaper/background role is the bottom layer — never frost it. */
    if (strstr(ns, "wallpaper") || strstr(ns, "background"))
        return FALSE;
    return TRUE;
}

static void apply_gnoblin_chrome_defaults(MetaWindow* window, GnoblinEffects* out) {
    const char* ns = gnoblin_rules_layer_namespace(window);

    if (!is_gnoblin_chrome(ns))
        return;
    if (!gnoblin_config_get_bool("effects", "gnoblin-chrome-blur", TRUE))
        return;

    if (!out->blur_enabled) {
        out->blur_enabled = TRUE;
        out->blur_radius = (float)MAX(gnoblin_config_get_int("effects", "gnoblin-chrome-blur-radius",
                                                             32),
                                      1);
    }
    /* Match the frost to the panels' own corner radius so the blur does not poke
     * past their rounded edges. */
    if (!out->rounding_enabled) {
        int r = gnoblin_config_get_int("effects", "gnoblin-chrome-rounding", 14);
        if (r > 0) {
            out->rounding_enabled = TRUE;
            out->rounded.radius = (float)r;
        }
    }
}

void gnoblin_rules_effects(MetaWindow* window, GnoblinEffects* out) {
    GnoblinRuleHints* hints;

    global_effects(out);

    /* Built-in default: frost gnoblin's own chrome (before per-rule overrides,
     * so config rules win). */
    apply_gnoblin_chrome_defaults(window, out);

    /* The top bar spans the whole top edge of the screen, so it gets NO rounding,
     * NO shadow and NO frost: a flush, flat, GNOME-style bar. It keeps its own
     * translucent tint. The blur-behind cannot frost a screen-edge surface
     * cleanly — there is nothing above the screen to sample, so even the
     * padded-capture blur smears a rounded halo back into the top corners (the
     * "gap" Kieran kept seeing). Rounding/shadow would also notch the bezel. An
     * explicit [window-rules] rule still wins. */
    {
        const char* ns = gnoblin_rules_layer_namespace(window);
        if (ns && g_strcmp0(ns, "gnoblin-topbar") == 0) {
            /* Flat, flush bar: NO rounding/shadow (those notched the screen edge).
             * Blur STAYS ON: the bar and its drop-down popouts share this one
             * surface, so the frost is gated per-pixel by the surface's own alpha
             * (flat bar + rounded popouts both frost; the transparent gap between
             * them does not). With rounding off the frost mask is rectangular, but
             * the alpha gate still follows the popout's rounded Slint shape. */
            out->rounding_enabled = FALSE;
            out->rounded.radius = 0.0f;
            out->shadow_enabled = FALSE;
        }
    }

    /* The dock keeps its rounded pill + soft drop-shadow, but NOT the compositor
     * frost: on the GPU path the blur capture smears into the dock's own
     * drop-shadow, painting a broken rectangular halo around the pill (Kieran's
     * dock screenshot). The frost-behind and the shadow second-pass interact
     * badly there and can't be tuned without real-HW iteration, so the dock ships
     * un-frosted (a clean translucent pill) rather than smeared. An explicit
     * [window-rules] `rule = layer~=^gnoblin-dock$ | blur N` still re-enables it. */
    /* The dock keeps the default chrome frost (rounded pill + soft shadow +
     * blur): verified clean on the real GPU render path. If a temporal flicker
     * shows up in the dock shadow live, that's the shadow second-pass, not the
     * frost — fix it there, don't disable the blur. */

    /* Make sure rules have been applied so the per-rule hints are populated. */
    hints = get_hints(window);
    if (!hints->applied)
        gnoblin_rules_apply(window);

    /* Layer over the global defaults with whatever the matching rule(s) set. */
    if (hints->rounding_set) {
        out->rounded.radius = (float)MAX(hints->rounding, 0);
        out->rounding_enabled = hints->rounding > 0;
    }
    if (hints->no_round) {
        out->rounding_enabled = FALSE;
    }
    if (hints->algo_set)
        out->rounded.algorithm = hints->algo;
    if (hints->smoothing_set)
        out->rounded.smoothing = hints->smoothing;
    if (hints->border_set) {
        out->rounded.border_width = hints->border_width;
        memcpy(out->rounded.border_color, hints->border_color, sizeof(out->rounded.border_color));
        if (!hints->border_style_set && hints->border_width > 0 &&
            out->rounded.border_style == GNOBLIN_BORDER_NONE)
            out->rounded.border_style = GNOBLIN_BORDER_LINE;
    }
    if (hints->border_style_set)
        out->rounded.border_style = hints->border_style;
    if (hints->blur_set) {
        out->blur_enabled = hints->blur_on;
        if (hints->blur_on && !hints->blur_radius_set && out->blur_radius <= 0)
            out->blur_radius = 24.0f; /* a sane default when `blur` has no radius */
    }
    if (hints->blur_radius_set)
        out->blur_radius = hints->blur_radius;
    if (hints->blur_threshold_set)
        out->blur_alpha_threshold = hints->blur_threshold;
    if (hints->no_shadow)
        out->shadow_enabled = FALSE;

    /* RING border: fill the two-layer focus-aware colours from the spec defaults
     * (theme-aware), unless a rule already set explicit ring/border colours. The
     * ring needs the rounded shader to run even with a 0 inner-border width. */
    if (out->rounded.border_style == GNOBLIN_BORDER_RING) {
        apply_ring_border_defaults(&out->rounded, border_theme_is_dark());
        out->rounding_enabled = TRUE;
    }

    /* A border with no radius still needs the rounded shader to draw it; treat a
     * border as implying the shader runs (radius 0 = square border). */
    if (out->rounded.border_style != GNOBLIN_BORDER_NONE && out->rounded.border_width > 0)
        out->rounding_enabled = TRUE;
}

gboolean gnoblin_rules_no_round(MetaWindow* window) {
    return get_hints(window)->no_round;
}

gboolean gnoblin_rules_no_shadow(MetaWindow* window) {
    return get_hints(window)->no_shadow;
}

gboolean gnoblin_rules_blur(MetaWindow* window) {
    return get_hints(window)->blur;
}

int gnoblin_rules_opacity(MetaWindow* window, gboolean focused) {
    GnoblinRuleHints* h = get_hints(window);

    if (!focused && h->inactive_opacity >= 0)
        return h->inactive_opacity;
    return h->opacity;
}
