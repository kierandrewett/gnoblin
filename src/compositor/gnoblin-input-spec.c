/*
 * gnoblin-shell: strict parsing for [input] config fields.
 */

#include "gnoblin-input-spec.h"
#include "gnoblin-spec-util.h"

gboolean gnoblin_input_parse_pointer_speed(const char* text, double* out) {
    double value;

    if (!out || !gnoblin_spec_parse_double(text, &value))
        return FALSE;

    *out = CLAMP(value, -1.0, 1.0);
    return TRUE;
}
