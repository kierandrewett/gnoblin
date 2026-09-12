/* Expand config includes and record directories that can gain new matches. */
#include "gnoblin-config.h"

#include <fnmatch.h>
#include <string.h>

#define MAX_INCLUDE_ENTRIES 4096
#define MAX_INCLUDE_DEPTH 64

typedef struct {
    char **parts;
    GPtrArray *matches;
    GPtrArray *directories;
    guint entries;
} IncludeGlob;

static gboolean record_directory (IncludeGlob *glob, const char *path, GError **error)
{
    for (guint i = 0; i < glob->directories->len; i++) {
        if (g_str_equal (g_ptr_array_index (glob->directories, i), path))
            return TRUE;
    }
    if (++glob->entries > MAX_INCLUDE_ENTRIES) {
        g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                             "config include glob exceeds 4096 entries");
        return FALSE;
    }
    g_ptr_array_add (glob->directories, g_strdup (path));
    return TRUE;
}

static gboolean expand (IncludeGlob *glob, const char *directory, guint part,
                        guint depth, GError **error)
{
    if (depth > MAX_INCLUDE_DEPTH) {
        g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                             "config include glob exceeds 64 directories");
        return FALSE;
    }
    if (++glob->entries > MAX_INCLUDE_ENTRIES) {
        g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                             "config include glob exceeds 4096 entries");
        return FALSE;
    }
    const char *pattern = glob->parts[part];
    if (!pattern) {
        if (g_file_test (directory, G_FILE_TEST_IS_REGULAR)) {
            if (++glob->entries > MAX_INCLUDE_ENTRIES) {
                g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                                     "config include glob exceeds 4096 entries");
                return FALSE;
            }
            g_ptr_array_add (glob->matches, g_strdup (directory));
        }
        return TRUE;
    }
    if (!strpbrk (pattern, "*?[")) {
        g_autofree char *child = g_build_filename (directory, pattern, NULL);
        return expand (glob, child, part + 1, depth, error);
    }
    if (!record_directory (glob, directory, error))
        return FALSE;
    const gboolean recursive = g_str_equal (pattern, "**");
    if (recursive && !expand (glob, directory, part + 1, depth, error))
        return FALSE;
    g_autoptr(GError) local_error = NULL;
    g_autoptr(GDir) dir = g_dir_open (directory, 0, &local_error);
    if (!dir) {
        if (g_error_matches (local_error, G_FILE_ERROR, G_FILE_ERROR_NOENT) ||
            g_error_matches (local_error, G_FILE_ERROR, G_FILE_ERROR_NOTDIR))
            return TRUE;
        g_propagate_prefixed_error (error, g_steal_pointer (&local_error), "%s: ", directory);
        return FALSE;
    }
    const char *name;
    while ((name = g_dir_read_name (dir))) {
        if (fnmatch (recursive ? "*" : pattern, name, FNM_PERIOD) != 0)
            continue;
        g_autofree char *child = g_build_filename (directory, name, NULL);
        if (recursive) {
            if (!glob->parts[part + 1] && g_file_test (child, G_FILE_TEST_IS_REGULAR) &&
                !expand (glob, child, part + 1, depth + 1, error))
                return FALSE;
            if (g_file_test (child, G_FILE_TEST_IS_DIR) &&
                !g_file_test (child, G_FILE_TEST_IS_SYMLINK) &&
                !expand (glob, child, part, depth + 1, error))
                return FALSE;
        } else if (!expand (glob, child, part + 1, depth + 1, error)) {
            return FALSE;
        }
    }
    return TRUE;
}

static gint compare_paths (gconstpointer a, gconstpointer b)
{
    return strcmp (*(const char * const *) a, *(const char * const *) b);
}

GPtrArray *gnoblin_config_expand_paths (const char *including_file, const char *pattern,
                                       GPtrArray *watched_dirs, GError **error)
{
    if (!pattern || !*pattern) {
        g_set_error_literal (error, G_FILE_ERROR, G_FILE_ERROR_INVAL,
                             "config load expects a nonempty path or glob");
        return NULL;
    }
    g_autofree char *directory = g_path_get_dirname (including_file);
    g_autofree char *expanded = g_str_has_prefix (pattern, "~/")
        ? g_build_filename (g_get_home_dir (), pattern + 2, NULL) : g_strdup (pattern);
    g_autofree char *absolute = g_canonicalize_filename (expanded, directory);
    g_autoptr(GPtrArray) matches = g_ptr_array_new_with_free_func (g_free);
    if (!strpbrk (absolute, "*?[")) {
        g_ptr_array_add (matches, g_steal_pointer (&absolute));
        return g_steal_pointer (&matches);
    }
    g_auto(GStrv) parts = g_strsplit (absolute + 1, "/", -1);
    IncludeGlob glob = {.parts = parts, .matches = matches, .directories = watched_dirs};
    if (!expand (&glob, "/", 0, 0, error))
        return NULL;
    g_ptr_array_sort (matches, compare_paths);
    for (guint i = 1; i < matches->len;) {
        if (g_str_equal (g_ptr_array_index (matches, i - 1), g_ptr_array_index (matches, i)))
            g_ptr_array_remove_index (matches, i);
        else
            i++;
    }
    return g_steal_pointer (&matches);
}
