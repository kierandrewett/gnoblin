#include "gnoblin-runtime-protocol.h"

#include <errno.h>
#include <string.h>
#include <sys/socket.h>

#define FRAME_HEADER_SIZE 32u
#define FRAME_CHUNK_SIZE (60u * 1024u)
#define WRITER_MAX_QUEUED (32u * 1024u * 1024u)
#define FRAME_FIRST 1u
#define FRAME_LAST 2u

typedef struct {
    GnoblinRuntimePacketType type;
    guint64 request_id;
    GBytes* payload;
    gsize offset;
} OutMessage;

struct _GnoRuntimeWriter {
    GQueue queue;
    gsize queued_bytes;
    gsize chunk_size;
};

struct _GnoRuntimeReader {
    GByteArray* message;
    guint16 type;
    guint64 request_id;
    guint32 total_size;
    guint32 next_offset;
    gboolean active;
};

static const guint8 frame_magic[4] = {'G', 'N', 'R', 'T'};

static guint16 read_u16(const guint8* d) {
    return ((guint16)d[0] << 8) | d[1];
}
static guint32 read_u32(const guint8* d) {
    return ((guint32)d[0] << 24) | ((guint32)d[1] << 16) | ((guint32)d[2] << 8) | d[3];
}
static guint64 read_u64(const guint8* d) {
    return ((guint64)read_u32(d) << 32) | read_u32(d + 4);
}
static void write_u16(guint8* d, guint16 v) {
    d[0] = v >> 8;
    d[1] = v;
}
static void write_u32(guint8* d, guint32 v) {
    d[0] = v >> 24;
    d[1] = v >> 16;
    d[2] = v >> 8;
    d[3] = v;
}
static void write_u64(guint8* d, guint64 v) {
    write_u32(d, v >> 32);
    write_u32(d + 4, v);
}
static gboolean packet_type_valid(guint16 type) {
    return type >= GNOBLIN_RUNTIME_PACKET_HELLO &&
           type <= GNOBLIN_RUNTIME_PACKET_INPUT_DECISION;
}
static void out_message_free(OutMessage* message) {
    if (!message)
        return;
    g_bytes_unref(message->payload);
    g_free(message);
}

GnoblinRuntimeWriter* gnoblin_runtime_writer_new(void) {
    GnoblinRuntimeWriter* writer = g_new0(GnoblinRuntimeWriter, 1);
    g_queue_init(&writer->queue);
    writer->chunk_size = FRAME_CHUNK_SIZE;
    return writer;
}

void gnoblin_runtime_writer_free(GnoblinRuntimeWriter* writer) {
    if (!writer)
        return;
    g_queue_clear_full(&writer->queue, (GDestroyNotify)out_message_free);
    g_free(writer);
}

void gnoblin_runtime_writer_reset(GnoblinRuntimeWriter* writer) {
    if (!writer)
        return;
    g_queue_clear_full(&writer->queue, (GDestroyNotify)out_message_free);
    writer->queued_bytes = 0;
    writer->chunk_size = FRAME_CHUNK_SIZE;
}

gboolean gnoblin_runtime_writer_queue(GnoblinRuntimeWriter* writer, GnoblinRuntimePacketType type,
                                      guint64 request_id, GVariant* payload, GError** error) {
    g_return_val_if_fail(error == NULL || *error == NULL, FALSE);
    if (!writer || !packet_type_valid(type) || !payload ||
        !g_variant_is_of_type(payload, G_VARIANT_TYPE_VARDICT) ||
        !g_variant_is_normal_form(payload)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid Gnoblin runtime packet");
        return FALSE;
    }
    gsize size = g_variant_get_size(payload);
    gsize queued_size = size + FRAME_HEADER_SIZE;
    if (size > GNOBLIN_RUNTIME_PROTOCOL_MAX_PAYLOAD ||
        queued_size > WRITER_MAX_QUEUED - writer->queued_bytes) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NO_SPACE,
                            "Gnoblin runtime output queue limit exceeded");
        return FALSE;
    }
    OutMessage* message = g_new0(OutMessage, 1);
    message->type = type;
    message->request_id = request_id;
    message->payload = g_bytes_new(g_variant_get_data(payload), size);
    g_queue_push_tail(&writer->queue, message);
    writer->queued_bytes += queued_size;
    return TRUE;
}

gboolean gnoblin_runtime_writer_flush(GnoblinRuntimeWriter* writer, int fd, GError** error) {
    g_return_val_if_fail(error == NULL || *error == NULL, FALSE);
    if (!writer || fd < 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_ARGUMENT,
                            "invalid Gnoblin runtime writer");
        return FALSE;
    }
    while (!g_queue_is_empty(&writer->queue)) {
        OutMessage* m = g_queue_peek_head(&writer->queue);
        gsize size;
        const guint8* data = g_bytes_get_data(m->payload, &size);
        gsize remaining = size - m->offset;
        gsize chunk = MIN(remaining, writer->chunk_size);
        guint16 flags =
            (m->offset == 0 ? FRAME_FIRST : 0) | (m->offset + chunk == size ? FRAME_LAST : 0);
        g_autofree guint8* frame = g_malloc(FRAME_HEADER_SIZE + chunk);
        memcpy(frame, frame_magic, 4);
        write_u16(frame + 4, GNOBLIN_RUNTIME_PROTOCOL_MAJOR);
        write_u16(frame + 6, GNOBLIN_RUNTIME_PROTOCOL_MINOR);
        write_u16(frame + 8, m->type);
        write_u16(frame + 10, flags);
        write_u32(frame + 12, (guint32)chunk);
        write_u64(frame + 16, m->request_id);
        write_u32(frame + 24, (guint32)size);
        write_u32(frame + 28, (guint32)m->offset);
        if (chunk)
            memcpy(frame + FRAME_HEADER_SIZE, data + m->offset, chunk);
        ssize_t sent;
        do {
            sent = send(fd, frame, FRAME_HEADER_SIZE + chunk, MSG_NOSIGNAL | MSG_DONTWAIT);
        } while (sent < 0 && errno == EINTR);
        if (sent < 0) {
            if (errno == EMSGSIZE && writer->chunk_size > 1024) {
                writer->chunk_size = MAX((gsize)1024, writer->chunk_size / 2);
                continue;
            }
            g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                        "could not send Gnoblin runtime packet: %s", g_strerror(errno));
            return FALSE;
        }
        if ((gsize)sent != FRAME_HEADER_SIZE + chunk) {
            g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_PARTIAL_INPUT,
                                "Gnoblin runtime record was not sent atomically");
            return FALSE;
        }
        m->offset += chunk;
        if (flags & FRAME_LAST) {
            writer->queued_bytes -= size + FRAME_HEADER_SIZE;
            g_queue_pop_head(&writer->queue);
            out_message_free(m);
        }
    }
    return TRUE;
}

GnoblinRuntimeReader* gnoblin_runtime_reader_new(void) {
    GnoblinRuntimeReader* reader = g_new0(GnoblinRuntimeReader, 1);
    reader->message = g_byte_array_new();
    return reader;
}

void gnoblin_runtime_reader_free(GnoblinRuntimeReader* reader) {
    if (!reader)
        return;
    g_byte_array_unref(reader->message);
    g_free(reader);
}

void gnoblin_runtime_reader_reset(GnoblinRuntimeReader* reader) {
    if (!reader)
        return;
    g_byte_array_set_size(reader->message, 0);
    reader->type = 0;
    reader->request_id = 0;
    reader->total_size = 0;
    reader->next_offset = 0;
    reader->active = FALSE;
}

void gnoblin_runtime_packet_clear(GnoblinRuntimePacket* packet) {
    if (!packet)
        return;
    g_clear_pointer(&packet->payload, g_variant_unref);
    packet->request_id = 0;
    packet->type = 0;
}

gboolean gnoblin_runtime_reader_receive(GnoblinRuntimeReader* reader, int fd,
                                        GnoblinRuntimePacket* packet, gboolean* available,
                                        GError** error) {
    g_return_val_if_fail(reader && packet && available, FALSE);
    g_return_val_if_fail(error == NULL || *error == NULL, FALSE);
    memset(packet, 0, sizeof *packet);
    *available = FALSE;
    guint8 frame[FRAME_HEADER_SIZE + FRAME_CHUNK_SIZE];
    struct iovec iov = {.iov_base = frame, .iov_len = sizeof frame};
    struct msghdr msg = {.msg_iov = &iov, .msg_iovlen = 1};
    ssize_t received;
    do {
        received = recvmsg(fd, &msg, MSG_DONTWAIT);
    } while (received < 0 && errno == EINTR);
    if (received < 0) {
        if (errno == EAGAIN)
            return TRUE;
#if EWOULDBLOCK != EAGAIN
        if (errno == EWOULDBLOCK)
            return TRUE;
#endif
        g_set_error(error, G_IO_ERROR, g_io_error_from_errno(errno),
                    "could not receive Gnoblin runtime packet: %s", g_strerror(errno));
        return FALSE;
    }
    if (received == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_CLOSED,
                            "Gnoblin runtime connection closed");
        return FALSE;
    }
    if ((msg.msg_flags & MSG_TRUNC) || received < FRAME_HEADER_SIZE ||
        memcmp(frame, frame_magic, 4)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "invalid Gnoblin runtime record header");
        return FALSE;
    }
    if (read_u16(frame + 4) != GNOBLIN_RUNTIME_PROTOCOL_MAJOR ||
        read_u16(frame + 6) > GNOBLIN_RUNTIME_PROTOCOL_MINOR) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_NOT_SUPPORTED,
                            "unsupported Gnoblin runtime protocol version");
        return FALSE;
    }
    guint16 type = read_u16(frame + 8), flags = read_u16(frame + 10);
    guint32 chunk = read_u32(frame + 12), total = read_u32(frame + 24),
            offset = read_u32(frame + 28);
    guint64 id = read_u64(frame + 16);
    if (!packet_type_valid(type) || flags & ~(FRAME_FIRST | FRAME_LAST) ||
        chunk > FRAME_CHUNK_SIZE || total > GNOBLIN_RUNTIME_PROTOCOL_MAX_PAYLOAD ||
        (gsize)received != FRAME_HEADER_SIZE + chunk || offset > total || chunk > total - offset ||
        ((flags & FRAME_FIRST) != (offset == 0 ? FRAME_FIRST : 0))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "invalid Gnoblin runtime record framing");
        return FALSE;
    }
    if (flags & FRAME_FIRST) {
        if (reader->active) {
            /* A supervisor recovery control packet may follow a worker that
             * died mid-message. It is the explicit stream resynchronization
             * boundary; ordinary packets remain strict. */
            if (type == GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED ||
                type == GNOBLIN_RUNTIME_PACKET_WORKER_SUSPENDED ||
                type == GNOBLIN_RUNTIME_PACKET_WORKER_RESUME ||
                type == GNOBLIN_RUNTIME_PACKET_RECOVERY_FAILED)
                gnoblin_runtime_reader_reset(reader);
            else {
                g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                                    "interleaved Gnoblin runtime messages");
                return FALSE;
            }
        }
        reader->active = TRUE;
        reader->type = type;
        reader->request_id = id;
        reader->total_size = total;
        reader->next_offset = 0;
        g_byte_array_set_size(reader->message, 0);
    }
    if (!reader->active || type != reader->type || id != reader->request_id ||
        total != reader->total_size || offset != reader->next_offset ||
        ((flags & FRAME_LAST) != (offset + chunk == total ? FRAME_LAST : 0))) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "out-of-order Gnoblin runtime fragments");
        return FALSE;
    }
    if (chunk)
        g_byte_array_append(reader->message, frame + FRAME_HEADER_SIZE, chunk);
    reader->next_offset += chunk;
    if (!(flags & FRAME_LAST))
        return TRUE;

    GBytes* bytes = g_bytes_new(reader->message->data, reader->message->len);
    GVariant* payload =
        g_variant_ref_sink(g_variant_new_from_bytes(G_VARIANT_TYPE_VARDICT, bytes, FALSE));
    g_bytes_unref(bytes);
    if (!g_variant_is_normal_form(payload)) {
        g_variant_unref(payload);
        reader->active = FALSE;
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_INVALID_DATA,
                            "Gnoblin runtime payload is not a normal-form dictionary");
        return FALSE;
    }
    packet->type = reader->type;
    packet->request_id = reader->request_id;
    packet->payload = payload;
    reader->active = FALSE;
    *available = TRUE;
    return TRUE;
}
