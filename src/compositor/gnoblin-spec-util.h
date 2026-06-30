/*
 * gnoblin-shell: shared cursor tokenizer for the strict [section]-spec parsers.
 *
 * Every gnoblin-*-spec.c parser walks a config-string with the same primitives
 * (skip whitespace, parse a base-10 int, check for trailing junk). This is the
 * single copy they share, rather than re-declaring identical statics in each.
 */
#pragma once

#include <glib.h>

G_BEGIN_DECLS

/* Advance *p past ASCII whitespace. */
void gnoblin_spec_skip_spaces(char** p);

/* TRUE if only whitespace (or nothing) remains from p onward. */
gboolean gnoblin_spec_at_end(char* p);

/* Parse a base-10 int at *p (skipping leading whitespace), advancing *p past the
 * digits. FALSE on no digits / overflow / errno. */
gboolean gnoblin_spec_parse_int(char** p, int* out);

/* Parse `text` as a single whole int with nothing but whitespace around it. */
gboolean gnoblin_spec_parse_whole_int(const char* text, int* out);

/* Parse `text` as a whole non-negative int (rejects negatives and trailing
 * junk). For sizes that can't be < 0 — rounding, border-width, blur-radius. */
gboolean gnoblin_spec_parse_nonneg_int(const char* text, int* out);

/* Parse a 1-based workspace number into a 0-based index (rejects <= 0 and
 * trailing junk). */
gboolean gnoblin_spec_parse_workspace_index(const char* text, int* zero_based_index);

/* Parse an integer percentage, clamped to [0, 100] (rejects trailing junk). */
gboolean gnoblin_spec_parse_percent(const char* text, int* percent);

/* Parse a base-10 unsigned int from `text`, rejecting negatives, overflow, and
 * trailing junk. */
gboolean gnoblin_spec_parse_uint(const char* text, guint* out);

/* Parse one finite double at *p (skipping leading whitespace), advancing *p past
 * the number. FALSE on no number / overflow / errno. */
gboolean gnoblin_spec_parse_double_token(char** p, double* out);

/* Parse `text` as a single finite double with only whitespace around it (no
 * range — the caller clamps/range-checks the result). */
gboolean gnoblin_spec_parse_double(const char* text, double* out);

G_END_DECLS
