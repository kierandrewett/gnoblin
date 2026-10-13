/* SPDX-License-Identifier: LGPL-2.1-or-later */
/* Real PipeWire buffer reader; no GUI, compositor or PipeWire daemon. */
#include "gnoblin-screenshot-capture.c"
#include <glib/gstdio.h>

static void pixels(void) {
    /* Two BGRx rows, nonzero chunk offset, padded stride; producer can reuse
     * its buffer immediately after dequeue returns. */
    uint8_t bytes[32] = {0, 0, 0, 0, 3, 2, 1, 99, 6,  5,  4,  99,
                         0, 0, 0, 0, 9, 8, 7, 99, 12, 11, 10, 99};
    struct spa_chunk chunk = {.offset = 4, .size = 20, .stride = 12};
    struct spa_data data = {
        .type = SPA_DATA_MemPtr, .data = bytes, .maxsize = sizeof bytes, .chunk = &chunk};
    struct spa_buffer buffer = {.n_datas = 1, .datas = &data};
    struct spa_video_info_raw info = {.format = SPA_VIDEO_FORMAT_BGRx, .size = SPA_RECTANGLE(2, 2)};
    g_autoptr(GError) error = NULL;
    g_autoptr(GdkPixbuf) image = copy_frame(&buffer, &info, &error);
    g_assert_no_error(error);
    g_assert_nonnull(image);
    memset(bytes, 0, sizeof bytes);
    const uint8_t expected[] = {1, 2, 3, 4, 5, 6};
    g_assert_cmpmem(gdk_pixbuf_get_pixels(image), 6, expected, 6);
    const uint8_t expected2[] = {7, 8, 9, 10, 11, 12};
    g_assert_cmpmem(gdk_pixbuf_get_pixels(image) + gdk_pixbuf_get_rowstride(image), 6, expected2,
                    6);
}

static void rejected_buffers(void) {
    uint8_t bytes[16] = {0};
    struct spa_chunk chunk = {.size = sizeof bytes, .stride = 8};
    struct spa_data data = {
        .type = SPA_DATA_MemPtr, .data = bytes, .maxsize = sizeof bytes, .chunk = &chunk};
    struct spa_buffer buffer = {.n_datas = 1, .datas = &data};
    struct spa_video_info_raw info = {.format = SPA_VIDEO_FORMAT_BGRx, .size = SPA_RECTANGLE(2, 2)};
    for (int i = 0; i < 11; i++) {
        struct spa_data d = data;
        struct spa_chunk c = chunk;
        struct spa_buffer b = buffer;
        struct spa_video_info_raw f = info;
        d.chunk = &c;
        b.datas = &d;
        switch (i) {
        case 0:
            c.stride = -8;
            break;
        case 1:
            c.stride = 7;
            break;
        case 2:
            c.size = 15;
            break;
        case 3:
            c.offset = UINT32_MAX;
            break;
        case 4:
            d.data = NULL;
            break;
        case 5:
            c.flags = SPA_CHUNK_FLAG_CORRUPTED;
            break;
        case 6:
            f.size.width = UINT32_MAX;
            break;
        case 7:
            f.size.height = 0;
            break;
        case 8:
            f.format = SPA_VIDEO_FORMAT_BGRA;
            break;
        case 9:
            b.n_datas = 0;
            break;
        case 10:
            d.type = SPA_DATA_DmaBuf;
            break;
        }
        g_autoptr(GError) error = NULL;
        g_autoptr(GdkPixbuf) image = copy_frame(&b, &f, &error);
        g_assert_null(image);
        g_assert_nonnull(error);
    }
}

static void cropped_or_corrupt(void) {
    uint8_t bytes[16] = {0};
    struct spa_chunk chunk = {.size = sizeof bytes, .stride = 8};
    struct spa_data data = {
        .type = SPA_DATA_MemPtr, .data = bytes, .maxsize = sizeof bytes, .chunk = &chunk};
    struct spa_meta_region crop = {.region = SPA_REGION(0, 0, 1, 2)};
    struct spa_meta_header header = {.flags = SPA_META_HEADER_FLAG_CORRUPTED};
    struct spa_meta meta = {.type = SPA_META_VideoCrop, .size = sizeof crop, .data = &crop};
    struct spa_buffer buffer = {.n_datas = 1, .datas = &data, .n_metas = 1, .metas = &meta};
    struct spa_video_info_raw info = {.format = SPA_VIDEO_FORMAT_BGRx, .size = SPA_RECTANGLE(2, 2)};
    g_autoptr(GError) error = NULL;
    g_assert_null(copy_frame(&buffer, &info, &error));
    g_assert_nonnull(error);
    g_clear_error(&error);
    meta.type = SPA_META_Header;
    meta.size = sizeof header;
    meta.data = &header;
    g_assert_null(copy_frame(&buffer, &info, &error));
    g_assert_nonnull(error);
}

int main(int argc, char** argv) {
    g_test_init(&argc, &argv, NULL);
    g_test_add_func("/screenshot/pixels/padded-owned-bgrx", pixels);
    g_test_add_func("/screenshot/pixels/reject-invalid", rejected_buffers);
    g_test_add_func("/screenshot/pixels/reject-crop-corrupted", cropped_or_corrupt);
    return g_test_run();
}
