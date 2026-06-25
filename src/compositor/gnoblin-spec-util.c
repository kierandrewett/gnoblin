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

gboolean gnoblin_spec_parse_double(const char* text, double* out) {
    g_autofree char* copy = NULL;
    char* s;
    char* end = NULL;
    double value;

    if (!text || !out)
        return FALSE;

    copy = g_strdup(text);
    s = g_strstrip(copy);
    if (*s == '\0')
        return FALSE;

    errno = 0;
    value = g_ascii_strtod(s, &end);
    if (errno != 0 || end == s || !isfinite(value) || !gnoblin_spec_at_end(end))
        return FALSE;

    *out = value;
    return TRUE;
}
