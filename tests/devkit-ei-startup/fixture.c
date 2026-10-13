/* SPDX-License-Identifier: GPL-2.0-or-later
 * Compile the complete pinned/patched mdk-ei.c against real libei/libeis.
 * Only MdkSeat's UI-facing wrapper is replaced with a small GObject holder.
 */
#include <errno.h>
#include <fcntl.h>
#include <gio/gio.h>
#include <libei.h>
#include <libeis.h>
#include <poll.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <unistd.h>

#include "mdk-ei.h"
#include "mdk-seat.h"

static unsigned int created, finalized, disconnects;
static guint source_id;

static void on_ei_finalized(gpointer data, GObject* object) {
    finalized++;
}

static gpointer tracked_object_new(GType type, const char* first_property_name) {
    GObject* object;

    g_assert_cmpuint(type, ==, MDK_TYPE_EI);
    g_assert_null(first_property_name);
    object = g_object_new(type, NULL);
    g_object_weak_ref(object, on_ei_finalized, NULL);
    created++;
    return object;
}

static guint tracked_source_attach(GSource* source, GMainContext* context) {
    source_id = g_source_attach(source, context);
    return source_id;
}

static struct ei_event* tracked_get_event(struct ei* ei) {
    struct ei_event* event = ei_get_event(ei);

    if (event && ei_event_get_type(event) == EI_EVENT_DISCONNECT) {
        if (disconnects++ == 0) {
            fputs("Observed real EI_EVENT_DISCONNECT\n", stderr);
            fflush(stderr);
        }
    }
    return event;
}

/* Observe ownership and events without replacing dispatch, backend setup,
 * constructor, error handling, source teardown, or the finalizer. */
#define g_object_new tracked_object_new
#define g_source_attach tracked_source_attach
#define ei_get_event tracked_get_event
#include "mdk-ei.c"
#undef ei_get_event
#undef g_source_attach
#undef g_object_new

struct _MdkSeat {
    GObject parent;
    struct ei_seat* seat;
};

G_DEFINE_TYPE(MdkSeat, mdk_seat, G_TYPE_OBJECT)

static void seat_finalize(GObject* object) {
    MdkSeat* seat = MDK_SEAT(object);

    ei_seat_unref(seat->seat);
    G_OBJECT_CLASS(mdk_seat_parent_class)->finalize(object);
}

static void mdk_seat_class_init(MdkSeatClass* klass) {
    G_OBJECT_CLASS(klass)->finalize = seat_finalize;
}

static void mdk_seat_init(MdkSeat* seat) {}

MdkSeat* mdk_seat_new(MdkEi* ei, struct ei_seat* ei_seat) {
    MdkSeat* seat = g_object_new(MDK_TYPE_SEAT, NULL);

    seat->seat = ei_seat_ref(ei_seat);
    return seat;
}

void mdk_seat_process_event(MdkSeat* seat, struct ei_event* event) {
    /* The server publishes a seat with no devices. Device/UI behavior is
     * outside the constructor-disconnect test. */
    g_assert_not_reached();
}

typedef struct {
    struct eis* eis;
    gboolean reject;
    gint stop;
} Server;

static gpointer serve(gpointer data) {
    Server* server = data;
    struct pollfd poll_fd = {.fd = eis_get_fd(server->eis), .events = POLLIN};

    while (!g_atomic_int_get(&server->stop)) {
        struct eis_event* event;
        int result = poll(&poll_fd, 1, 20);

        if (result < 0 && errno == EINTR)
            continue;
        g_assert_cmpint(result, >=, 0);
        eis_dispatch(server->eis);
        while ((event = eis_get_event(server->eis))) {
            if (eis_event_get_type(event) == EIS_EVENT_CLIENT_CONNECT) {
                struct eis_client* client = eis_event_get_client(event);

                if (server->reject)
                    eis_client_disconnect(client);
                else {
                    struct eis_seat* seat;

                    eis_client_connect(client);
                    seat = eis_client_new_seat(client, "fixture seat");
                    eis_seat_configure_capability(seat, EIS_DEVICE_CAP_POINTER);
                    eis_seat_add(seat);
                    eis_seat_unref(seat);
                }
            }
            eis_event_unref(event);
        }
    }
    return NULL;
}

static void assert_released(int fd, const struct stat* original) {
    struct stat current;

    g_assert_cmpuint(created, >, 0);
    g_assert_cmpuint(finalized, ==, created);
    if (source_id)
        g_assert_null(g_main_context_find_source_by_id(NULL, source_id));
    /* libei may close the socket before GLib creates its main-context eventfd,
     * which can reuse this number. Check socket identity, not the integer. */
    errno = 0;
    if (fstat(fd, &current) < 0)
        g_assert_cmpint(errno, ==, EBADF);
    else
        g_assert_true(current.st_dev != original->st_dev || current.st_ino != original->st_ino);
}

static void closed_peer(gboolean collect_error) {
    int pair[2];
    struct stat original;
    g_autoptr(GError) error = NULL;
    MdkEi* ei;

    g_assert_cmpint(socketpair(AF_UNIX, SOCK_STREAM, 0, pair), ==, 0);
    g_assert_cmpint(fstat(pair[0], &original), ==, 0);
    close(pair[1]);
    ei = mdk_ei_new(NULL, pair[0], collect_error ? &error : NULL);
    g_assert_null(ei);
    if (collect_error)
        g_assert_error(error, G_IO_ERROR, G_IO_ERROR_CONNECTION_CLOSED);
    g_assert_cmpuint(disconnects, >, 0);
    assert_released(pair[0], &original);
}

static void server_case(gboolean reject) {
    Server server = {.eis = eis_new(NULL), .reject = reject};
    g_autoptr(GError) error = NULL;
    GThread* thread;
    MdkEi* ei;
    int fd;
    struct stat original;

    g_assert_nonnull(server.eis);
    g_assert_cmpint(eis_setup_backend_fd(server.eis), ==, 0);
    fd = eis_backend_fd_add_client(server.eis);
    g_assert_cmpint(fd, >=, 0);
    g_assert_cmpint(fstat(fd, &original), ==, 0);
    thread = g_thread_new("fixture-eis", serve, &server);
    ei = mdk_ei_new(NULL, fd, &error);
    if (reject) {
        g_assert_null(ei);
        g_assert_error(error, G_IO_ERROR, G_IO_ERROR_CONNECTION_CLOSED);
        g_assert_cmpuint(disconnects, ==, 1);
    } else {
        g_assert_no_error(error);
        g_assert_nonnull(ei);
        g_assert_nonnull(mdk_ei_get_default_seat(ei));
        g_assert_cmpuint(finalized, ==, 0);
        g_assert_nonnull(g_main_context_find_source_by_id(NULL, source_id));
        g_object_unref(ei);
    }
    assert_released(fd, &original);
    g_atomic_int_set(&server.stop, TRUE);
    g_thread_join(thread);
    eis_unref(server.eis);
}

int main(int argc, char** argv) {
    if (argc != 2)
        return 2;
    if (strcmp(argv[1], "closed-peer") == 0)
        closed_peer(TRUE);
    else if (strcmp(argv[1], "null-error") == 0)
        closed_peer(FALSE);
    else if (strcmp(argv[1], "repeat") == 0) {
        for (int i = 0; i < 32; i++)
            closed_peer(TRUE);
    } else if (strcmp(argv[1], "rejected") == 0)
        server_case(TRUE);
    else if (strcmp(argv[1], "seat") == 0)
        server_case(FALSE);
    else
        return 2;
    printf("PASS %s: objects=%u finalized=%u; source and fd released\n", argv[1], created,
           finalized);
    return 0;
}
