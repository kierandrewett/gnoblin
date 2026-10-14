#pragma once

#include <vulkan/vulkan.h>
#define XR_USE_GRAPHICS_API_VULKAN 1
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#include <glib.h>
#include "gnoblin-vr-math.h"

/* Controller input: an aim pose and the buttons that matter for pointing at a desktop. */

typedef struct {
    gboolean active; /* the aim pose is tracked this frame */
    VrPose aim;
    gboolean select; /* trigger */
    gboolean grab;   /* grip */
    gboolean menu;
    float scroll[2]; /* thumbstick, -1 to 1 */
} VrHand;

typedef struct VrInput VrInput;

/* Creates the actions, suggests bindings for the common controller profiles and attaches them to
 * the session. Returns NULL when the runtime accepts none; the scene still renders. */
VrInput* vr_input_new(XrInstance instance, XrSession session);
void vr_input_free(VrInput* input);
/* Reads both hands (index 0 left, 1 right) relative to @space at @time. */
void vr_input_sync(VrInput* input, XrSession session, XrSpace space, XrTime time, VrHand hands[2]);
