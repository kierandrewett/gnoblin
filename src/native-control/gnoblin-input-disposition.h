/* Physical input-stream disposition lifecycle. */
#pragma once

#include <glib.h>

G_BEGIN_DECLS

/*
 * A fresh physical stream makes an older disposition for the same device and
 * control stale. Returns whether one was removed. Repeated key presses belong
 * to the existing stream and must retain its disposition.
 */
gboolean gnoblin_input_disposition_discard_stale(GHashTable* dispositions,
                                                  const char* stream_key,
                                                  gboolean stream_begin,
                                                  gboolean repeated);

G_END_DECLS
