/*
 * gnoblin-shell: strict parsing for [output] monitor layout specs.
 */

#include "gnoblin-output-spec.h"
#include "gnoblin-spec-util.h"

#include <string.h>

static const char* token_argument(const char* token, const char* keyword) {
    gsize len = strlen(keyword);

    if (!g_str_has_prefix(token, keyword))
        return NULL;
    if (!g_ascii_isspace(token[len]))
        return NULL;
    return token + len;
}

gboolean gnoblin_output_transform_from_name(const char* text, guint* out) {
    g_autofree char* copy = NULL;
    const char* s;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    s = g_strstrip(copy);
    if (!g_strcmp0(s, "0") || !g_strcmp0(s, "normal"))
        *out = 0;
    else if (!g_strcmp0(s, "90"))
        *out = 1;
    else if (!g_strcmp0(s, "180"))
        *out = 2;
    else if (!g_strcmp0(s, "270"))
        *out = 3;
    else if (!g_strcmp0(s, "flipped"))
        *out = 4;
    else if (!g_strcmp0(s, "flipped-90"))
        *out = 5;
    else if (!g_strcmp0(s, "flipped-180"))
        *out = 6;
    else if (!g_strcmp0(s, "flipped-270"))
        *out = 7;
    else
        return FALSE;

    return TRUE;
}

static gboolean parse_mode(const char* text, GnoblinOutputSpec* out) {
    g_autofree char* copy = NULL;
    char* p;
    int w = 0, h = 0;
    double refresh = 0.0;

    copy = g_strdup(text);
    p = copy;
    if (!gnoblin_spec_parse_extent_token(&p, &w, &h))
        return FALSE;

    gnoblin_spec_skip_spaces(&p);
    if (*p == '@') {
        p++;
        if (!gnoblin_spec_parse_double(p, &refresh) || refresh <= 0.0)
            return FALSE;
    } else if (!gnoblin_spec_at_end(p)) {
        return FALSE;
    }

    out->has_mode = TRUE;
    out->mode_w = w;
    out->mode_h = h;
    out->mode_refresh = refresh;
    return TRUE;
}

static gboolean parse_scale(const char* text, GnoblinOutputSpec* out) {
    double scale;

    if (!gnoblin_spec_parse_double(text, &scale) || scale <= 0.0)
        return FALSE;

    out->has_scale = TRUE;
    out->scale = scale;
    return TRUE;
}

static gboolean parse_position(const char* text, GnoblinOutputSpec* out) {
    g_autofree char* copy = NULL;
    char* p;
    int x = 0, y = 0;

    copy = g_strdup(text);
    p = copy;
    if (!gnoblin_spec_parse_int(&p, &x))
        return FALSE;
    if (!g_ascii_isspace(*p))
        return FALSE;
    if (!gnoblin_spec_parse_int(&p, &y) || !gnoblin_spec_at_end(p))
        return FALSE;

    out->has_position = TRUE;
    out->pos_x = x;
    out->pos_y = y;
    return TRUE;
}

gboolean gnoblin_output_parse_spec(const char* value, GnoblinOutputSpec* out) {
    g_auto(GStrv) parts = NULL;
    int i;

    if (!value || !out)
        return FALSE;

    memset(out, 0, sizeof(*out));
    parts = g_strsplit(value, ",", -1);
    for (i = 0; parts && parts[i]; i++) {
        g_autofree char* token = g_strdup(g_strstrip(parts[i]));
        const char* arg;
        guint transform;

        if (*token == '\0') {
            continue;
        } else if ((arg = token_argument(token, "mode"))) {
            parse_mode(arg, out);
        } else if ((arg = token_argument(token, "scale"))) {
            parse_scale(arg, out);
        } else if ((arg = token_argument(token, "position"))) {
            parse_position(arg, out);
        } else if ((arg = token_argument(token, "transform"))) {
            if (gnoblin_output_transform_from_name(arg, &transform)) {
                out->has_transform = TRUE;
                out->transform = transform;
            }
        } else if (!g_strcmp0(token, "primary")) {
            out->primary = TRUE;
        } else if (!g_strcmp0(token, "disable") || !g_strcmp0(token, "off")) {
            out->disable = TRUE;
        }
    }

    return TRUE;
}
