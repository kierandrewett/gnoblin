#pragma once

#include <gio/gio.h>
#include <spawn.h>

/* Prepare actions that leave child_fd as target_fd in the child, close the
 * supervisor endpoint, and handle the case where parent_fd equals target_fd. */
gboolean gnoblin_runtime_spawn_add_channel_actions(posix_spawn_file_actions_t* actions,
                                                   int parent_fd, int child_fd, int target_fd,
                                                   int* temporary_fd, GError** error);
