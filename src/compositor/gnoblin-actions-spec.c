/*
 * gnoblin-shell: strict parsing for dispatcher action arguments.
 */

#include "gnoblin-actions-spec.h"
#include "gnoblin-spec-util.h"

#include <errno.h>
#include <math.h>
#include <string.h>

static gboolean parse_double(char** p, double* out) {
    char* end = NULL;
    double value;

    gnoblin_spec_skip_spaces(p);
    errno = 0;
    value = g_ascii_strtod(*p, &end);
    if (errno != 0 || end == *p || !isfinite(value))
        return FALSE;

    *out = value;
    *p = end;
    return TRUE;
}

gboolean gnoblin_actions_parse_snap_region(const char* text, double* x, double* y, double* w,
                                           double* h) {
    g_autofree char* copy = NULL;
    char* p;
    double fx = 0, fy = 0, fw = 0, fh = 0;

    if (!text || !x || !y || !w || !h)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!parse_double(&p, &fx) || !parse_double(&p, &fy) || !parse_double(&p, &fw) ||
        !parse_double(&p, &fh) || !gnoblin_spec_at_end(p) || fw <= 0.0 || fh <= 0.0)
        return FALSE;

    *x = fx;
    *y = fy;
    *w = fw;
    *h = fh;
    return TRUE;
}

gboolean gnoblin_actions_parse_workspace_index(const char* text, int* zero_based_index) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || value <= 0 || !zero_based_index)
        return FALSE;

    *zero_based_index = value - 1;
    return TRUE;
}

gboolean gnoblin_actions_parse_monitor_index(const char* text, int* index) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || value < 0 || !index)
        return FALSE;

    *index = value;
    return TRUE;
}

gboolean gnoblin_actions_parse_percent(const char* text, int* percent) {
    int value;

    if (!gnoblin_spec_parse_whole_int(text, &value) || !percent)
        return FALSE;

    *percent = CLAMP(value, 0, 100);
    return TRUE;
}

gboolean gnoblin_actions_parse_uint(const char* text, guint* out) {
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
    if (errno != 0 || end == p || value > G_MAXUINT)
        return FALSE;
    if (!gnoblin_spec_at_end(end))
        return FALSE;

    *out = (guint)value;
    return TRUE;
}
