#include "gnoblin-input-disposition.h"

gboolean gnoblin_input_disposition_discard_stale(GHashTable* dispositions,
                                                  const char* stream_key,
                                                  gboolean stream_begin,
                                                  gboolean repeated) {
    if (!dispositions || !stream_key || !stream_begin || repeated)
        return FALSE;
    return g_hash_table_remove(dispositions, stream_key);
}
