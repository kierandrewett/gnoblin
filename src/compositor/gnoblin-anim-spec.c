/*
 * gnoblin-shell: strict parsing for animation numeric config fields.
 */

#include "gnoblin-anim-spec.h"
#include "gnoblin-spec-util.h"

gboolean gnoblin_anim_parse_duration_ms(const char* text, guint* out) {
    return gnoblin_spec_parse_uint(text, out);
}

gboolean gnoblin_anim_parse_scale(const char* text, double* out) {
    double value;

    if (!out || !gnoblin_spec_parse_double(text, &value) || value < 0.0 || value > 2.0)
        return FALSE;

    *out = value;
    return TRUE;
}
