/*
 * gnoblin-shell: shared cursor tokenizer for the strict [section]-spec parsers.
 *
 * Every gnoblin-*-spec.c parser walks a config-string with the same primitives
 * (skip whitespace, parse a base-10 int, check for trailing junk). This is the
 * single copy they share, rather than re-declaring identical statics in each.
 */
#pragma once

#include <glib.h>

/* Advance *p past ASCII whitespace. */
void gnoblin_spec_skip_spaces(char** p);

/* TRUE if only whitespace (or nothing) remains from p onward. */
gboolean gnoblin_spec_at_end(char* p);

/* Parse a base-10 int at *p (skipping leading whitespace), advancing *p past the
 * digits. FALSE on no digits / overflow / errno. */
gboolean gnoblin_spec_parse_int(char** p, int* out);

/* Parse `text` as a single whole int with nothing but whitespace around it. */
gboolean gnoblin_spec_parse_whole_int(const char* text, int* out);
