/*
 * gnoblin-shell: parser for CSS-style box-shadow specs.
 */

#include "gnoblin-shadow-spec.h"

#include "gnoblin-color-spec.h"

#include <ctype.h>
#include <errno.h>
#include <float.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

static gboolean parse_finite_float(const char* s, char** end, float* out) {
    char* local_end = NULL;
    double value;

    if (!s || !out)
        return FALSE;

    errno = 0;
    value = g_ascii_strtod(s, &local_end);
    if (errno != 0 || local_end == s || !isfinite(value) || fabs(value) > FLT_MAX)
        return FALSE;

    *out = (float)value;
    if (end)
        *end = local_end;
    return TRUE;
}

gboolean gnoblin_shadow_parse_css_color(const char* s, float out[4]) {
    g_autofree char* copy = NULL;

    if (!s || !out)
        return FALSE;

    copy = g_strdup(s);
    s = g_strstrip(copy);
    if (s[0] == '#') {
        /* Delegate strict #rrggbb[aa] parsing to the shared colour parser;
         * shadow only adds the 0-1 float form on top. */
        guint8 r, g, b, a;

        if (!gnoblin_color_parse_hex(s, &r, &g, &b, &a))
            return FALSE;

        out[0] = r / 255.0f;
        out[1] = g / 255.0f;
        out[2] = b / 255.0f;
        out[3] = a / 255.0f;
        return TRUE;
    }

    if (g_str_has_prefix(s, "rgb(") || g_str_has_prefix(s, "rgba(")) {
        const char* open = strchr(s, '(');
        const char* close = open ? strchr(open, ')') : NULL;
        g_autofree char* inner = NULL;
        char *tok, *save = NULL;
        float v[4] = {0, 0, 0, 1};
        gboolean wants_alpha = g_str_has_prefix(s, "rgba(");
        int n = 0;

        if (!close || close[1] != '\0')
            return FALSE;

        inner = g_strndup(open + 1, close - (open + 1));
        for (tok = strtok_r(inner, ",", &save); tok && n < 5; tok = strtok_r(NULL, ",", &save)) {
            char* end = NULL;
            char* stripped = g_strstrip(tok);

            if (!parse_finite_float(stripped, &end, &v[n]) || *g_strstrip(end) != '\0')
                return FALSE;
            n++;
        }

        if ((!wants_alpha && n != 3) || (wants_alpha && n != 4))
            return FALSE;
        if (v[0] < 0 || v[0] > 255 || v[1] < 0 || v[1] > 255 || v[2] < 0 || v[2] > 255)
            return FALSE;
        if (wants_alpha && (v[3] < 0 || v[3] > 1))
            return FALSE;

        out[0] = v[0] / 255.0f;
        out[1] = v[1] / 255.0f;
        out[2] = v[2] / 255.0f;
        out[3] = wants_alpha ? v[3] : 1.0f;
        return TRUE;
    }

    return FALSE;
}

static gboolean parse_shadow_layer(const char* s, GnoblinShadowLayer* out) {
    const char *color = NULL, *c;
    g_autofree char* numpart = NULL;
    char *tok, *save = NULL;
    float nums[4] = {0, 0, 0, 0};
    int n = 0;

    if (!s || !out)
        return FALSE;

    for (c = s; *c; c++) {
        if (*c == '#' || g_str_has_prefix(c, "rgb")) {
            color = c;
            break;
        }
    }
    if (!color || !gnoblin_shadow_parse_css_color(color, out->color))
        return FALSE;

    numpart = g_strndup(s, color - s);
    for (tok = strtok_r(numpart, " \t", &save); tok && n < 5; tok = strtok_r(NULL, " \t", &save)) {
        char* end = NULL;

        if (!parse_finite_float(tok, &end, &nums[n]))
            return FALSE;
        if (*end && g_strcmp0(end, "px") != 0)
            return FALSE;
        n++;
    }

    if (n < 3 || n > 4 || nums[2] < 0)
        return FALSE;

    out->offset_x = nums[0];
    out->offset_y = nums[1];
    out->blur = nums[2];
    out->spread = n >= 4 ? nums[3] : 0.0f;
    return TRUE;
}

int gnoblin_shadow_parse_box_shadow(const char* css, GnoblinShadowLayer* layers, int max) {
    const char *start, *p;
    int count = 0, depth = 0;

    if (!css || !layers || max <= 0)
        return 0;

    start = css;
    for (p = css;; p++) {
        if (*p == '(') {
            depth++;
        } else if (*p == ')' && depth > 0) {
            depth--;
        }

        if ((*p == ',' && depth == 0) || *p == '\0') {
            g_autofree char* seg = g_strndup(start, p - start);
            if (count < max && parse_shadow_layer(g_strstrip(seg), &layers[count]))
                count++;
            start = p + 1;
            if (*p == '\0')
                break;
        }
    }

    return count;
}

static float shadow_layer_reach(const GnoblinShadowLayer* layer) {
    if (!layer)
        return 0.0f;
    return MAX(fabsf(layer->offset_x), fabsf(layer->offset_y)) + layer->blur +
           MAX(layer->spread, 0.0f);
}

float gnoblin_shadow_pad_for_layers(const GnoblinShadowLayer* layers, int count) {
    float pad = 0.0f;

    if (!layers || count <= 0)
        return 0.0f;

    for (int i = 0; i < count; i++)
        pad = MAX(pad, shadow_layer_reach(&layers[i]));

    return ceilf(pad) + 4.0f;
}
