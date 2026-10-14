#define _GNU_SOURCE
#include "gnoblin-vr-preview.h"
#include "gnoblin-vr-compose.h"
#include "gnoblin-vr-vk.h"

#include <stdio.h>
#include <string.h>

static gboolean write_ppm(const char* path, const guint8* rgba, uint32_t width, uint32_t height) {
    FILE* file = fopen(path, "wb");
    if (!file)
        return FALSE;
    fprintf(file, "P6\n%u %u\n255\n", width, height);
    for (gsize i = 0; i < (gsize)width * height; i++)
        fwrite(rgba + i * 4, 1, 3, file);
    return fclose(file) == 0;
}

/* Renders both eyes of the published scene without a headset or an OpenXR runtime. The eyes sit
 * 64 mm apart at GNOBLIN_VR_HEAD (x,y,z metres, default 0,1.6,0) looking down -Z. */
int gnoblin_vr_render_eyes_main(const char* output_directory) {
    g_autofree char* directory = g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", NULL);
    VrView* view = vr_view_new(directory);
    vr_view_refresh(view);

    VrPose head = {{0.0, 1.6, 0.0}, {0.0, 0.0, 0.0, 1.0}};
    const char* head_text = g_getenv("GNOBLIN_VR_HEAD");
    if (head_text)
        sscanf(head_text, "%lf,%lf,%lf", &head.position[0], &head.position[1], &head.position[2]);

    VrVulkan vulkan;
    g_autoptr(GError) error = NULL;
    if (!vr_vulkan_create_offscreen(&vulkan, &error)) {
        g_printerr("gnoblin-vr: %s\n", error->message);
        return 1;
    }
    VrRenderer* renderer = vr_renderer_new(&vulkan, VK_FORMAT_R8G8B8A8_UNORM, &error);
    if (!renderer) {
        g_printerr("gnoblin-vr: %s\n", error->message);
        vr_vulkan_destroy_offscreen(&vulkan);
        return 1;
    }
    g_mkdir_with_parents(output_directory, 0755);
    const uint32_t size = 960;
    const double half_angle = 50.0 * G_PI / 180.0;
    int status = 0;
    for (int eye = 0; eye < 2; eye++) {
        VrPose pose = head;
        pose.position[0] += eye == 0 ? -0.032 : 0.032;
        float view_matrix[16], projection[16];
        vr_mat4_view(&pose, view_matrix);
        vr_mat4_projection(-half_angle, half_angle, half_angle, -half_angle, 0.05f, 100.0f,
                           projection);
        g_autoptr(GArray) items = g_array_new(FALSE, FALSE, sizeof(VrDrawItem));
        vr_compose_surfaces(view, &head, items);
        const float clear[4] = {0.03f, 0.035f, 0.05f, 1.0f};
        guint8* pixels =
            vr_renderer_draw_to_rgba(renderer, (const VrDrawItem*)items->data, items->len, size,
                                     size, view_matrix, projection, clear, &error);
        if (!pixels) {
            g_printerr("gnoblin-vr: %s\n", error ? error->message : "render failed");
            status = 1;
            break;
        }
        g_autofree char* name = g_strdup_printf("%s.ppm", eye == 0 ? "left" : "right");
        g_autofree char* path = g_build_filename(output_directory, name, NULL);
        if (!write_ppm(path, pixels, size, size)) {
            g_printerr("gnoblin-vr: cannot write %s\n", path);
            status = 1;
        }
        g_free(pixels);
    }
    g_print("rendered %u surface(s) at scene revision %" G_GUINT64_FORMAT "\n", view->surfaces->len,
            view->revision);
    vr_renderer_free(renderer);
    vr_vulkan_destroy_offscreen(&vulkan);
    vr_view_free(view);
    return status;
}
