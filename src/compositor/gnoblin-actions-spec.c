/*
 * gnoblin-shell: strict parsing for dispatcher action arguments.
 */

#include "gnoblin-actions-spec.h"
#include "gnoblin-spec-util.h"

gboolean gnoblin_actions_parse_snap_region(const char* text, double* x, double* y, double* w,
                                           double* h) {
    g_autofree char* copy = NULL;
    char* p;
    double fx = 0, fy = 0, fw = 0, fh = 0;

    if (!text || !x || !y || !w || !h)
        return FALSE;

    copy = g_strdup(text);
    p = copy;
    if (!gnoblin_spec_parse_double_token(&p, &fx) || !gnoblin_spec_parse_double_token(&p, &fy) ||
        !gnoblin_spec_parse_double_token(&p, &fw) || !gnoblin_spec_parse_double_token(&p, &fh) ||
        !gnoblin_spec_at_end(p) || fw <= 0.0 || fh <= 0.0)
        return FALSE;

    *x = fx;
    *y = fy;
    *w = fw;
    *h = fh;
    return TRUE;
}

/* Thin wrappers over the shared gnoblin-spec-util parsers; rules-spec exposes
 * the same logic under gnoblin_rules_parse_*. */
gboolean gnoblin_actions_parse_workspace_index(const char* text, int* zero_based_index) {
    return gnoblin_spec_parse_workspace_index(text, zero_based_index);
}

gboolean gnoblin_actions_parse_monitor_index(const char* text, int* index) {
    /* A monitor index is just a non-negative int. */
    return gnoblin_spec_parse_nonneg_int(text, index);
}

gboolean gnoblin_actions_parse_percent(const char* text, int* percent) {
    return gnoblin_spec_parse_percent(text, percent);
}

gboolean gnoblin_actions_parse_uint(const char* text, guint* out) {
    return gnoblin_spec_parse_uint(text, out);
}
