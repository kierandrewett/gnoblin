#define _GNU_SOURCE
#include "gnoblin-vr-vk.h"

#include <math.h>
#include <string.h>

/* glslc -mfmt=c emits the braces itself. */
static const uint32_t quad_vert_spv[] =
#include "quad.vert.inc"
    ;
static const uint32_t quad_frag_spv[] =
#include "quad.frag.inc"
    ;

#define MAX_TEXTURES 128
#define SAMPLES VK_SAMPLE_COUNT_4_BIT

typedef struct {
    char* key;
    VkImage image;
    VkDeviceMemory memory;
    VkImageView view;
    VkBuffer staging;
    VkDeviceMemory staging_memory;
    void* mapped;
    VkDescriptorSet set;
    uint32_t width;
    uint32_t height;
    uint32_t mips;
    guint64 serial;
    gboolean uploaded;
    gboolean seen; /* mark for the sweep */
} Texture;

typedef struct {
    VkImageView view;
    VkFramebuffer framebuffer;
    uint32_t width;
    uint32_t height;
} Target;

struct VrRenderer {
    VrVulkan vk;
    VkFormat format;
    VkFormat texture_format;
    VkRenderPass render_pass;
    VkDescriptorSetLayout set_layout;
    VkPipelineLayout pipeline_layout;
    VkPipeline pipeline;
    VkSampler sampler;
    VkDescriptorPool pool;
    VkCommandPool command_pool;
    VkCommandBuffer command;
    VkFence fence;
    GHashTable* textures;     /* key -> Texture */
    GHashTable* framebuffers; /* VkImage handle -> Target */
    /* The multisampled colour image the scene is drawn into before it resolves to the target. */
    VkImage msaa_image;
    VkDeviceMemory msaa_memory;
    VkImageView msaa_view;
    uint32_t msaa_width;
    uint32_t msaa_height;
};

#define VK_CHECK(call, what)                                                                       \
    do {                                                                                           \
        VkResult check_result = (call);                                                            \
        if (check_result != VK_SUCCESS) {                                                          \
            g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "%s failed (VkResult %d)", what,     \
                        (int)check_result);                                                        \
            return FALSE;                                                                          \
        }                                                                                          \
    } while (0)

void vr_vulkan_wanted_features(VkPhysicalDevice physical, VkPhysicalDeviceFeatures* features,
                               gboolean* anisotropy) {
    VkPhysicalDeviceFeatures supported;
    vkGetPhysicalDeviceFeatures(physical, &supported);
    memset(features, 0, sizeof *features);
    features->samplerAnisotropy = supported.samplerAnisotropy;
    if (anisotropy)
        *anisotropy = supported.samplerAnisotropy == VK_TRUE;
}

static gboolean find_memory(const VrRenderer* r, uint32_t type_bits, VkMemoryPropertyFlags wanted,
                            uint32_t* index) {
    VkPhysicalDeviceMemoryProperties properties;
    vkGetPhysicalDeviceMemoryProperties(r->vk.physical, &properties);
    for (uint32_t i = 0; i < properties.memoryTypeCount; i++)
        if ((type_bits & (1u << i)) &&
            (properties.memoryTypes[i].propertyFlags & wanted) == wanted) {
            *index = i;
            return TRUE;
        }
    return FALSE;
}

static gboolean create_buffer(VrRenderer* r, VkDeviceSize size, VkBufferUsageFlags usage,
                              VkMemoryPropertyFlags properties, VkBuffer* buffer,
                              VkDeviceMemory* memory, GError** error) {
    VkBufferCreateInfo info = {VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO};
    info.size = size;
    info.usage = usage;
    info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
    VK_CHECK(vkCreateBuffer(r->vk.device, &info, NULL, buffer), "vkCreateBuffer");
    VkMemoryRequirements requirements;
    vkGetBufferMemoryRequirements(r->vk.device, *buffer, &requirements);
    VkMemoryAllocateInfo allocate = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
    allocate.allocationSize = requirements.size;
    if (!find_memory(r, requirements.memoryTypeBits, properties, &allocate.memoryTypeIndex)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "no suitable buffer memory");
        return FALSE;
    }
    VK_CHECK(vkAllocateMemory(r->vk.device, &allocate, NULL, memory), "vkAllocateMemory");
    VK_CHECK(vkBindBufferMemory(r->vk.device, *buffer, *memory, 0), "vkBindBufferMemory");
    return TRUE;
}

static void texture_destroy(VrRenderer* r, Texture* texture) {
    vkDeviceWaitIdle(r->vk.device);
    if (texture->set)
        vkFreeDescriptorSets(r->vk.device, r->pool, 1, &texture->set);
    if (texture->mapped)
        vkUnmapMemory(r->vk.device, texture->staging_memory);
    vkDestroyBuffer(r->vk.device, texture->staging, NULL);
    vkFreeMemory(r->vk.device, texture->staging_memory, NULL);
    vkDestroyImageView(r->vk.device, texture->view, NULL);
    vkDestroyImage(r->vk.device, texture->image, NULL);
    vkFreeMemory(r->vk.device, texture->memory, NULL);
    g_free(texture->key);
    g_free(texture);
}

static Texture* texture_create(VrRenderer* r, const char* key, uint32_t width, uint32_t height,
                               GError** error) {
    Texture* texture = g_new0(Texture, 1);
    texture->key = g_strdup(key);
    texture->width = width;
    texture->height = height;
    texture->mips = 1;
    for (uint32_t size = MAX(width, height); size > 1; size >>= 1)
        texture->mips++;
    VkImageCreateInfo image = {VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
    image.imageType = VK_IMAGE_TYPE_2D;
    image.format = r->texture_format;
    image.extent = (VkExtent3D){width, height, 1};
    image.mipLevels = texture->mips;
    image.arrayLayers = 1;
    image.samples = VK_SAMPLE_COUNT_1_BIT;
    image.tiling = VK_IMAGE_TILING_OPTIMAL;
    image.usage = VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT |
                  VK_IMAGE_USAGE_SAMPLED_BIT;
    image.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    VkResult result = vkCreateImage(r->vk.device, &image, NULL, &texture->image);
    if (result == VK_SUCCESS) {
        VkMemoryRequirements requirements;
        vkGetImageMemoryRequirements(r->vk.device, texture->image, &requirements);
        VkMemoryAllocateInfo allocate = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
        allocate.allocationSize = requirements.size;
        if (find_memory(r, requirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
                        &allocate.memoryTypeIndex))
            result = vkAllocateMemory(r->vk.device, &allocate, NULL, &texture->memory);
        else
            result = VK_ERROR_OUT_OF_DEVICE_MEMORY;
    }
    if (result == VK_SUCCESS)
        result = vkBindImageMemory(r->vk.device, texture->image, texture->memory, 0);
    if (result == VK_SUCCESS) {
        VkImageViewCreateInfo view = {VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
        view.image = texture->image;
        view.viewType = VK_IMAGE_VIEW_TYPE_2D;
        view.format = image.format;
        view.subresourceRange =
            (VkImageSubresourceRange){VK_IMAGE_ASPECT_COLOR_BIT, 0, texture->mips, 0, 1};
        result = vkCreateImageView(r->vk.device, &view, NULL, &texture->view);
    }
    if (result == VK_SUCCESS &&
        !create_buffer(r, (VkDeviceSize)width * height * 4, VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
                       VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                       &texture->staging, &texture->staging_memory, error)) {
        texture_destroy(r, texture);
        return NULL;
    }
    if (result == VK_SUCCESS)
        result = vkMapMemory(r->vk.device, texture->staging_memory, 0, VK_WHOLE_SIZE, 0,
                             &texture->mapped);
    if (result == VK_SUCCESS) {
        VkDescriptorSetAllocateInfo allocate = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
        allocate.descriptorPool = r->pool;
        allocate.descriptorSetCount = 1;
        allocate.pSetLayouts = &r->set_layout;
        result = vkAllocateDescriptorSets(r->vk.device, &allocate, &texture->set);
    }
    if (result != VK_SUCCESS) {
        g_set_error(error, G_IO_ERROR, G_IO_ERROR_FAILED, "cannot create a window texture (%d)",
                    (int)result);
        texture_destroy(r, texture);
        return NULL;
    }
    VkDescriptorImageInfo descriptor = {r->sampler, texture->view,
                                        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
    VkWriteDescriptorSet write = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET};
    write.dstSet = texture->set;
    write.descriptorCount = 1;
    write.descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
    write.pImageInfo = &descriptor;
    vkUpdateDescriptorSets(r->vk.device, 1, &write, 0, NULL);
    return texture;
}

static void barrier(VkCommandBuffer command, VkImage image, uint32_t first_level,
                    uint32_t level_count, VkImageLayout from, VkImageLayout to,
                    VkAccessFlags source, VkAccessFlags destination, VkPipelineStageFlags first,
                    VkPipelineStageFlags last) {
    VkImageMemoryBarrier b = {VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER};
    b.srcAccessMask = source;
    b.dstAccessMask = destination;
    b.oldLayout = from;
    b.newLayout = to;
    b.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    b.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    b.image = image;
    b.subresourceRange =
        (VkImageSubresourceRange){VK_IMAGE_ASPECT_COLOR_BIT, first_level, level_count, 0, 1};
    vkCmdPipelineBarrier(command, first, last, 0, 0, NULL, 0, NULL, 1, &b);
}

/* Copies the staging pixels into level 0 and builds the rest of the chain by blitting down. */
static void upload_texture(VkCommandBuffer command, Texture* texture) {
    barrier(command, texture->image, 0, texture->mips, VK_IMAGE_LAYOUT_UNDEFINED,
            VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 0, VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT);
    VkBufferImageCopy copy = {0};
    copy.imageSubresource = (VkImageSubresourceLayers){VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1};
    copy.imageExtent = (VkExtent3D){texture->width, texture->height, 1};
    vkCmdCopyBufferToImage(command, texture->staging, texture->image,
                           VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &copy);
    int32_t width = (int32_t)texture->width, height = (int32_t)texture->height;
    for (uint32_t level = 1; level < texture->mips; level++) {
        barrier(command, texture->image, level - 1, 1, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, VK_ACCESS_TRANSFER_WRITE_BIT,
                VK_ACCESS_TRANSFER_READ_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_PIPELINE_STAGE_TRANSFER_BIT);
        int32_t next_width = MAX(width / 2, 1), next_height = MAX(height / 2, 1);
        VkImageBlit blit = {0};
        blit.srcSubresource =
            (VkImageSubresourceLayers){VK_IMAGE_ASPECT_COLOR_BIT, level - 1, 0, 1};
        blit.srcOffsets[1] = (VkOffset3D){width, height, 1};
        blit.dstSubresource = (VkImageSubresourceLayers){VK_IMAGE_ASPECT_COLOR_BIT, level, 0, 1};
        blit.dstOffsets[1] = (VkOffset3D){next_width, next_height, 1};
        vkCmdBlitImage(command, texture->image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                       texture->image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &blit,
                       VK_FILTER_LINEAR);
        barrier(command, texture->image, level - 1, 1, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_ACCESS_TRANSFER_READ_BIT,
                VK_ACCESS_SHADER_READ_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT);
        width = next_width;
        height = next_height;
    }
    barrier(command, texture->image, texture->mips - 1, 1, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT);
}

static VkShaderModule make_shader(VrRenderer* r, const uint32_t* code, size_t size) {
    VkShaderModuleCreateInfo info = {VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
    info.codeSize = size;
    info.pCode = code;
    VkShaderModule module = VK_NULL_HANDLE;
    vkCreateShaderModule(r->vk.device, &info, NULL, &module);
    return module;
}

VrRenderer* vr_renderer_new(const VrVulkan* vulkan, VkFormat color_format, GError** error) {
    VrRenderer* r = g_new0(VrRenderer, 1);
    r->vk = *vulkan;
    r->format = color_format;
    /* Window pixels are sRGB. An sRGB swapchain encodes on write, so decode on read to match. */
    r->texture_format =
        (color_format == VK_FORMAT_R8G8B8A8_SRGB || color_format == VK_FORMAT_B8G8R8A8_SRGB)
            ? VK_FORMAT_B8G8R8A8_SRGB
            : VK_FORMAT_B8G8R8A8_UNORM;
    r->textures = g_hash_table_new(g_str_hash, g_str_equal);
    r->framebuffers = g_hash_table_new(g_direct_hash, g_direct_equal);
    VkDevice device = vulkan->device;

    /* Attachment 0 is the multisampled image, attachment 1 the swapchain image it resolves to. */
    VkAttachmentDescription attachments[2] = {0};
    attachments[0].format = color_format;
    attachments[0].samples = SAMPLES;
    attachments[0].loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
    attachments[0].storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachments[0].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    attachments[0].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachments[0].initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    attachments[0].finalLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
    attachments[1].format = color_format;
    attachments[1].samples = VK_SAMPLE_COUNT_1_BIT;
    attachments[1].loadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    attachments[1].storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    attachments[1].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    attachments[1].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachments[1].initialLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
    attachments[1].finalLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
    VkAttachmentReference colour_reference = {0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkAttachmentReference resolve_reference = {1, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription subpass = {0};
    subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    subpass.colorAttachmentCount = 1;
    subpass.pColorAttachments = &colour_reference;
    subpass.pResolveAttachments = &resolve_reference;
    VkRenderPassCreateInfo pass = {VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO};
    pass.attachmentCount = 2;
    pass.pAttachments = attachments;
    pass.subpassCount = 1;
    pass.pSubpasses = &subpass;

    VkDescriptorSetLayoutBinding binding = {0, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 1,
                                            VK_SHADER_STAGE_FRAGMENT_BIT, NULL};
    VkDescriptorSetLayoutCreateInfo layout = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
    layout.bindingCount = 1;
    layout.pBindings = &binding;
    VkPushConstantRange push[2] = {{VK_SHADER_STAGE_VERTEX_BIT, 0, 64},
                                   {VK_SHADER_STAGE_FRAGMENT_BIT, 64, 16}};
    VkSamplerCreateInfo sampler = {VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO};
    sampler.magFilter = VK_FILTER_LINEAR;
    sampler.minFilter = VK_FILTER_LINEAR;
    sampler.mipmapMode = VK_SAMPLER_MIPMAP_MODE_LINEAR;
    sampler.addressModeU = sampler.addressModeV = sampler.addressModeW =
        VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    sampler.maxLod = VK_LOD_CLAMP_NONE;
    if (vulkan->sampler_anisotropy) {
        sampler.anisotropyEnable = VK_TRUE;
        sampler.maxAnisotropy = 8.0f;
    }
    VkDescriptorPoolSize pool_size = {VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, MAX_TEXTURES};
    VkDescriptorPoolCreateInfo pool = {VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
    pool.flags = VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT;
    pool.maxSets = MAX_TEXTURES;
    pool.poolSizeCount = 1;
    pool.pPoolSizes = &pool_size;
    VkCommandPoolCreateInfo command_pool = {VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
    command_pool.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;
    command_pool.queueFamilyIndex = vulkan->queue_family;
    VkFenceCreateInfo fence = {VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};

    gboolean ok =
        vkCreateRenderPass(device, &pass, NULL, &r->render_pass) == VK_SUCCESS &&
        vkCreateDescriptorSetLayout(device, &layout, NULL, &r->set_layout) == VK_SUCCESS &&
        vkCreateSampler(device, &sampler, NULL, &r->sampler) == VK_SUCCESS &&
        vkCreateDescriptorPool(device, &pool, NULL, &r->pool) == VK_SUCCESS &&
        vkCreateCommandPool(device, &command_pool, NULL, &r->command_pool) == VK_SUCCESS &&
        vkCreateFence(device, &fence, NULL, &r->fence) == VK_SUCCESS;
    if (ok) {
        VkPipelineLayoutCreateInfo pipeline_layout = {
            VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
        pipeline_layout.setLayoutCount = 1;
        pipeline_layout.pSetLayouts = &r->set_layout;
        pipeline_layout.pushConstantRangeCount = 2;
        pipeline_layout.pPushConstantRanges = push;
        ok = vkCreatePipelineLayout(device, &pipeline_layout, NULL, &r->pipeline_layout) ==
             VK_SUCCESS;
    }
    if (ok) {
        VkCommandBufferAllocateInfo allocate = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
        allocate.commandPool = r->command_pool;
        allocate.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
        allocate.commandBufferCount = 1;
        ok = vkAllocateCommandBuffers(device, &allocate, &r->command) == VK_SUCCESS;
    }
    if (ok) {
        VkShaderModule vertex = make_shader(r, quad_vert_spv, sizeof quad_vert_spv);
        VkShaderModule fragment = make_shader(r, quad_frag_spv, sizeof quad_frag_spv);
        VkPipelineShaderStageCreateInfo stages[2] = {
            {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, NULL, 0,
             VK_SHADER_STAGE_VERTEX_BIT, vertex, "main", NULL},
            {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, NULL, 0,
             VK_SHADER_STAGE_FRAGMENT_BIT, fragment, "main", NULL}};
        VkPipelineVertexInputStateCreateInfo input = {
            VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO};
        VkPipelineInputAssemblyStateCreateInfo assembly = {
            VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO};
        assembly.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP;
        VkPipelineViewportStateCreateInfo viewport = {
            VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO};
        viewport.viewportCount = 1;
        viewport.scissorCount = 1;
        VkPipelineRasterizationStateCreateInfo raster = {
            VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO};
        raster.polygonMode = VK_POLYGON_MODE_FILL;
        raster.cullMode = VK_CULL_MODE_NONE;
        raster.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE;
        raster.lineWidth = 1.0f;
        VkPipelineMultisampleStateCreateInfo multisample = {
            VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO};
        multisample.rasterizationSamples = SAMPLES;
        VkPipelineColorBlendAttachmentState blend = {0};
        blend.blendEnable = VK_TRUE;
        blend.srcColorBlendFactor = VK_BLEND_FACTOR_ONE;
        blend.dstColorBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
        blend.colorBlendOp = VK_BLEND_OP_ADD;
        blend.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE;
        blend.dstAlphaBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
        blend.alphaBlendOp = VK_BLEND_OP_ADD;
        blend.colorWriteMask = 0xF;
        VkPipelineColorBlendStateCreateInfo colour = {
            VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO};
        colour.attachmentCount = 1;
        colour.pAttachments = &blend;
        VkDynamicState dynamic_states[] = {VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR};
        VkPipelineDynamicStateCreateInfo dynamic = {
            VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO};
        dynamic.dynamicStateCount = 2;
        dynamic.pDynamicStates = dynamic_states;
        VkGraphicsPipelineCreateInfo info = {VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO};
        info.stageCount = 2;
        info.pStages = stages;
        info.pVertexInputState = &input;
        info.pInputAssemblyState = &assembly;
        info.pViewportState = &viewport;
        info.pRasterizationState = &raster;
        info.pMultisampleState = &multisample;
        info.pColorBlendState = &colour;
        info.pDynamicState = &dynamic;
        info.layout = r->pipeline_layout;
        info.renderPass = r->render_pass;
        ok = vertex && fragment &&
             vkCreateGraphicsPipelines(device, VK_NULL_HANDLE, 1, &info, NULL, &r->pipeline) ==
                 VK_SUCCESS;
        if (vertex)
            vkDestroyShaderModule(device, vertex, NULL);
        if (fragment)
            vkDestroyShaderModule(device, fragment, NULL);
    }
    if (!ok) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED,
                            "cannot create the Vulkan scene renderer");
        vr_renderer_free(r);
        return NULL;
    }
    return r;
}

static void destroy_targets(VrRenderer* r) {
    GHashTableIter iter;
    gpointer key, value;
    g_hash_table_iter_init(&iter, r->framebuffers);
    while (g_hash_table_iter_next(&iter, &key, &value)) {
        Target* target = value;
        vkDestroyFramebuffer(r->vk.device, target->framebuffer, NULL);
        vkDestroyImageView(r->vk.device, target->view, NULL);
        g_free(target);
    }
    g_hash_table_remove_all(r->framebuffers);
}

static void destroy_msaa(VrRenderer* r) {
    if (r->msaa_view)
        vkDestroyImageView(r->vk.device, r->msaa_view, NULL);
    if (r->msaa_image)
        vkDestroyImage(r->vk.device, r->msaa_image, NULL);
    if (r->msaa_memory)
        vkFreeMemory(r->vk.device, r->msaa_memory, NULL);
    r->msaa_view = VK_NULL_HANDLE;
    r->msaa_image = VK_NULL_HANDLE;
    r->msaa_memory = VK_NULL_HANDLE;
    r->msaa_width = r->msaa_height = 0;
}

void vr_renderer_free(VrRenderer* r) {
    if (!r)
        return;
    VkDevice device = r->vk.device;
    vkDeviceWaitIdle(device);
    GHashTableIter iter;
    gpointer key, value;
    g_hash_table_iter_init(&iter, r->textures);
    while (g_hash_table_iter_next(&iter, &key, &value))
        texture_destroy(r, value);
    destroy_targets(r);
    destroy_msaa(r);
    g_hash_table_unref(r->textures);
    g_hash_table_unref(r->framebuffers);
    if (r->pipeline)
        vkDestroyPipeline(device, r->pipeline, NULL);
    if (r->pipeline_layout)
        vkDestroyPipelineLayout(device, r->pipeline_layout, NULL);
    if (r->fence)
        vkDestroyFence(device, r->fence, NULL);
    if (r->command_pool)
        vkDestroyCommandPool(device, r->command_pool, NULL);
    if (r->pool)
        vkDestroyDescriptorPool(device, r->pool, NULL);
    if (r->sampler)
        vkDestroySampler(device, r->sampler, NULL);
    if (r->set_layout)
        vkDestroyDescriptorSetLayout(device, r->set_layout, NULL);
    if (r->render_pass)
        vkDestroyRenderPass(device, r->render_pass, NULL);
    g_free(r);
}

static gboolean ensure_msaa(VrRenderer* r, uint32_t width, uint32_t height, GError** error) {
    if (r->msaa_image && r->msaa_width == width && r->msaa_height == height)
        return TRUE;
    vkDeviceWaitIdle(r->vk.device);
    destroy_targets(r); /* framebuffers reference the old view */
    destroy_msaa(r);
    VkImageCreateInfo info = {VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
    info.imageType = VK_IMAGE_TYPE_2D;
    info.format = r->format;
    info.extent = (VkExtent3D){width, height, 1};
    info.mipLevels = 1;
    info.arrayLayers = 1;
    info.samples = SAMPLES;
    info.tiling = VK_IMAGE_TILING_OPTIMAL;
    info.usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
    info.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    VK_CHECK(vkCreateImage(r->vk.device, &info, NULL, &r->msaa_image), "vkCreateImage");
    VkMemoryRequirements requirements;
    vkGetImageMemoryRequirements(r->vk.device, r->msaa_image, &requirements);
    VkMemoryAllocateInfo allocate = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
    allocate.allocationSize = requirements.size;
    if (!find_memory(r, requirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
                     &allocate.memoryTypeIndex)) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "no memory for multisampling");
        return FALSE;
    }
    VK_CHECK(vkAllocateMemory(r->vk.device, &allocate, NULL, &r->msaa_memory), "vkAllocateMemory");
    VK_CHECK(vkBindImageMemory(r->vk.device, r->msaa_image, r->msaa_memory, 0),
             "vkBindImageMemory");
    VkImageViewCreateInfo view = {VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
    view.image = r->msaa_image;
    view.viewType = VK_IMAGE_VIEW_TYPE_2D;
    view.format = r->format;
    view.subresourceRange = (VkImageSubresourceRange){VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    VK_CHECK(vkCreateImageView(r->vk.device, &view, NULL, &r->msaa_view), "vkCreateImageView");
    r->msaa_width = width;
    r->msaa_height = height;
    return TRUE;
}

static Target* target_for(VrRenderer* r, VkImage image, uint32_t width, uint32_t height,
                          GError** error) {
    if (!ensure_msaa(r, width, height, error))
        return NULL;
    Target* target = g_hash_table_lookup(r->framebuffers, image);
    if (target)
        return target;
    target = g_new0(Target, 1);
    target->width = width;
    target->height = height;
    VkImageViewCreateInfo view = {VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
    view.image = image;
    view.viewType = VK_IMAGE_VIEW_TYPE_2D;
    view.format = r->format;
    view.subresourceRange = (VkImageSubresourceRange){VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    VkImageView views[2];
    VkFramebufferCreateInfo framebuffer = {VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
    framebuffer.renderPass = r->render_pass;
    framebuffer.attachmentCount = 2;
    framebuffer.pAttachments = views;
    framebuffer.width = width;
    framebuffer.height = height;
    framebuffer.layers = 1;
    if (vkCreateImageView(r->vk.device, &view, NULL, &target->view) != VK_SUCCESS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "cannot create a target view");
        g_free(target);
        return NULL;
    }
    views[0] = r->msaa_view;
    views[1] = target->view;
    if (vkCreateFramebuffer(r->vk.device, &framebuffer, NULL, &target->framebuffer) != VK_SUCCESS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "cannot create a framebuffer");
        vkDestroyImageView(r->vk.device, target->view, NULL);
        g_free(target);
        return NULL;
    }
    g_hash_table_insert(r->framebuffers, image, target);
    return target;
}

gboolean vr_renderer_draw(VrRenderer* r, const VrDrawItem* items, guint count, VkImage image,
                          uint32_t width, uint32_t height, const float view_matrix[16],
                          const float projection[16], const float clear_colour[4], GError** error) {
    Target* target = target_for(r, image, width, height, error);
    if (!target)
        return FALSE;

    VkCommandBuffer command = r->command;
    VK_CHECK(vkResetCommandBuffer(command, 0), "vkResetCommandBuffer");
    VkCommandBufferBeginInfo begin = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    begin.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
    VK_CHECK(vkBeginCommandBuffer(command, &begin), "vkBeginCommandBuffer");

    /* Upload changed pixels, then drop textures nothing refers to any more. */
    GHashTableIter iter;
    gpointer key, value;
    g_hash_table_iter_init(&iter, r->textures);
    while (g_hash_table_iter_next(&iter, &key, &value))
        ((Texture*)value)->seen = FALSE;
    for (guint i = 0; i < count; i++) {
        const VrDrawItem* item = &items[i];
        Texture* texture = g_hash_table_lookup(r->textures, item->key);
        if (texture && item->pixels &&
            (texture->width != item->pixel_width || texture->height != item->pixel_height)) {
            g_hash_table_remove(r->textures, item->key);
            texture_destroy(r, texture);
            texture = NULL;
        }
        if (!texture) {
            if (!item->pixels || g_hash_table_size(r->textures) >= MAX_TEXTURES)
                continue;
            texture = texture_create(r, item->key, item->pixel_width, item->pixel_height, error);
            if (!texture)
                return FALSE;
            g_hash_table_insert(r->textures, texture->key, texture);
        }
        texture->seen = TRUE;
        if (item->pixels && (!texture->uploaded || texture->serial != item->serial)) {
            memcpy(texture->mapped, item->pixels, (size_t)texture->width * texture->height * 4);
            upload_texture(command, texture);
            texture->uploaded = TRUE;
            texture->serial = item->serial;
        }
    }
    GPtrArray* stale = g_ptr_array_new();
    g_hash_table_iter_init(&iter, r->textures);
    while (g_hash_table_iter_next(&iter, &key, &value))
        if (!((Texture*)value)->seen)
            g_ptr_array_add(stale, value);
    for (guint i = 0; i < stale->len; i++) {
        Texture* texture = g_ptr_array_index(stale, i);
        g_hash_table_remove(r->textures, texture->key);
        texture_destroy(r, texture);
    }
    g_ptr_array_unref(stale);

    VkClearValue clears[2];
    memset(clears, 0, sizeof clears);
    memcpy(clears[0].color.float32, clear_colour, 4 * sizeof(float));
    VkRenderPassBeginInfo pass = {VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO};
    pass.renderPass = r->render_pass;
    pass.framebuffer = target->framebuffer;
    pass.renderArea = (VkRect2D){{0, 0}, {width, height}};
    pass.clearValueCount = 2;
    pass.pClearValues = clears;
    vkCmdBeginRenderPass(command, &pass, VK_SUBPASS_CONTENTS_INLINE);
    vkCmdBindPipeline(command, VK_PIPELINE_BIND_POINT_GRAPHICS, r->pipeline);
    /* A negative height flips Y so the projection's up is up in the image. */
    VkViewport viewport = {0.0f, (float)height, (float)width, -(float)height, 0.0f, 1.0f};
    VkRect2D scissor = {{0, 0}, {width, height}};
    vkCmdSetViewport(command, 0, 1, &viewport);
    vkCmdSetScissor(command, 0, 1, &scissor);

    float view_projection[16];
    /* out = projection * view */
    for (int column = 0; column < 4; column++)
        for (int row = 0; row < 4; row++) {
            float sum = 0.0f;
            for (int k = 0; k < 4; k++)
                sum += projection[k * 4 + row] * view_matrix[column * 4 + k];
            view_projection[column * 4 + row] = sum;
        }
    for (guint i = 0; i < count; i++) {
        const VrDrawItem* item = &items[i];
        Texture* texture = g_hash_table_lookup(r->textures, item->key);
        if (!texture || !texture->uploaded)
            continue;
        float mvp[16];
        for (int column = 0; column < 4; column++)
            for (int row = 0; row < 4; row++) {
                float sum = 0.0f;
                for (int k = 0; k < 4; k++)
                    sum += view_projection[k * 4 + row] * item->model[column * 4 + k];
                mvp[column * 4 + row] = sum;
            }
        float shape[4] = {item->size[0], item->size[1], item->radius, item->opacity};
        vkCmdPushConstants(command, r->pipeline_layout, VK_SHADER_STAGE_VERTEX_BIT, 0, 64, mvp);
        vkCmdPushConstants(command, r->pipeline_layout, VK_SHADER_STAGE_FRAGMENT_BIT, 64, 16,
                           shape);
        vkCmdBindDescriptorSets(command, VK_PIPELINE_BIND_POINT_GRAPHICS, r->pipeline_layout, 0, 1,
                                &texture->set, 0, NULL);
        vkCmdDraw(command, 4, 1, 0, 0);
    }
    vkCmdEndRenderPass(command);
    VK_CHECK(vkEndCommandBuffer(command), "vkEndCommandBuffer");

    VkSubmitInfo submit = {VK_STRUCTURE_TYPE_SUBMIT_INFO};
    submit.commandBufferCount = 1;
    submit.pCommandBuffers = &command;
    VK_CHECK(vkResetFences(r->vk.device, 1, &r->fence), "vkResetFences");
    VK_CHECK(vkQueueSubmit(r->vk.queue, 1, &submit, r->fence), "vkQueueSubmit");
    VK_CHECK(vkWaitForFences(r->vk.device, 1, &r->fence, VK_TRUE, UINT64_MAX), "vkWaitForFences");
    return TRUE;
}

/* --- Offscreen --------------------------------------------------------------------------- */

gboolean vr_vulkan_create_offscreen(VrVulkan* vulkan, GError** error) {
    memset(vulkan, 0, sizeof *vulkan);
    VkApplicationInfo application = {VK_STRUCTURE_TYPE_APPLICATION_INFO};
    application.pApplicationName = "gnoblin-vr";
    application.apiVersion = VK_API_VERSION_1_1;
    VkInstanceCreateInfo instance = {VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    instance.pApplicationInfo = &application;
    VK_CHECK(vkCreateInstance(&instance, NULL, &vulkan->instance), "vkCreateInstance");
    VkPhysicalDevice devices[8];
    uint32_t count = 8;
    VK_CHECK(vkEnumeratePhysicalDevices(vulkan->instance, &count, devices),
             "vkEnumeratePhysicalDevices");
    if (count == 0) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "no Vulkan device");
        return FALSE;
    }
    vulkan->physical = devices[0];
    uint32_t families = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(vulkan->physical, &families, NULL);
    VkQueueFamilyProperties properties[16];
    families = MIN(families, 16u);
    vkGetPhysicalDeviceQueueFamilyProperties(vulkan->physical, &families, properties);
    vulkan->queue_family = G_MAXUINT32;
    for (uint32_t i = 0; i < families; i++)
        if (properties[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) {
            vulkan->queue_family = i;
            break;
        }
    if (vulkan->queue_family == G_MAXUINT32) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "no graphics queue");
        return FALSE;
    }
    float priority = 1.0f;
    VkDeviceQueueCreateInfo queue = {VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
    queue.queueFamilyIndex = vulkan->queue_family;
    queue.queueCount = 1;
    queue.pQueuePriorities = &priority;
    VkPhysicalDeviceFeatures features;
    vr_vulkan_wanted_features(vulkan->physical, &features, &vulkan->sampler_anisotropy);
    VkDeviceCreateInfo device = {VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
    device.queueCreateInfoCount = 1;
    device.pQueueCreateInfos = &queue;
    device.pEnabledFeatures = &features;
    VK_CHECK(vkCreateDevice(vulkan->physical, &device, NULL, &vulkan->device), "vkCreateDevice");
    vkGetDeviceQueue(vulkan->device, vulkan->queue_family, 0, &vulkan->queue);
    return TRUE;
}

void vr_vulkan_destroy_offscreen(VrVulkan* vulkan) {
    if (vulkan->device)
        vkDestroyDevice(vulkan->device, NULL);
    if (vulkan->instance)
        vkDestroyInstance(vulkan->instance, NULL);
    memset(vulkan, 0, sizeof *vulkan);
}

static void submit_and_wait(VrRenderer* r) {
    VkSubmitInfo submit = {VK_STRUCTURE_TYPE_SUBMIT_INFO};
    submit.commandBufferCount = 1;
    submit.pCommandBuffers = &r->command;
    vkResetFences(r->vk.device, 1, &r->fence);
    vkQueueSubmit(r->vk.queue, 1, &submit, r->fence);
    vkWaitForFences(r->vk.device, 1, &r->fence, VK_TRUE, UINT64_MAX);
}

guint8* vr_renderer_draw_to_rgba(VrRenderer* r, const VrDrawItem* items, guint count,
                                 uint32_t width, uint32_t height, const float view_matrix[16],
                                 const float projection[16], const float clear_colour[4],
                                 GError** error) {
    VkImageCreateInfo info = {VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
    info.imageType = VK_IMAGE_TYPE_2D;
    info.format = r->format;
    info.extent = (VkExtent3D){width, height, 1};
    info.mipLevels = 1;
    info.arrayLayers = 1;
    info.samples = VK_SAMPLE_COUNT_1_BIT;
    info.tiling = VK_IMAGE_TILING_OPTIMAL;
    info.usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
    info.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    VkImage image = VK_NULL_HANDLE;
    VkDeviceMemory memory = VK_NULL_HANDLE;
    VkBuffer readback = VK_NULL_HANDLE;
    VkDeviceMemory readback_memory = VK_NULL_HANDLE;
    guint8* result = NULL;
    VkDeviceSize size = (VkDeviceSize)width * height * 4;
    if (vkCreateImage(r->vk.device, &info, NULL, &image) != VK_SUCCESS) {
        g_set_error_literal(error, G_IO_ERROR, G_IO_ERROR_FAILED, "cannot create the target");
        return NULL;
    }
    VkMemoryRequirements requirements;
    vkGetImageMemoryRequirements(r->vk.device, image, &requirements);
    VkMemoryAllocateInfo allocate = {VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
    allocate.allocationSize = requirements.size;
    if (!find_memory(r, requirements.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
                     &allocate.memoryTypeIndex) ||
        vkAllocateMemory(r->vk.device, &allocate, NULL, &memory) != VK_SUCCESS ||
        vkBindImageMemory(r->vk.device, image, memory, 0) != VK_SUCCESS ||
        !create_buffer(r, size, VK_BUFFER_USAGE_TRANSFER_DST_BIT,
                       VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
                       &readback, &readback_memory, error))
        goto out;

    /* Put the image in the layout the renderer expects, as an XR runtime would hand it over. */
    {
        VkCommandBufferBeginInfo begin = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
        begin.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        vkResetCommandBuffer(r->command, 0);
        vkBeginCommandBuffer(r->command, &begin);
        barrier(r->command, image, 0, 1, VK_IMAGE_LAYOUT_UNDEFINED,
                VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, 0, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT);
        vkEndCommandBuffer(r->command);
        submit_and_wait(r);
    }
    if (!vr_renderer_draw(r, items, count, image, width, height, view_matrix, projection,
                          clear_colour, error))
        goto out;
    {
        VkCommandBufferBeginInfo begin = {VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
        begin.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        vkResetCommandBuffer(r->command, 0);
        vkBeginCommandBuffer(r->command, &begin);
        barrier(r->command, image, 0, 1, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_ACCESS_TRANSFER_READ_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_PIPELINE_STAGE_TRANSFER_BIT);
        VkBufferImageCopy copy = {0};
        copy.imageSubresource = (VkImageSubresourceLayers){VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1};
        copy.imageExtent = (VkExtent3D){width, height, 1};
        vkCmdCopyImageToBuffer(r->command, image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, readback, 1,
                               &copy);
        vkEndCommandBuffer(r->command);
        submit_and_wait(r);
    }
    {
        void* mapped = NULL;
        if (vkMapMemory(r->vk.device, readback_memory, 0, size, 0, &mapped) == VK_SUCCESS) {
            result = g_memdup2(mapped, size);
            vkUnmapMemory(r->vk.device, readback_memory);
        }
    }
out:
    /* The framebuffer cache keys on the image handle, so drop it before the image goes. */
    {
        Target* target = g_hash_table_lookup(r->framebuffers, image);
        if (target) {
            vkDeviceWaitIdle(r->vk.device);
            vkDestroyFramebuffer(r->vk.device, target->framebuffer, NULL);
            vkDestroyImageView(r->vk.device, target->view, NULL);
            g_free(target);
            g_hash_table_remove(r->framebuffers, image);
        }
    }
    if (readback)
        vkDestroyBuffer(r->vk.device, readback, NULL);
    if (readback_memory)
        vkFreeMemory(r->vk.device, readback_memory, NULL);
    if (image)
        vkDestroyImage(r->vk.device, image, NULL);
    if (memory)
        vkFreeMemory(r->vk.device, memory, NULL);
    return result;
}
