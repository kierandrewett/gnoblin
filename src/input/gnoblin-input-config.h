#pragma once

#include <glib.h>

/* Return a vardict copy with the validated `input` subtree in Mutter's native
 * GVariant types. Documents without input are returned by reference. */
GVariant* gnoblin_native_input_config_normalize(GVariant* document);

/* Normalize a validated `input` vardict into Mutter's native GVariant types. */
GVariant* gnoblin_native_input_normalize(GVariant* input);
