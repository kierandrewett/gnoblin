#pragma once

#include <vulkan/vulkan.h>
#include <gio/gio.h>

/* Draws textured quads for the VR client with Vulkan: 4x multisampling, mipmapped textures with
 * anisotropic filtering, and rounded-corner edges that stay smooth at any distance. The same
 * code renders into an OpenXR swapchain image and into an offscreen image for the preview and
 * tests, so what the tests see is what the headset gets. */

typedef struct {
    VkInstance instance;
    VkPhysicalDevice physical;
    VkDevice device;
    uint32_t queue_family;
    VkQueue queue;
    gboolean sampler_anisotropy; /* the device was created with samplerAnisotropy */
} VrVulkan;

/* The device features the renderer wants, filled from what the physical device supports. Pass
 * it to vkCreateDevice (directly or through xrCreateVulkanDeviceKHR). */
void vr_vulkan_wanted_features(VkPhysicalDevice physical, VkPhysicalDeviceFeatures* features,
                               gboolean* anisotropy);

typedef struct VrRenderer VrRenderer;

/* One quad. The texture is cached under @key; pixels are uploaded when @serial differs from the
 * cached one, so a caller passes the same pixels every frame and only pays for changes. */
typedef struct {
    const char* key;
    const guint8* pixels; /* premultiplied BGRA, NULL to reuse the cached texture */
    uint32_t pixel_width;
    uint32_t pixel_height;
    guint64 serial;
    float model[16]; /* a unit quad centred on the origin, in the XY plane */
    float size[2];   /* metres, for the corner mask */
    float radius;    /* corner radius in metres */
    float opacity;
} VrDrawItem;

VrRenderer* vr_renderer_new(const VrVulkan* vulkan, VkFormat color_format, GError** error);
void vr_renderer_free(VrRenderer* renderer);

/* Renders @items in order into @image, which must be in COLOR_ATTACHMENT_OPTIMAL and stays
 * there. @clear is the background colour, premultiplied, alpha 0 for passthrough. Textures
 * whose key is absent from @items are released. */
gboolean vr_renderer_draw(VrRenderer* renderer, const VrDrawItem* items, guint count, VkImage image,
                          uint32_t width, uint32_t height, const float view_matrix[16],
                          const float projection[16], const float clear[4], GError** error);

/* Offscreen target for the preview and tests: creates a VrVulkan on the first Vulkan device. */
gboolean vr_vulkan_create_offscreen(VrVulkan* vulkan, GError** error);
void vr_vulkan_destroy_offscreen(VrVulkan* vulkan);

/* Renders to a temporary image and returns tightly packed RGBA8 pixels (free with g_free). */
guint8* vr_renderer_draw_to_rgba(VrRenderer* renderer, const VrDrawItem* items, guint count,
                                 uint32_t width, uint32_t height, const float view_matrix[16],
                                 const float projection[16], const float clear[4], GError** error);
