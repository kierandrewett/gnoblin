#pragma once

#include <glib.h>
#include "gnoblin-vr-math.h"

/* The scene as the presentation client sees it: the surfaces the compositor published in
 * scene.ini and the newest pixels of each window. */

typedef struct {
    char* id;
    char* frame; /* file name stem under frames/ */
    gboolean head_locked;
    VrPose anchor; /* pose of the anchor the surface hangs from */
    VrPose local;  /* pose relative to the anchor */
    double width;  /* metres */
    double height;
    /* Latest frame. pixels is NULL until the first frame arrives. */
    guint8* pixels;
    guint32 pixel_width;
    guint32 pixel_height;
    guint64 serial;
    gboolean fresh;      /* pixels changed since the renderer last uploaded them */
    guint64 texture_key; /* renderer-owned slot for the uploaded image */
} VrSurface;

typedef struct {
    guint64 revision;
    char* environment;   /* "opaque" or "passthrough" */
    GPtrArray* surfaces; /* VrSurface*, lowest stacking rank first */
    char* directory;
    gint64 scene_mtime;
    gint64 scene_size;
} VrView;

VrView* vr_view_new(const char* directory);
void vr_view_free(VrView* view);
/* Re-reads scene.ini when it changed and every surface's newest frame. Returns TRUE if anything
 * changed since the last call. */
gboolean vr_view_refresh(VrView* view);
/* Nearest surface a ray crosses, with the point on it. Returns NULL when the ray misses. */
typedef struct {
    const VrSurface* surface;
    double distance;
    double u; /* 0 at the left edge, 1 at the right */
    double v; /* 0 at the top edge, 1 at the bottom */
    double point[3];
} VrHit;
gboolean vr_view_hit(const VrView* view, const VrPose* head, const double origin[3],
                     const double direction[3], VrHit* hit);
/* World pose of a surface; head-locked anchors follow @head. */
VrPose vr_surface_world_pose(const VrSurface* surface, const VrPose* head);
