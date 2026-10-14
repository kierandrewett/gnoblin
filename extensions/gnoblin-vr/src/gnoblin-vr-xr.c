#define _GNU_SOURCE
#include <vulkan/vulkan.h>
#define XR_USE_GRAPHICS_API_VULKAN 1
#include "gnoblin-vr-common.h"
#include "gnoblin-vr-compose.h"
#include "gnoblin-vr-input.h"
#include "gnoblin-vr-vk.h"

#include <string.h>

/* The OpenXR session: one projection layer, one swapchain per eye, drawn by the Vulkan scene
 * renderer from the scene the compositor publishes, with a ray and cursor for each controller.
 * Used for every runtime (Monado, WiVRn). */

#define MAX_VIEWS 4

typedef struct {
    XrSwapchain swapchain;
    XrSwapchainImageVulkan2KHR* images;
    uint32_t image_count;
    uint32_t width;
    uint32_t height;
} Eye;

static const int64_t preferred_formats[] = {VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_B8G8R8A8_UNORM,
                                            VK_FORMAT_R8G8B8A8_SRGB, VK_FORMAT_B8G8R8A8_SRGB};

static gboolean choose_format(XrSession session, VkFormat* format) {
    uint32_t count = 0;
    if (xrEnumerateSwapchainFormats(session, 0, &count, NULL) != XR_SUCCESS || !count)
        return FALSE;
    int64_t* formats = g_new0(int64_t, count);
    gboolean found = FALSE;
    if (xrEnumerateSwapchainFormats(session, count, &count, formats) == XR_SUCCESS)
        for (guint p = 0; !found && p < G_N_ELEMENTS(preferred_formats); p++)
            for (uint32_t i = 0; i < count; i++)
                if (formats[i] == preferred_formats[p]) {
                    *format = (VkFormat)preferred_formats[p];
                    found = TRUE;
                    break;
                }
    g_free(formats);
    return found;
}

static gboolean create_eye(XrSession session, VkFormat format,
                           const XrViewConfigurationView* configuration, Eye* eye) {
    XrSwapchainCreateInfo info = {XR_TYPE_SWAPCHAIN_CREATE_INFO};
    info.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT;
    info.format = format;
    info.sampleCount = 1;
    info.width = configuration->recommendedImageRectWidth;
    info.height = configuration->recommendedImageRectHeight;
    info.faceCount = 1;
    info.arraySize = 1;
    info.mipCount = 1;
    eye->width = info.width;
    eye->height = info.height;
    if (xrCreateSwapchain(session, &info, &eye->swapchain) != XR_SUCCESS)
        return FALSE;
    if (xrEnumerateSwapchainImages(eye->swapchain, 0, &eye->image_count, NULL) != XR_SUCCESS ||
        !eye->image_count)
        return FALSE;
    eye->images = g_new0(XrSwapchainImageVulkan2KHR, eye->image_count);
    for (uint32_t i = 0; i < eye->image_count; i++)
        eye->images[i].type = XR_TYPE_SWAPCHAIN_IMAGE_VULKAN2_KHR;
    return xrEnumerateSwapchainImages(eye->swapchain, eye->image_count, &eye->image_count,
                                      (XrSwapchainImageBaseHeader*)eye->images) == XR_SUCCESS;
}

static VrPose pose_from_xr(const XrPosef* pose) {
    VrPose result = {
        {pose->position.x, pose->position.y, pose->position.z},
        {pose->orientation.x, pose->orientation.y, pose->orientation.z, pose->orientation.w}};
    return result;
}

/* Floor level first, so a window placed 1.5 m up is at eye height for a standing user. A
 * runtime that only knows the head origin gets that origin dropped by an average eye height. */
static XrSpace create_world_space(XrSession session, const char** description) {
    static const struct {
        XrReferenceSpaceType type;
        float drop;
        const char* name;
    } choices[] = {{XR_REFERENCE_SPACE_TYPE_LOCAL_FLOOR, 0.0f, "local floor"},
                   {XR_REFERENCE_SPACE_TYPE_STAGE, 0.0f, "stage"},
                   {XR_REFERENCE_SPACE_TYPE_LOCAL, -1.6f, "local, floor assumed 1.6 m below"}};
    for (guint i = 0; i < G_N_ELEMENTS(choices); i++) {
        XrReferenceSpaceCreateInfo info = {XR_TYPE_REFERENCE_SPACE_CREATE_INFO};
        info.referenceSpaceType = choices[i].type;
        info.poseInReferenceSpace.orientation.w = 1.0f;
        info.poseInReferenceSpace.position.y = choices[i].drop;
        XrSpace space = XR_NULL_HANDLE;
        if (xrCreateReferenceSpace(session, &info, &space) == XR_SUCCESS) {
            *description = choices[i].name;
            return space;
        }
    }
    return XR_NULL_HANDLE;
}

static gboolean blend_mode_supported(XrInstance instance, XrSystemId system,
                                     XrEnvironmentBlendMode wanted) {
    uint32_t count = 0;
    if (xrEnumerateEnvironmentBlendModes(instance, system,
                                         XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &count,
                                         NULL) != XR_SUCCESS ||
        !count)
        return FALSE;
    XrEnvironmentBlendMode* modes = g_new0(XrEnvironmentBlendMode, count);
    gboolean found = FALSE;
    if (xrEnumerateEnvironmentBlendModes(instance, system,
                                         XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, count, &count,
                                         modes) == XR_SUCCESS)
        for (uint32_t i = 0; i < count; i++)
            found |= modes[i] == wanted;
    g_free(modes);
    return found;
}

/* The controller state other processes read: head, both aim poses, buttons and what each ray
 * hits, so the Lua policy can point, click, grab and scroll without a second tracking path. */
static void publish_input(const VrPose* head, const VrHand hands[2], const VrHit hits[2],
                          const gboolean hit_valid[2]) {
    static guint64 serial;
    static gint64 last_write;
    static char* last_text;
    GString* text = g_string_new(NULL);
    g_string_append_printf(text, "[head]\npos=%.4f;%.4f;%.4f\nrot=%.5f;%.5f;%.5f;%.5f\n",
                           head->position[0], head->position[1], head->position[2],
                           head->rotation[0], head->rotation[1], head->rotation[2],
                           head->rotation[3]);
    static const char* const names[2] = {"left", "right"};
    for (int h = 0; h < 2; h++) {
        g_string_append_printf(text, "\n[%s]\nactive=%d\n", names[h], hands[h].active);
        if (!hands[h].active)
            continue;
        const VrPose* aim = &hands[h].aim;
        g_string_append_printf(
            text,
            "pos=%.4f;%.4f;%.4f\nrot=%.5f;%.5f;%.5f;%.5f\nselect=%d\ngrab=%d\nmenu=%d\n"
            "scroll=%.3f;%.3f\n",
            aim->position[0], aim->position[1], aim->position[2], aim->rotation[0],
            aim->rotation[1], aim->rotation[2], aim->rotation[3], hands[h].select, hands[h].grab,
            hands[h].menu, hands[h].scroll[0], hands[h].scroll[1]);
        if (hit_valid[h])
            g_string_append_printf(text,
                                   "hit_surface=%s\nhit_u=%.4f\nhit_v=%.4f\nhit_distance=%.3f\n",
                                   hits[h].surface->id, hits[h].u, hits[h].v, hits[h].distance);
    }
    /* Write when something changed, at most 30 times a second. */
    gint64 now = g_get_monotonic_time();
    if (g_strcmp0(text->str, last_text) == 0 || now - last_write < 33000) {
        g_string_free(text, TRUE);
        return;
    }
    last_write = now;
    g_free(last_text);
    last_text = g_strdup(text->str);
    g_autofree char* directory = g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", NULL);
    g_mkdir_with_parents(directory, 0700);
    g_autofree char* path = g_build_filename(directory, "input.ini", NULL);
    g_autofree char* full =
        g_strdup_printf("[input]\nserial=%" G_GUINT64_FORMAT "\n\n%s", ++serial, text->str);
    g_file_set_contents(path, full, -1, NULL);
    g_string_free(text, TRUE);
}

int gnoblin_vr_xr_main(void) {
    if (!gnoblin_vr_extension_available(XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME))
        return 99; /* The runtime cannot take Vulkan clients; the caller tries something else. */

    const char* extensions[2] = {XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME};
    uint32_t extension_count = 1;
    if (gnoblin_vr_extension_available("XR_META_touch_controller_plus"))
        extensions[extension_count++] = "XR_META_touch_controller_plus";
    XrInstanceCreateInfo create_info = {XR_TYPE_INSTANCE_CREATE_INFO};
    g_strlcpy(create_info.applicationInfo.applicationName, "Gnoblin VR",
              sizeof create_info.applicationInfo.applicationName);
    create_info.applicationInfo.applicationVersion = 1;
    g_strlcpy(create_info.applicationInfo.engineName, "Gnoblin",
              sizeof create_info.applicationInfo.engineName);
    create_info.applicationInfo.engineVersion = 1;
    create_info.applicationInfo.apiVersion = XR_CURRENT_API_VERSION;
    create_info.enabledExtensionCount = extension_count;
    create_info.enabledExtensionNames = extensions;
    XrInstance instance = XR_NULL_HANDLE;
    XrResult result = xrCreateInstance(&create_info, &instance);
    if (result == XR_ERROR_RUNTIME_UNAVAILABLE || result == XR_ERROR_FORM_FACTOR_UNAVAILABLE)
        return gnoblin_vr_absent("no OpenXR runtime is available");
    if (result != XR_SUCCESS)
        return gnoblin_vr_failed("the OpenXR loader could not create a Vulkan instance");

    XrSystemGetInfo system_info = {XR_TYPE_SYSTEM_GET_INFO};
    system_info.formFactor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
    XrSystemId system_id = XR_NULL_SYSTEM_ID;
    result = xrGetSystem(instance, &system_info, &system_id);
    if (result == XR_ERROR_FORM_FACTOR_UNAVAILABLE || result == XR_ERROR_SYSTEM_INVALID) {
        xrDestroyInstance(instance);
        return gnoblin_vr_absent("OpenXR is installed, but no headset is available");
    }
    if (result != XR_SUCCESS) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the OpenXR runtime could not query a headset");
    }

    PFN_xrGetVulkanGraphicsRequirements2KHR get_requirements = NULL;
    PFN_xrCreateVulkanInstanceKHR create_vulkan_instance = NULL;
    PFN_xrGetVulkanGraphicsDevice2KHR get_vulkan_device = NULL;
    PFN_xrCreateVulkanDeviceKHR create_vulkan_device = NULL;
    xrGetInstanceProcAddr(instance, "xrGetVulkanGraphicsRequirements2KHR",
                          (PFN_xrVoidFunction*)&get_requirements);
    xrGetInstanceProcAddr(instance, "xrCreateVulkanInstanceKHR",
                          (PFN_xrVoidFunction*)&create_vulkan_instance);
    xrGetInstanceProcAddr(instance, "xrGetVulkanGraphicsDevice2KHR",
                          (PFN_xrVoidFunction*)&get_vulkan_device);
    xrGetInstanceProcAddr(instance, "xrCreateVulkanDeviceKHR",
                          (PFN_xrVoidFunction*)&create_vulkan_device);
    if (!get_requirements || !create_vulkan_instance || !get_vulkan_device ||
        !create_vulkan_device) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime lacks the Vulkan entry points");
    }
    XrGraphicsRequirementsVulkan2KHR requirements = {XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR};
    if (get_requirements(instance, system_id, &requirements) != XR_SUCCESS) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime rejected the Vulkan requirements query");
    }

    VkApplicationInfo application = {VK_STRUCTURE_TYPE_APPLICATION_INFO};
    application.pApplicationName = "Gnoblin VR";
    application.apiVersion = VK_API_VERSION_1_1;
    VkInstanceCreateInfo vk_instance_info = {VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    vk_instance_info.pApplicationInfo = &application;
    XrVulkanInstanceCreateInfoKHR instance_request = {XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR};
    instance_request.systemId = system_id;
    instance_request.pfnGetInstanceProcAddr = vkGetInstanceProcAddr;
    instance_request.vulkanCreateInfo = &vk_instance_info;
    VrVulkan vulkan = {0};
    VkResult vk_result = VK_SUCCESS;
    if (create_vulkan_instance(instance, &instance_request, &vulkan.instance, &vk_result) !=
            XR_SUCCESS ||
        vk_result != VK_SUCCESS) {
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime could not create a Vulkan instance");
    }
    XrVulkanGraphicsDeviceGetInfoKHR device_query = {XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR};
    device_query.systemId = system_id;
    device_query.vulkanInstance = vulkan.instance;
    if (get_vulkan_device(instance, &device_query, &vulkan.physical) != XR_SUCCESS) {
        vkDestroyInstance(vulkan.instance, NULL);
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime did not pick a Vulkan device");
    }
    uint32_t families = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(vulkan.physical, &families, NULL);
    VkQueueFamilyProperties properties[32];
    families = MIN(families, 32u);
    vkGetPhysicalDeviceQueueFamilyProperties(vulkan.physical, &families, properties);
    vulkan.queue_family = G_MAXUINT32;
    for (uint32_t i = 0; i < families; i++)
        if (properties[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) {
            vulkan.queue_family = i;
            break;
        }
    float priority = 1.0f;
    VkDeviceQueueCreateInfo queue = {VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
    queue.queueFamilyIndex = vulkan.queue_family;
    queue.queueCount = 1;
    queue.pQueuePriorities = &priority;
    VkPhysicalDeviceFeatures features;
    vr_vulkan_wanted_features(vulkan.physical, &features, &vulkan.sampler_anisotropy);
    VkDeviceCreateInfo vk_device_info = {VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
    vk_device_info.queueCreateInfoCount = 1;
    vk_device_info.pQueueCreateInfos = &queue;
    vk_device_info.pEnabledFeatures = &features;
    XrVulkanDeviceCreateInfoKHR device_request = {XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR};
    device_request.systemId = system_id;
    device_request.pfnGetInstanceProcAddr = vkGetInstanceProcAddr;
    device_request.vulkanPhysicalDevice = vulkan.physical;
    device_request.vulkanCreateInfo = &vk_device_info;
    if (vulkan.queue_family == G_MAXUINT32 ||
        create_vulkan_device(instance, &device_request, &vulkan.device, &vk_result) != XR_SUCCESS ||
        vk_result != VK_SUCCESS) {
        vkDestroyInstance(vulkan.instance, NULL);
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime could not create a Vulkan device");
    }
    vkGetDeviceQueue(vulkan.device, vulkan.queue_family, 0, &vulkan.queue);

    XrGraphicsBindingVulkan2KHR binding = {XR_TYPE_GRAPHICS_BINDING_VULKAN2_KHR};
    binding.instance = vulkan.instance;
    binding.physicalDevice = vulkan.physical;
    binding.device = vulkan.device;
    binding.queueFamilyIndex = vulkan.queue_family;
    binding.queueIndex = 0;
    XrSessionCreateInfo session_info = {XR_TYPE_SESSION_CREATE_INFO};
    session_info.next = &binding;
    session_info.systemId = system_id;
    XrSession session = XR_NULL_HANDLE;
    result = xrCreateSession(instance, &session_info, &session);
    if (result != XR_SUCCESS) {
        vkDestroyDevice(vulkan.device, NULL);
        vkDestroyInstance(vulkan.instance, NULL);
        xrDestroyInstance(instance);
        return gnoblin_vr_failed("the runtime could not create a Vulkan session");
    }

    uint32_t view_count = 0;
    XrViewConfigurationView configuration[MAX_VIEWS];
    for (int i = 0; i < MAX_VIEWS; i++)
        configuration[i].type = XR_TYPE_VIEW_CONFIGURATION_VIEW;
    result = xrEnumerateViewConfigurationViews(instance, system_id,
                                               XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, MAX_VIEWS,
                                               &view_count, configuration);
    VkFormat format = VK_FORMAT_UNDEFINED;
    const char* space_description = "";
    XrSpace space = XR_NULL_HANDLE;
    Eye eyes[MAX_VIEWS] = {0};
    VrRenderer* renderer = NULL;
    VrInput* input = NULL;
    const char* failure = NULL;
    g_autoptr(GError) error = NULL;
    if (result != XR_SUCCESS || view_count < 2 || view_count > MAX_VIEWS)
        failure = "the headset does not report a supported stereo view configuration";
    else if (!(space = create_world_space(session, &space_description)))
        failure = "the runtime could not create a reference space";
    else if (!choose_format(session, &format))
        failure = "the runtime offers no usable 8-bit colour format";
    else {
        for (uint32_t i = 0; i < view_count && !failure; i++)
            if (!create_eye(session, format, &configuration[i], &eyes[i]))
                failure = "the runtime could not create swapchains";
        if (!failure) {
            renderer = vr_renderer_new(&vulkan, format, &error);
            if (!renderer)
                failure = error->message;
        }
    }
    guint32 widths[MAX_VIEWS] = {0}, heights[MAX_VIEWS] = {0};
    for (uint32_t i = 0; i < view_count && i < MAX_VIEWS; i++) {
        widths[i] = eyes[i].width;
        heights[i] = eyes[i].height;
    }
    int exit_code = 0;
    g_autofree char* directory = g_build_filename(g_get_user_runtime_dir(), "gnoblin-vr", NULL);
    VrView* scene = vr_view_new(directory);
    if (failure) {
        exit_code = gnoblin_vr_failed(failure);
    } else {
        gboolean passthrough_available =
            blend_mode_supported(instance, system_id, XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND);
        g_message("gnoblin-vr: %s space, passthrough %s", space_description,
                  passthrough_available ? "available" : "unavailable");
        input = vr_input_new(instance, session);
        if (!input)
            g_message("gnoblin-vr: no controller bindings were accepted; rendering without rays");
        VrCursor cursor = {0};
        VrCursor fallback_cursor = {0};
        vr_cursor_default(&fallback_cursor);
        guint8 ray_pixels[2][4];
        vr_compose_solid_pixel(ray_pixels[0], 120, 200, 255, 200);
        vr_compose_solid_pixel(ray_pixels[1], 255, 170, 90, 200);
        int dominant = 1;
        gboolean previous_select[2] = {FALSE, FALSE};
        GArray* items = g_array_new(FALSE, FALSE, sizeof(VrDrawItem));

        gnoblin_vr_write_terminal_status("stopped",
                                         "waiting for the OpenXR session to become ready",
                                         view_count, widths, heights);
        gboolean began = FALSE, lost = FALSE;
        XrCompositionLayerProjectionView layer_views[MAX_VIEWS];
        while (!gnoblin_vr_stopping && !lost) {
            XrEventDataBuffer event = {XR_TYPE_EVENT_DATA_BUFFER};
            result = xrPollEvent(instance, &event);
            if (result == XR_EVENT_UNAVAILABLE) {
                if (!began) {
                    g_usleep(10000);
                    continue;
                }
            } else if (result != XR_SUCCESS) {
                lost = TRUE;
                break;
            } else if (event.type == XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) {
                XrEventDataSessionStateChanged* changed = (XrEventDataSessionStateChanged*)&event;
                if (changed->state == XR_SESSION_STATE_READY && !began) {
                    XrSessionBeginInfo begin_info = {XR_TYPE_SESSION_BEGIN_INFO};
                    begin_info.primaryViewConfigurationType =
                        XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
                    if (xrBeginSession(session, &begin_info) != XR_SUCCESS) {
                        lost = TRUE;
                        break;
                    }
                    began = TRUE;
                    gnoblin_vr_write_terminal_status("running", "OpenXR session is active",
                                                     view_count, widths, heights);
                    g_message("gnoblin-vr: OpenXR session is running (%u views)", view_count);
                } else if (changed->state == XR_SESSION_STATE_STOPPING && began) {
                    xrEndSession(session);
                    began = FALSE;
                    lost = TRUE;
                } else if (changed->state == XR_SESSION_STATE_EXITING ||
                           changed->state == XR_SESSION_STATE_LOSS_PENDING) {
                    lost = TRUE;
                }
            }
            if (!began)
                continue;
            XrFrameState frame = {XR_TYPE_FRAME_STATE};
            if (xrWaitFrame(session, NULL, &frame) != XR_SUCCESS ||
                xrBeginFrame(session, NULL) != XR_SUCCESS) {
                lost = TRUE;
                break;
            }
            XrViewLocateInfo locate = {XR_TYPE_VIEW_LOCATE_INFO};
            locate.viewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
            locate.displayTime = frame.predictedDisplayTime;
            locate.space = space;
            XrView views[MAX_VIEWS];
            for (uint32_t i = 0; i < view_count; i++)
                views[i] = (XrView){XR_TYPE_VIEW};
            uint32_t located = 0;
            XrViewState view_state = {XR_TYPE_VIEW_STATE};
            XrCompositionLayerProjection projection = {XR_TYPE_COMPOSITION_LAYER_PROJECTION};
            gboolean have_layer = FALSE;
            gboolean passthrough = FALSE;
            if (frame.shouldRender &&
                xrLocateViews(session, &locate, &view_state, view_count, &located, views) ==
                    XR_SUCCESS &&
                located == view_count) {
                vr_view_refresh(scene);
                const char* wanted = g_getenv("GNOBLIN_VR_ENVIRONMENT");
                if (!wanted)
                    wanted = scene->environment;
                passthrough = passthrough_available && wanted && g_str_equal(wanted, "passthrough");

                VrPose head = pose_from_xr(&views[0].pose);
                head.position[0] = (views[0].pose.position.x + views[1].pose.position.x) / 2.0;
                head.position[1] = (views[0].pose.position.y + views[1].pose.position.y) / 2.0;
                head.position[2] = (views[0].pose.position.z + views[1].pose.position.z) / 2.0;
                double head_up[3], up_axis[3] = {0, 1, 0};
                vr_quat_rotate(head.rotation, up_axis, head_up);

                VrHand hands[2] = {{0}, {0}};
                VrHit hits[2];
                gboolean hit_valid[2] = {FALSE, FALSE};
                if (input)
                    vr_input_sync(input, session, space, frame.predictedDisplayTime, hands);
                for (int h = 0; h < 2; h++) {
                    if (hands[h].select && !previous_select[h])
                        dominant = h;
                    previous_select[h] = hands[h].select;
                }
                g_array_set_size(items, 0);
                vr_compose_surfaces(scene, &head, items);
                gboolean have_sprite = vr_cursor_refresh(&cursor, directory);
                for (int h = 0; h < 2; h++) {
                    if (!hands[h].active)
                        continue;
                    double forward_axis[3] = {0, 0, -1}, direction[3];
                    vr_quat_rotate(hands[h].aim.rotation, forward_axis, direction);
                    hit_valid[h] =
                        vr_view_hit(scene, &head, hands[h].aim.position, direction, &hits[h]);
                    double length = hit_valid[h] ? hits[h].distance : 1.5;
                    vr_compose_ray(items, h == 0 ? "ray:left" : "ray:right", hands[h].aim.position,
                                   direction, length, head.position, ray_pixels[h],
                                   hands[h].select ? 0.006f : 0.003f);
                }
                if (hands[dominant].active && hit_valid[dominant])
                    vr_compose_cursor(items, have_sprite ? &cursor : &fallback_cursor,
                                      hits[dominant].point, head.position, head_up);
                if (input)
                    publish_input(&head, hands, hits, hit_valid);

                float clear[4] = {0.03f, 0.035f, 0.05f, 1.0f};
                if (passthrough)
                    clear[0] = clear[1] = clear[2] = clear[3] = 0.0f;
                for (uint32_t i = 0; i < view_count; i++) {
                    uint32_t index = 0;
                    XrSwapchainImageAcquireInfo acquire = {XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO};
                    XrSwapchainImageWaitInfo wait = {XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO};
                    wait.timeout = XR_INFINITE_DURATION;
                    if (xrAcquireSwapchainImage(eyes[i].swapchain, &acquire, &index) !=
                            XR_SUCCESS ||
                        xrWaitSwapchainImage(eyes[i].swapchain, &wait) != XR_SUCCESS)
                        continue;
                    VrPose eye_pose = pose_from_xr(&views[i].pose);
                    float view_matrix[16], projection_matrix[16];
                    vr_mat4_view(&eye_pose, view_matrix);
                    vr_mat4_projection(views[i].fov.angleLeft, views[i].fov.angleRight,
                                       views[i].fov.angleUp, views[i].fov.angleDown, 0.05f, 100.0f,
                                       projection_matrix);
                    g_autoptr(GError) draw_error = NULL;
                    if (!vr_renderer_draw(renderer, (const VrDrawItem*)items->data, items->len,
                                          eyes[i].images[index].image, eyes[i].width,
                                          eyes[i].height, view_matrix, projection_matrix, clear,
                                          &draw_error))
                        g_warning("gnoblin-vr: %s", draw_error->message);
                    XrSwapchainImageReleaseInfo release = {XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO};
                    xrReleaseSwapchainImage(eyes[i].swapchain, &release);
                    memset(&layer_views[i], 0, sizeof layer_views[i]);
                    layer_views[i].type = XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW;
                    layer_views[i].pose = views[i].pose;
                    layer_views[i].fov = views[i].fov;
                    layer_views[i].subImage.swapchain = eyes[i].swapchain;
                    layer_views[i].subImage.imageRect.extent.width = (int32_t)eyes[i].width;
                    layer_views[i].subImage.imageRect.extent.height = (int32_t)eyes[i].height;
                }
                projection.space = space;
                projection.viewCount = view_count;
                projection.views = layer_views;
                /* The layer is premultiplied, so the runtime blends with its alpha as it is. */
                if (passthrough)
                    projection.layerFlags = XR_COMPOSITION_LAYER_BLEND_TEXTURE_SOURCE_ALPHA_BIT;
                have_layer = TRUE;
            }
            XrFrameEndInfo end = {XR_TYPE_FRAME_END_INFO};
            end.displayTime = frame.predictedDisplayTime;
            end.environmentBlendMode = passthrough ? XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND
                                                   : XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
            const XrCompositionLayerBaseHeader* layers[1] = {
                (XrCompositionLayerBaseHeader*)&projection};
            end.layerCount = have_layer ? 1 : 0;
            end.layers = layers;
            if (xrEndFrame(session, &end) != XR_SUCCESS)
                lost = TRUE;
        }
        if (began)
            xrEndSession(session);
        gnoblin_vr_write_terminal_status(gnoblin_vr_stopping ? "stopped" : "absent",
                                         gnoblin_vr_stopping ? "stop requested"
                                                             : "the session ended",
                                         view_count, widths, heights);
        exit_code = gnoblin_vr_stopping ? 0 : 1;
        g_array_unref(items);
        vr_cursor_clear(&cursor);
        vr_cursor_clear(&fallback_cursor);
    }
    vr_input_free(input);
    vr_view_free(scene);
    vr_renderer_free(renderer);
    for (uint32_t i = 0; i < MAX_VIEWS; i++) {
        if (eyes[i].swapchain)
            xrDestroySwapchain(eyes[i].swapchain);
        g_free(eyes[i].images);
    }
    if (space)
        xrDestroySpace(space);
    xrDestroySession(session);
    vkDestroyDevice(vulkan.device, NULL);
    vkDestroyInstance(vulkan.instance, NULL);
    xrDestroyInstance(instance);
    return exit_code;
}
