#define _GNU_SOURCE
#include "gnoblin-runtime-spawn.h"

#include <errno.h>
#include <fcntl.h>
#include <unistd.h>

gboolean gnoblin_runtime_spawn_add_channel_actions(posix_spawn_file_actions_t* actions,
                                                   int parent_fd, int child_fd, int target_fd,
                                                   int* temporary_fd, GError** error) {
    g_return_val_if_fail(actions != NULL, FALSE);
    g_return_val_if_fail(temporary_fd != NULL, FALSE);
    g_return_val_if_fail(error == NULL || *error == NULL, FALSE);
    *temporary_fd = -1;
    if (parent_fd < 0 || child_fd < 0 || target_fd < 0 || parent_fd == child_fd) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid private runtime channel descriptors");
        return FALSE;
    }

    int source_fd = child_fd;
    int duplicate_fd = -1;
    if (child_fd == target_fd) {
        duplicate_fd = fcntl(child_fd, F_DUPFD_CLOEXEC, target_fd + 1);
        if (duplicate_fd < 0) {
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "could not move the child runtime channel: %s", g_strerror(errno));
            return FALSE;
        }
        source_fd = duplicate_fd;
    }

    /* Close first: parent_fd itself can be the descriptor number reserved for
     * the child endpoint. The following dup2 must then repopulate that slot. */
    int result = posix_spawn_file_actions_addclose(actions, parent_fd);
    if (result == 0)
        result = posix_spawn_file_actions_adddup2(actions, source_fd, target_fd);
    if (result == 0 && child_fd != target_fd)
        result = posix_spawn_file_actions_addclose(actions, child_fd);
    if (result == 0 && duplicate_fd >= 0)
        result = posix_spawn_file_actions_addclose(actions, duplicate_fd);
    if (result != 0) {
        if (duplicate_fd >= 0)
            close(duplicate_fd);
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(result),
                    "could not prepare private runtime descriptor actions: %s", g_strerror(result));
        return FALSE;
    }
    *temporary_fd = duplicate_fd;
    return TRUE;
}
