/*
 * gnoblin-shell: shared cursor tokenizer — see gnoblin-spec-util.h.
 */

#include "gnoblin-spec-util.h"

#include <errno.h>
#include <math.h>

void gnoblin_spec_skip_spaces(char** p) {
    while (**p && g_ascii_isspace(**p))
        (*p)++;
}

gboolean gnoblin_spec_at_end(char* p) {
    gnoblin_spec_skip_spaces(&p);
    return *p == '\0';
}

gboolean gnoblin_spec_parse_int(char** p, int* out) {
    char* end = NULL;
    gint64 value;

    gnoblin_spec_skip_spaces(p);
    errno = 0;
    value = g_ascii_strtoll(*p, &end, 10);
    if (errno != 0 || end == *p || value < G_MININT || value > G_MAXINT)
        return FALSE;

    *out = (int)value;
    *p = end;
    return TRUE;
}

gboolean gnoblin_spec_parse_whole_int(const char* text, int* out) {
    g_autofree char* copy = NULL;
    char* p;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    return gnoblin_spec_parse_int(&p, out) && gnoblin_spec_at_end(p);
}

gboolean gnoblin_spec_parse_nonneg_int(const char* text, int* out) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || value < 0 || !out)
        return FALSE;

    *out = value;
    return TRUE;
}

gboolean gnoblin_spec_parse_workspace_index(const char* text, int* zero_based_index) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || value <= 0 || !zero_based_index)
        return FALSE;

    *zero_based_index = value - 1;
    return TRUE;
}

gboolean gnoblin_spec_parse_percent(const char* text, int* percent) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || !percent)
        return FALSE;

    *percent = CLAMP(value, 0, 100);
    return TRUE;
}

gboolean gnoblin_spec_parse_extent_token(char** p, int* width, int* height) {
    char* cursor;
    int w = 0;
    int h = 0;

    if (!p || !*p || !width || !height)
        return FALSE;

    cursor = *p;
    if (!gnoblin_spec_parse_int(&cursor, &w))
        return FALSE;
    gnoblin_spec_skip_spaces(&cursor);
    if (*cursor != 'x' && *cursor != 'X')
        return FALSE;
    cursor++;
    if (!gnoblin_spec_parse_int(&cursor, &h) || w <= 0 || h <= 0)
        return FALSE;

    *width = w;
    *height = h;
    *p = cursor;
    return TRUE;
}

gboolean gnoblin_spec_parse_extent(const char* text, int* width, int* height) {
    g_autofree char* copy = NULL;
    char* p;
    int w = 0;
    int h = 0;

    if (!text || !width || !height)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!gnoblin_spec_parse_extent_token(&p, &w, &h) || !gnoblin_spec_at_end(p))
        return FALSE;

    *width = w;
    *height = h;
    return TRUE;
}

gboolean gnoblin_spec_parse_uint(const char* text, guint* out) {
    g_autofree char* copy = NULL;
    char* p;
    char* end = NULL;
    guint64 value;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    gnoblin_spec_skip_spaces(&p);
    if (*p == '-')
        return FALSE;

    errno = 0;
    value = g_ascii_strtoull(p, &end, 10);
    if (errno != 0 || end == p || value > G_MAXUINT || !gnoblin_spec_at_end(end))
        return FALSE;

    *out = (guint)value;
    return TRUE;
}

gboolean gnoblin_spec_parse_double_token(char** p, double* out) {
    char* end = NULL;
    double value;

    if (!p || !*p || !out)
        return FALSE;

    gnoblin_spec_skip_spaces(p);
    errno = 0;
    value = g_ascii_strtod(*p, &end);
    if (errno != 0 || end == *p || !isfinite(value))
        return FALSE;

    *out = value;
    *p = end;
    return TRUE;
}

gboolean gnoblin_spec_parse_double(const char* text, double* out) {
    g_autofree char* copy = NULL;
    char* p;
    double value;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!gnoblin_spec_parse_double_token(&p, &value) || !gnoblin_spec_at_end(p))
        return FALSE;

    *out = value;
    return TRUE;
}
