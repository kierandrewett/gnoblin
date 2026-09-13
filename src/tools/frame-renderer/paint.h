/* SPDX-License-Identifier: MIT */
#pragma once
#include <stdint.h>
#include <poll.h>
struct FramePaint {
    int width, height, top, right, bottom, left;
    uint32_t state, color;
    const char *title, *style;
    void* view;
    uint32_t hover, pressed;
};
#ifdef __cplusplus
extern "C" {
#endif
void frame_paint(void* pixels, const struct FramePaint* model);
void frame_paint_init(int* argc, char*** argv);
/* Optional hooks. Transport supplies defaults when a painter omits them. */
void* frame_paint_create(void);
void frame_paint_destroy(void* view);
typedef void (*FrameRegion)(void*, uint32_t, int, int, int, int);
int frame_paint_regions(const struct FramePaint*, FrameRegion emit, void* context);
int frame_paint_dispatch(void); /* nonzero requests repaint after toolkit/theme changes */
int frame_paint_timeout(void);  /* milliseconds; -1 means no toolkit polling */
int frame_paint_needs_redraw(void* view);
int frame_paint_poll(struct pollfd* fds, nfds_t count, int timeout);
#ifdef __cplusplus
}
#endif
