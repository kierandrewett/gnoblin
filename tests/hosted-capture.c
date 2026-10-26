/* Consume one PipeWire video node, the way a session manager consumes a ScreenCast stream.
 *
 * usage: hosted-capture WIDTH HEIGHT
 *
 * The tool negotiates BGRx at the given size without a DMA-BUF modifier, so the producer sends
 * memory-mapped buffers that the tool can read. It talks over stdin and stdout:
 *   stdout  "NODE <id>"            once the consumer node exists (link its input port to the
 * producer) "STREAMING <w> <h>"    once the producer accepted the format "FRAME <count>"        at
 * most every 100 ms while frames arrive "DUMPED <path> <count>" after a dump request stdin   "dump
 * PATH"            write the newest frame as a raw BGRx file, preceded by no header "quit" exit The
 * raw file has WIDTH*HEIGHT*4 bytes with a row stride of WIDTH*4. */
#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include <pipewire/pipewire.h>
#include <spa/param/video/format-utils.h>
#include <spa/param/video/type-info.h>
#include <spa/pod/builder.h>

struct capture {
    struct pw_main_loop* loop;
    struct pw_stream* stream;
    struct spa_hook listener;
    uint32_t width, height;
    int announced;
    uint8_t* frame;
    uint64_t count;
    uint64_t last_report_ms;
    char input[512];
    size_t input_len;
};

static uint64_t now_ms(void) {
    struct timespec ts;

    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
}

static void on_state_changed(void* data, enum pw_stream_state old, enum pw_stream_state state,
                             const char* error) {
    struct capture* c = data;

    (void)old;
    if (state == PW_STREAM_STATE_ERROR) {
        fprintf(stderr, "stream error: %s\n", error ? error : "unknown");
        pw_main_loop_quit(c->loop);
    } else if ((state == PW_STREAM_STATE_CONNECTING || state == PW_STREAM_STATE_PAUSED) &&
               !c->announced && pw_stream_get_node_id(c->stream) != SPA_ID_INVALID) {
        c->announced = 1;
        printf("NODE %u\n", pw_stream_get_node_id(c->stream));
        fflush(stdout);
    }
}

static void on_param_changed(void* data, uint32_t id, const struct spa_pod* param) {
    struct capture* c = data;
    struct spa_video_info_raw info;
    uint8_t buffer[512];
    struct spa_pod_builder b = SPA_POD_BUILDER_INIT(buffer, sizeof buffer);
    const struct spa_pod* params[1];

    if (!param || id != SPA_PARAM_Format)
        return;
    if (spa_format_video_raw_parse(param, &info) < 0)
        return;
    if (info.size.width != c->width || info.size.height != c->height) {
        fprintf(stderr, "producer chose %ux%u, wanted %ux%u\n", info.size.width, info.size.height,
                c->width, c->height);
        pw_main_loop_quit(c->loop);
        return;
    }
    /* Memory-mapped buffers only: ask for MemFd and MemPtr, never DMA-BUF. */
    params[0] = spa_pod_builder_add_object(
        &b, SPA_TYPE_OBJECT_ParamBuffers, SPA_PARAM_Buffers, SPA_PARAM_BUFFERS_buffers,
        SPA_POD_CHOICE_RANGE_Int(4, 2, 16), SPA_PARAM_BUFFERS_blocks, SPA_POD_Int(1),
        SPA_PARAM_BUFFERS_dataType,
        SPA_POD_CHOICE_FLAGS_Int((1 << SPA_DATA_MemFd) | (1 << SPA_DATA_MemPtr)));
    pw_stream_update_params(c->stream, params, 1);
    printf("STREAMING %u %u\n", info.size.width, info.size.height);
    fflush(stdout);
}

static void on_process(void* data) {
    struct capture* c = data;
    struct pw_buffer* pwb = pw_stream_dequeue_buffer(c->stream);

    if (!pwb)
        return;
    struct spa_buffer* buf = pwb->buffer;
    struct spa_data* d = &buf->datas[0];

    if (d->data && d->chunk->size > 0 && !(d->chunk->flags & SPA_CHUNK_FLAG_CORRUPTED)) {
        uint32_t stride = d->chunk->stride ? (uint32_t)d->chunk->stride : c->width * 4;

        for (uint32_t row = 0; row < c->height; row++)
            memcpy(c->frame + (size_t)row * c->width * 4,
                   (uint8_t*)d->data + d->chunk->offset + (size_t)row * stride,
                   (size_t)c->width * 4);
        c->count++;
        uint64_t now = now_ms();
        if (now - c->last_report_ms >= 100) {
            c->last_report_ms = now;
            printf("FRAME %llu\n", (unsigned long long)c->count);
            fflush(stdout);
        }
    }
    pw_stream_queue_buffer(c->stream, pwb);
}

static const struct pw_stream_events stream_events = {
    PW_VERSION_STREAM_EVENTS,
    .state_changed = on_state_changed,
    .param_changed = on_param_changed,
    .process = on_process,
};

static void handle_line(struct capture* c, char* line) {
    if (strncmp(line, "dump ", 5) == 0) {
        FILE* f = fopen(line + 5, "wb");

        if (f) {
            fwrite(c->frame, 1, (size_t)c->width * c->height * 4, f);
            fclose(f);
            printf("DUMPED %s %llu\n", line + 5, (unsigned long long)c->count);
        } else {
            printf("DUMPFAILED %s\n", line + 5);
        }
        fflush(stdout);
    } else if (strcmp(line, "quit") == 0) {
        pw_main_loop_quit(c->loop);
    }
}

static void on_stdin(void* data, int fd, uint32_t mask) {
    struct capture* c = data;
    ssize_t n;

    if (mask & (SPA_IO_HUP | SPA_IO_ERR)) {
        pw_main_loop_quit(c->loop);
        return;
    }
    n = read(fd, c->input + c->input_len, sizeof c->input - c->input_len - 1);
    if (n <= 0) {
        pw_main_loop_quit(c->loop);
        return;
    }
    c->input_len += (size_t)n;
    c->input[c->input_len] = '\0';
    char* newline;
    while ((newline = strchr(c->input, '\n'))) {
        *newline = '\0';
        handle_line(c, c->input);
        c->input_len -= (size_t)(newline + 1 - c->input);
        memmove(c->input, newline + 1, c->input_len + 1);
    }
}

int main(int argc, char** argv) {
    struct capture c = {0};
    uint8_t buffer[1024];
    struct spa_pod_builder b = SPA_POD_BUILDER_INIT(buffer, sizeof buffer);
    const struct spa_pod* params[1];

    if (argc != 3) {
        fprintf(stderr, "usage: %s WIDTH HEIGHT\n", argv[0]);
        return 2;
    }
    pw_init(&argc, &argv);
    c.width = (uint32_t)atoi(argv[1]);
    c.height = (uint32_t)atoi(argv[2]);
    c.frame = calloc((size_t)c.width * c.height, 4);
    c.loop = pw_main_loop_new(NULL);
    c.stream =
        pw_stream_new_simple(pw_main_loop_get_loop(c.loop), "hosted-capture",
                             pw_properties_new(PW_KEY_MEDIA_TYPE, "Video", PW_KEY_MEDIA_CATEGORY,
                                               "Capture", PW_KEY_MEDIA_ROLE, "Screen", NULL),
                             &stream_events, &c);
    params[0] = spa_pod_builder_add_object(
        &b, SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat, SPA_FORMAT_mediaType,
        SPA_POD_Id(SPA_MEDIA_TYPE_video), SPA_FORMAT_mediaSubtype,
        SPA_POD_Id(SPA_MEDIA_SUBTYPE_raw), SPA_FORMAT_VIDEO_format,
        SPA_POD_CHOICE_ENUM_Id(2, SPA_VIDEO_FORMAT_BGRx, SPA_VIDEO_FORMAT_BGRx),
        SPA_FORMAT_VIDEO_size, SPA_POD_Rectangle(&SPA_RECTANGLE(c.width, c.height)),
        SPA_FORMAT_VIDEO_framerate, SPA_POD_Fraction(&SPA_FRACTION(0, 1)),
        SPA_FORMAT_VIDEO_maxFramerate,
        SPA_POD_CHOICE_RANGE_Fraction(&SPA_FRACTION(60, 1), &SPA_FRACTION(1, 1),
                                      &SPA_FRACTION(1000, 1)));
    /* Without a session manager nothing links the nodes. The caller links the ports itself
     * after it reads NODE. */
    if (pw_stream_connect(c.stream, PW_DIRECTION_INPUT, PW_ID_ANY,
                          PW_STREAM_FLAG_MAP_BUFFERS | PW_STREAM_FLAG_AUTOCONNECT, params, 1) < 0) {
        fprintf(stderr, "cannot connect the stream\n");
        return 1;
    }
    pw_loop_add_io(pw_main_loop_get_loop(c.loop), STDIN_FILENO, SPA_IO_IN | SPA_IO_HUP, false,
                   on_stdin, &c);
    pw_main_loop_run(c.loop);
    pw_stream_destroy(c.stream);
    pw_main_loop_destroy(c.loop);
    pw_deinit();
    return 0;
}
