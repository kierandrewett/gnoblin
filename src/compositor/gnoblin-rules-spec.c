/*
 * gnoblin-shell: strict parsing for [window-rules] action arguments.
 */

#include "gnoblin-rules-spec.h"

#include <errno.h>
#include <string.h>

static void skip_spaces(char** p) {
    while (**p && g_ascii_isspace(**p))
        (*p)++;
}

static gboolean at_end(char* p) {
    skip_spaces(&p);
    return *p == '\0';
}

static gboolean parse_int(char** p, int* out) {
    char* end = NULL;
    gint64 value;

    skip_spaces(p);
    errno = 0;
    value = g_ascii_strtoll(*p, &end, 10);
    if (errno != 0 || end == *p || value < G_MININT || value > G_MAXINT)
        return FALSE;

    *out = (int)value;
    *p = end;
    return TRUE;
}

static gboolean parse_whole_int(const char* text, int* out) {
    g_autofree char* copy = NULL;
    char* p;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    return parse_int(&p, out) && at_end(p);
}

gboolean gnoblin_rules_parse_size(const char* text, int* width, int* height) {
    g_autofree char* copy = NULL;
    char* p;
    int w = 0, h = 0;

    if (!text || !width || !height)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!parse_int(&p, &w))
        return FALSE;
    skip_spaces(&p);
    if (*p != 'x' && *p != 'X')
        return FALSE;
    p++;
    if (!parse_int(&p, &h) || !at_end(p) || w <= 0 || h <= 0)
        return FALSE;

    *width = w;
    *height = h;
    return TRUE;
}

gboolean gnoblin_rules_parse_position(const char* text, int* x, int* y) {
    g_autofree char* copy = NULL;
    char* p;
    int px = 0, py = 0;

    if (!text || !x || !y)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!parse_int(&p, &px))
        return FALSE;
    if (*p == ',') {
        p++;
    } else if (g_ascii_isspace(*p)) {
        skip_spaces(&p);
    } else {
        return FALSE;
    }
    if (!parse_int(&p, &py) || !at_end(p))
        return FALSE;

    *x = px;
    *y = py;
    return TRUE;
}

gboolean gnoblin_rules_parse_workspace_index(const char* text, int* zero_based_index) {
    int value;

    if (!parse_whole_int(text, &value) || value <= 0 || !zero_based_index)
        return FALSE;

    *zero_based_index = value - 1;
    return TRUE;
}

gboolean gnoblin_rules_parse_monitor_index(const char* text, int* index) {
    int value;

    if (!parse_whole_int(text, &value) || value < 0 || !index)
        return FALSE;

    *index = value;
    return TRUE;
}

gboolean gnoblin_rules_parse_percent(const char* text, int* percent) {
    int value;

    if (!parse_whole_int(text, &value) || !percent)
        return FALSE;

    *percent = CLAMP(value, 0, 100);
    return TRUE;
}
