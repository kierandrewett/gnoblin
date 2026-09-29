/* Private, inherited-fd protocol between gnoblin and its compositor child. */
#pragma once

#include <gio/gio.h>

G_BEGIN_DECLS

#define GNOBLIN_RUNTIME_PROTOCOL_MAJOR 2
#define GNOBLIN_RUNTIME_PROTOCOL_MINOR 4
#define GNOBLIN_RUNTIME_PROTOCOL_MAX_PAYLOAD (16u * 1024u * 1024u)

typedef enum {
    GNOBLIN_RUNTIME_PACKET_HELLO = 1,
    GNOBLIN_RUNTIME_PACKET_CONFIG = 2,
    GNOBLIN_RUNTIME_PACKET_EVENT = 3,
    GNOBLIN_RUNTIME_PACKET_OPERATION = 4,
    GNOBLIN_RUNTIME_PACKET_COMPLETION = 5,
    GNOBLIN_RUNTIME_PACKET_SHUTDOWN = 6,
    GNOBLIN_RUNTIME_PACKET_ERROR = 7,
    GNOBLIN_RUNTIME_PACKET_API_REQUEST = 8,
    GNOBLIN_RUNTIME_PACKET_API_RESPONSE = 9,
    GNOBLIN_RUNTIME_PACKET_STATE = 10,
    /* CONFIG with a nonzero request_id receives a correlated result. */
    GNOBLIN_RUNTIME_PACKET_CONFIG_RESULT = 11,
    GNOBLIN_RUNTIME_PACKET_WORKER_DISCONNECTED = 12,
    GNOBLIN_RUNTIME_PACKET_WORKER_SUSPENDED = 13,
    GNOBLIN_RUNTIME_PACKET_WORKER_RESUME = 14,
} GnoblinRuntimePacketType;

typedef struct {
    GnoblinRuntimePacketType type;
    guint64 request_id;
    GVariant* payload;
} GnoblinRuntimePacket;

typedef struct _GnoRuntimeWriter GnoblinRuntimeWriter;
typedef struct _GnoRuntimeReader GnoblinRuntimeReader;

GnoblinRuntimeWriter* gnoblin_runtime_writer_new(void);
void gnoblin_runtime_writer_free(GnoblinRuntimeWriter* writer);
/* Queue owns a reference to payload. Queue is bounded. */
gboolean gnoblin_runtime_writer_queue(GnoblinRuntimeWriter* writer, GnoblinRuntimePacketType type,
                                      guint64 request_id, GVariant* payload, GError** error);
/* Sends with MSG_DONTWAIT, including when fd itself is blocking. Returns TRUE
 * when drained; FALSE with WOULD_BLOCK means watch G_IO_OUT and retry. This API
 * never waits and has no GCancellable because callers own readiness scheduling. */
gboolean gnoblin_runtime_writer_flush(GnoblinRuntimeWriter* writer, int fd, GError** error);
/* Discard queued output after the peer discards the corresponding stream. */
void gnoblin_runtime_writer_reset(GnoblinRuntimeWriter* writer);

GnoblinRuntimeReader* gnoblin_runtime_reader_new(void);
void gnoblin_runtime_reader_free(GnoblinRuntimeReader* reader);
/* Receives with MSG_DONTWAIT, including when fd itself is blocking.
 * available=FALSE means no complete packet yet (including EAGAIN). This API
 * never waits and has no GCancellable because callers own readiness scheduling. */
gboolean gnoblin_runtime_reader_receive(GnoblinRuntimeReader* reader, int fd,
                                        GnoblinRuntimePacket* packet, gboolean* available,
                                        GError** error);
/* Discard an incomplete fragmented message. */
void gnoblin_runtime_reader_reset(GnoblinRuntimeReader* reader);
void gnoblin_runtime_packet_clear(GnoblinRuntimePacket* packet);
G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinRuntimeWriter, gnoblin_runtime_writer_free)
G_DEFINE_AUTOPTR_CLEANUP_FUNC(GnoblinRuntimeReader, gnoblin_runtime_reader_free)

G_END_DECLS
