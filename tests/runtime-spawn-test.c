#define _GNU_SOURCE
#include "../src/session/gnoblin-runtime-spawn.h"

#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

#define CHILD_CHANNEL_FD 198
#define CHILD_HOST_FD 200

extern char** environ;
static char* test_program;

static int child_mode(void) {
    char request = 0;
    if (recv(CHILD_CHANNEL_FD, &request, 1, 0) != 1 || request != 'Q')
        return 10;
    char response = 'A';
    if (send(CHILD_CHANNEL_FD, &response, 1, MSG_NOSIGNAL) != 1)
        return 11;
    return 0;
}

static int child_isolated_mode(void) {
    char compositor_message = 0;
    char host_message = 0;
    if (recv(CHILD_CHANNEL_FD, &compositor_message, 1, 0) != 1 || compositor_message != 'M' ||
        recv(CHILD_HOST_FD, &host_message, 1, 0) != 1 || host_message != 'H')
        return 12;
    char compositor_reply = 'm';
    char host_reply = 'h';
    if (send(CHILD_CHANNEL_FD, &compositor_reply, 1, MSG_NOSIGNAL) != 1 ||
        send(CHILD_HOST_FD, &host_reply, 1, MSG_NOSIGNAL) != 1)
        return 13;
    return 0;
}

static void run_target_collision(gboolean parent_collision, gboolean child_collision) {
    int sockets[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets), ==, 0);

    int saved_target = fcntl(CHILD_CHANNEL_FD, F_DUPFD_CLOEXEC, CHILD_CHANNEL_FD + 1);
    g_assert_true(saved_target >= 0 || errno == EBADF);
    if (saved_target < 0)
        saved_target = -1;

    int parent_fd = sockets[0];
    int child_fd = sockets[1];
    if (parent_collision) {
        g_assert_cmpint(dup2(parent_fd, CHILD_CHANNEL_FD), ==, CHILD_CHANNEL_FD);
        close(parent_fd);
        parent_fd = CHILD_CHANNEL_FD;
    }
    if (child_collision) {
        g_assert_cmpint(dup2(child_fd, CHILD_CHANNEL_FD), ==, CHILD_CHANNEL_FD);
        close(child_fd);
        child_fd = CHILD_CHANNEL_FD;
    }

    posix_spawn_file_actions_t actions;
    g_assert_cmpint(posix_spawn_file_actions_init(&actions), ==, 0);
    int temporary_fd = -1;
    g_autoptr(GError) error = NULL;
    g_assert_true(gnoblin_runtime_spawn_add_channel_actions(
        &actions, parent_fd, child_fd, CHILD_CHANNEL_FD, &temporary_fd, &error));
    g_assert_no_error(error);

    char* child_argv[] = {test_program, "--child", NULL};
    pid_t child = 0;
    g_assert_cmpint(posix_spawn(&child, test_program, &actions, NULL, child_argv, environ), ==, 0);
    posix_spawn_file_actions_destroy(&actions);
    if (temporary_fd >= 0)
        close(temporary_fd);
    if (child_fd != CHILD_CHANNEL_FD)
        close(child_fd);

    char request = 'Q';
    g_assert_cmpint(send(parent_fd, &request, 1, MSG_NOSIGNAL), ==, 1);
    char response = 0;
    g_assert_cmpint(recv(parent_fd, &response, 1, 0), ==, 1);
    g_assert_cmpint(response, ==, 'A');

    int status = 0;
    g_assert_cmpint(waitpid(child, &status, 0), ==, child);
    g_assert_true(WIFEXITED(status));
    g_assert_cmpint(WEXITSTATUS(status), ==, 0);
    close(parent_fd);
    if (saved_target >= 0) {
        g_assert_cmpint(dup2(saved_target, CHILD_CHANNEL_FD), ==, CHILD_CHANNEL_FD);
        close(saved_target);
    }
}

static void test_parent_target_collision(void) {
    run_target_collision(TRUE, FALSE);
}

static void test_child_target_collision(void) {
    run_target_collision(FALSE, TRUE);
}

static void test_supervisor_channel_is_separate_from_compositor_channel(void) {
    int compositor[2];
    int supervisor[2];
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, compositor), ==, 0);
    g_assert_cmpint(socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, supervisor), ==, 0);
    int saved_compositor = fcntl(CHILD_CHANNEL_FD, F_DUPFD_CLOEXEC, CHILD_HOST_FD + 1);
    int saved_host = fcntl(CHILD_HOST_FD, F_DUPFD_CLOEXEC, CHILD_HOST_FD + 1);
    if (saved_compositor < 0)
        g_assert_cmpint(errno, ==, EBADF);
    if (saved_host < 0)
        g_assert_cmpint(errno, ==, EBADF);

    int compositor_parent = fcntl(compositor[0], F_DUPFD_CLOEXEC, CHILD_HOST_FD + 1);
    int compositor_child = fcntl(compositor[1], F_DUPFD_CLOEXEC, CHILD_HOST_FD + 1);
    int supervisor_parent = fcntl(supervisor[0], F_DUPFD_CLOEXEC, CHILD_HOST_FD + 1);
    int supervisor_child = fcntl(supervisor[1], F_DUPFD_CLOEXEC, CHILD_HOST_FD + 1);
    g_assert_cmpint(compositor_parent, >=, 0);
    g_assert_cmpint(compositor_child, >=, 0);
    g_assert_cmpint(supervisor_parent, >=, 0);
    g_assert_cmpint(supervisor_child, >=, 0);

    posix_spawn_file_actions_t actions;
    g_assert_cmpint(posix_spawn_file_actions_init(&actions), ==, 0);
    int temporary_compositor = -1;
    int temporary_supervisor = -1;
    g_autoptr(GError) error = NULL;
    g_assert_true(gnoblin_runtime_spawn_add_channel_actions(&actions, compositor_parent,
                                                            compositor_child, CHILD_CHANNEL_FD,
                                                            &temporary_compositor, &error));
    g_assert_no_error(error);
    g_assert_true(gnoblin_runtime_spawn_add_channel_actions(&actions, supervisor_parent,
                                                            supervisor_child, CHILD_HOST_FD,
                                                            &temporary_supervisor, &error));
    g_assert_no_error(error);
    char* child_argv[] = {test_program, "--child-isolated", NULL};
    pid_t child = 0;
    g_assert_cmpint(posix_spawn(&child, test_program, &actions, NULL, child_argv, environ), ==, 0);
    posix_spawn_file_actions_destroy(&actions);
    if (temporary_compositor >= 0)
        close(temporary_compositor);
    if (temporary_supervisor >= 0)
        close(temporary_supervisor);
    close(compositor_parent);
    close(compositor_child);
    close(supervisor_parent);
    close(supervisor_child);
    close(compositor[1]);
    close(supervisor[1]);

    char compositor_message = 'M';
    char host_message = 'H';
    g_assert_cmpint(send(compositor[0], &compositor_message, 1, MSG_NOSIGNAL), ==, 1);
    g_assert_cmpint(send(supervisor[0], &host_message, 1, MSG_NOSIGNAL), ==, 1);
    char compositor_reply = 0;
    char host_reply = 0;
    g_assert_cmpint(recv(compositor[0], &compositor_reply, 1, 0), ==, 1);
    g_assert_cmpint(recv(supervisor[0], &host_reply, 1, 0), ==, 1);
    g_assert_cmpint(compositor_reply, ==, 'm');
    g_assert_cmpint(host_reply, ==, 'h');

    int status = 0;
    g_assert_cmpint(waitpid(child, &status, 0), ==, child);
    g_assert_true(WIFEXITED(status));
    g_assert_cmpint(WEXITSTATUS(status), ==, 0);
    close(compositor[0]);
    close(supervisor[0]);
    if (saved_compositor >= 0) {
        g_assert_cmpint(dup2(saved_compositor, CHILD_CHANNEL_FD), ==, CHILD_CHANNEL_FD);
        close(saved_compositor);
    }
    if (saved_host >= 0) {
        g_assert_cmpint(dup2(saved_host, CHILD_HOST_FD), ==, CHILD_HOST_FD);
        close(saved_host);
    }
}

int main(int argc, char** argv) {
    if (argc == 2 && strcmp(argv[1], "--child") == 0)
        return child_mode();
    if (argc == 2 && strcmp(argv[1], "--child-isolated") == 0)
        return child_isolated_mode();
    test_program = realpath(argv[0], NULL);
    g_assert_nonnull(test_program);
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/runtime-spawn/parent-target-collision", test_parent_target_collision);
    g_test_add_func("/runtime-spawn/child-target-collision", test_child_target_collision);
    g_test_add_func("/runtime-spawn/supervisor-channel-separate",
                    test_supervisor_channel_is_separate_from_compositor_channel);
    int result = g_test_run();
    free(test_program);
    return result;
}
