#pragma once

#include <stdint.h>

/* One window frame exported by the compositor for the VR client, at
 * $XDG_RUNTIME_DIR/gnoblin-vr/frames/<name>.rgba. The compositor writes it to a temporary file
 * and renames it into place, so a reader sees a whole frame or the previous one.
 *
 * The header is followed by height rows of stride bytes: premultiplied BGRA, 8 bits per channel,
 * top row first (Cairo ARGB32 on a little-endian machine). */
#define GNOBLIN_VR_FRAME_MAGIC 0x31465647u /* "GVF1" */
#define GNOBLIN_VR_FRAME_MAX_DIMENSION 8192

typedef struct {
    uint32_t magic;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint64_t serial; /* changes with every new frame */
    uint64_t reserved;
} GnoblinVrFrameHeader;
