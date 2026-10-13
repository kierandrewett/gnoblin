/*
 * Dear ImGui to Cogl renderer used by compositor-owned diagnostic surfaces.
 * It is intentionally small: ImGui emits draw lists, this content paints them
 * inside an existing Clutter actor, and callers provide the UI in a callback.
 */

#include "config.h"

#include "gnoblin-imgui-overlay.hpp"

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <vector>

#include "cogl/cogl.h"
#include "gnoblin-plugin-ui.h"
#include "imgui.h"
#include "imgui_internal.h"

struct _GnoblinImGuiOverlay {
    ClutterActor* actor;
    ClutterContent* content;
    ImGuiContext* context;
    CoglTexture* font_texture;
    float font_scale;
    GnoblinImGuiOverlayDrawFunc draw_callback;
    void* draw_user_data;
    GDestroyNotify draw_destroy;
    guint input_tick_id;
};

typedef struct _GnoblinImGuiContent {
    GObject parent;
    GnoblinImGuiOverlay* overlay;
} GnoblinImGuiContent;

typedef struct _GnoblinImGuiContentClass {
    GObjectClass parent_class;
} GnoblinImGuiContentClass;

static void gnoblin_imgui_content_interface_init(ClutterContentInterface* iface);

G_DEFINE_TYPE_WITH_CODE(GnoblinImGuiContent, gnoblin_imgui_content, G_TYPE_OBJECT,
                        G_IMPLEMENT_INTERFACE(CLUTTER_TYPE_CONTENT,
                                              gnoblin_imgui_content_interface_init))

static void gnoblin_imgui_content_class_init(GnoblinImGuiContentClass* klass) {
    (void)klass;
}

static void gnoblin_imgui_content_init(GnoblinImGuiContent* content) {
    content->overlay = nullptr;
}

static void gnoblin_imgui_render_draw_data(GnoblinImGuiOverlay* overlay,
                                           CoglFramebuffer* framebuffer, ImDrawData* draw_data) {
    if (draw_data == nullptr || draw_data->TotalVtxCount == 0)
        return;

    CoglContext* cogl_context = cogl_framebuffer_get_context(framebuffer);
    /* The overlay paints in the actor's logical coordinates, the same space as
     * ImGui's vertices and clip rectangles. Output density only changes the
     * font bake, so clip rectangles are not scaled. */
    const ImVec2 clip_offset = draw_data->DisplayPos;

    /* Only the font atlas is ever bound, so one pipeline serves the frame. */
    CoglTexture* pipeline_texture = nullptr;
    CoglPipeline* pipeline = nullptr;
    std::vector<guint> indices;

    for (int list_index = 0; list_index < draw_data->CmdListsCount; list_index++) {
        const ImDrawList* list = draw_data->CmdLists[list_index];
        if (list->VtxBuffer.Size == 0)
            continue;

        /* Upload each list's vertices once; its commands only differ by clip
         * rectangle and index range. */
        CoglAttributeBuffer* vertex_buffer = cogl_attribute_buffer_new(
            cogl_context, list->VtxBuffer.Size * sizeof(ImDrawVert), list->VtxBuffer.Data);
        CoglAttribute* attributes[3];
        attributes[0] = cogl_attribute_new(vertex_buffer, "cogl_position_in", sizeof(ImDrawVert),
                                           offsetof(ImDrawVert, pos), 2, COGL_ATTRIBUTE_TYPE_FLOAT);
        attributes[1] = cogl_attribute_new(vertex_buffer, "cogl_tex_coord0_in", sizeof(ImDrawVert),
                                           offsetof(ImDrawVert, uv), 2, COGL_ATTRIBUTE_TYPE_FLOAT);
        attributes[2] =
            cogl_attribute_new(vertex_buffer, "cogl_color_in", sizeof(ImDrawVert),
                               offsetof(ImDrawVert, col), 4, COGL_ATTRIBUTE_TYPE_UNSIGNED_BYTE);
        cogl_attribute_set_normalized(attributes[2], TRUE);

        for (int command_index = 0; command_index < list->CmdBuffer.Size; command_index++) {
            const ImDrawCmd* command = &list->CmdBuffer[command_index];
            if (command->UserCallback != nullptr) {
                if (command->UserCallback != ImDrawCallback_ResetRenderState)
                    command->UserCallback(list, command);
                continue;
            }

            const float clip_x1 = command->ClipRect.x - clip_offset.x;
            const float clip_y1 = command->ClipRect.y - clip_offset.y;
            const float clip_x2 = command->ClipRect.z - clip_offset.x;
            const float clip_y2 = command->ClipRect.w - clip_offset.y;
            if (clip_x1 >= clip_x2 || clip_y1 >= clip_y2)
                continue;

            CoglTexture* texture =
                reinterpret_cast<CoglTexture*>(static_cast<std::uintptr_t>(command->GetTexID()));
            if (texture == nullptr)
                texture = overlay->font_texture;
            if (texture == nullptr)
                continue;

            if (pipeline == nullptr || texture != pipeline_texture) {
                if (pipeline != nullptr)
                    g_object_unref(pipeline);
                pipeline = cogl_pipeline_new(cogl_context);
                cogl_pipeline_set_layer_texture(pipeline, 0, texture);
                cogl_pipeline_set_layer_filters(pipeline, 0, COGL_PIPELINE_FILTER_LINEAR,
                                                COGL_PIPELINE_FILTER_LINEAR);
                g_autoptr(GError) error = nullptr;
                cogl_pipeline_set_blend(
                    pipeline, "RGBA = ADD(SRC_COLOR*(SRC_COLOR[A]), DST_COLOR*(1-SRC_COLOR[A]))",
                    &error);
                pipeline_texture = texture;
            }

            indices.clear();
            indices.reserve(command->ElemCount);
            for (unsigned int i = 0; i < command->ElemCount; i++)
                indices.push_back(list->IdxBuffer[command->IdxOffset + i] + command->VtxOffset);

            CoglPrimitive* primitive = cogl_primitive_new_with_attributes(
                COGL_VERTICES_MODE_TRIANGLES, list->VtxBuffer.Size, attributes,
                G_N_ELEMENTS(attributes));
            CoglIndices* cogl_indices = cogl_indices_new(
                cogl_context, COGL_INDICES_TYPE_UNSIGNED_INT, indices.data(), indices.size());
            cogl_primitive_set_indices(primitive, cogl_indices, indices.size());

            cogl_framebuffer_push_rectangle_clip(framebuffer, clip_x1, clip_y1, clip_x2, clip_y2);
            cogl_primitive_draw(primitive, framebuffer, pipeline);
            cogl_framebuffer_pop_clip(framebuffer);

            g_object_unref(cogl_indices);
            g_object_unref(primitive);
        }

        g_object_unref(attributes[0]);
        g_object_unref(attributes[1]);
        g_object_unref(attributes[2]);
        g_object_unref(vertex_buffer);
    }
    if (pipeline != nullptr)
        g_object_unref(pipeline);
}

/* Panels must stay readable when the user's shell is broken, so they cannot
 * depend on fontconfig, Pango or a shell font. Prefer a monospace TrueType font,
 * then a proportional one, and keep ImGui's built-in font if none is installed.
 * stb_truetype cannot read CFF (.otf). */
static void gnoblin_imgui_load_system_font(ImGuiIO& io, float density) {
    static const char* const candidates[] = {
        "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/liberation-mono/LiberationMono-Regular.ttf",
        "/usr/share/fonts/google-noto-vf/NotoSansMono[wght].ttf",
        "/usr/share/fonts/adwaita-mono-fonts/AdwaitaMono-Regular.ttf",
        "/usr/share/fonts/adwaita-sans-fonts/AdwaitaSans-Regular.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    };
    /* The logical size stays 15 px: a sturdy monospace face with wide letterforms reads better
     * than a light, narrow one at this size. Density only raises the bake resolution so
     * text is sharp on a scaled output. */
    ImFontConfig config;
    config.RasterizerDensity = density;
    /* One sample per pixel on whole-pixel positions keeps stems sharp; the
     * default horizontal oversampling smears small text. */
    config.OversampleH = 1;
    config.OversampleV = 1;
    config.PixelSnapH = true;
    for (const char* path : candidates) {
        if (!g_file_test(path, G_FILE_TEST_IS_REGULAR))
            continue;
        if (io.Fonts->AddFontFromFileTTF(path, 15.0f, &config) != nullptr)
            return;
    }
    config.SizePixels = 15.0f;
    io.Fonts->AddFontDefault(&config);
}

/* Flat, square and neutral grey: a debug layer that looks the same over any
 * shell. */
static void gnoblin_imgui_apply_style() {
    ImGuiStyle& style = ImGui::GetStyle();
    ImGui::StyleColorsDark(&style);
    style.WindowRounding = style.ChildRounding = style.FrameRounding = 0.0f;
    style.PopupRounding = style.ScrollbarRounding = style.GrabRounding = 0.0f;
    style.WindowBorderSize = 1.0f;
    style.PopupBorderSize = 1.0f;
    style.FrameBorderSize = 0.0f;
    style.WindowPadding = ImVec2(10.0f, 8.0f);
    style.FramePadding = ImVec2(8.0f, 3.0f);
    style.ItemSpacing = ImVec2(8.0f, 4.0f);
    style.ScrollbarSize = 10.0f;

    const auto grey = [](int value, float alpha = 1.0f) {
        const float v = static_cast<float>(value) / 255.0f;
        return ImVec4(v, v, v, alpha);
    };
    ImVec4* c = style.Colors;
    c[ImGuiCol_Text] = grey(0xe8);
    c[ImGuiCol_TextDisabled] = grey(0x9c);
    c[ImGuiCol_WindowBg] = grey(0x1e);
    c[ImGuiCol_ChildBg] = ImVec4(0, 0, 0, 0);
    c[ImGuiCol_PopupBg] = grey(0x25);
    c[ImGuiCol_Border] = grey(0x3c);
    c[ImGuiCol_TitleBg] = c[ImGuiCol_TitleBgActive] = c[ImGuiCol_TitleBgCollapsed] = grey(0x2d);
    c[ImGuiCol_FrameBg] = c[ImGuiCol_FrameBgHovered] = c[ImGuiCol_FrameBgActive] = grey(0x1e);
    c[ImGuiCol_Button] = grey(0x33);
    c[ImGuiCol_ButtonHovered] = grey(0x44);
    c[ImGuiCol_ButtonActive] = grey(0x50);
    c[ImGuiCol_Header] = ImVec4(0.149f, 0.310f, 0.471f, 1.0f);
    c[ImGuiCol_HeaderHovered] = ImVec4(0.165f, 0.239f, 0.322f, 1.0f);
    c[ImGuiCol_HeaderActive] = ImVec4(0.149f, 0.310f, 0.471f, 1.0f);
    c[ImGuiCol_Separator] = grey(0x3c);
    c[ImGuiCol_ScrollbarBg] = ImVec4(0, 0, 0, 0);
    c[ImGuiCol_ScrollbarGrab] = grey(0x4a);
    c[ImGuiCol_ScrollbarGrabHovered] = grey(0x5e);
    c[ImGuiCol_ScrollbarGrabActive] = grey(0x72);
    c[ImGuiCol_TextSelectedBg] = ImVec4(0.149f, 0.310f, 0.471f, 0.8f);
    c[ImGuiCol_NavCursor] = ImVec4(0, 0, 0, 0);
}

/* Build the atlas and its texture. It is rebuilt when the output scale
 * changes so text stays sharp on a scaled monitor. */
static void gnoblin_imgui_upload_font(GnoblinImGuiOverlay* overlay, CoglFramebuffer* framebuffer,
                                      float scale) {
    if (overlay->font_texture != nullptr && overlay->font_scale == scale)
        return;

    ImGui::SetCurrentContext(overlay->context);
    ImGuiIO& io = ImGui::GetIO();
    if (overlay->font_texture != nullptr) {
        io.Fonts->Clear();
        g_clear_object(&overlay->font_texture);
    }
    gnoblin_imgui_load_system_font(io, scale);
    unsigned char* pixels = nullptr;
    int width = 0;
    int height = 0;
    io.Fonts->GetTexDataAsRGBA32(&pixels, &width, &height);
    /* Unhinted coverage reads thin on a dark background. Lift it a little. */
    static const struct Lift {
        unsigned char table[256];
        Lift() {
            for (int i = 0; i < 256; i++)
                table[i] = static_cast<unsigned char>(
                    std::lround(255.0 * std::pow(static_cast<double>(i) / 255.0, 0.72)));
        }
    } lift;
    for (int i = 0; i < width * height; i++)
        pixels[i * 4 + 3] = lift.table[pixels[i * 4 + 3]];
    overlay->font_texture = COGL_TEXTURE(
        cogl_texture_2d_new_from_data(cogl_framebuffer_get_context(framebuffer), width, height,
                                      COGL_PIXEL_FORMAT_RGBA_8888, width * 4, pixels, nullptr));
    io.Fonts->SetTexID(overlay->font_texture);
    overlay->font_scale = scale;
}

static gboolean gnoblin_imgui_input_tick(gpointer user_data) {
    auto* overlay = static_cast<GnoblinImGuiOverlay*>(user_data);
    overlay->input_tick_id = 0;
    gnoblin_imgui_overlay_queue_redraw(overlay);
    return G_SOURCE_REMOVE;
}

static void gnoblin_imgui_content_paint(ClutterContent* content, ClutterActor* actor,
                                        ClutterPaintNode* node,
                                        ClutterPaintContext* paint_context) {
    (void)node;
    GnoblinImGuiOverlay* overlay = reinterpret_cast<GnoblinImGuiContent*>(content)->overlay;
    if (overlay == nullptr || !clutter_actor_is_visible(actor) || overlay->draw_callback == nullptr)
        return;

    float width = 0.0f;
    float height = 0.0f;
    clutter_actor_get_size(actor, &width, &height);
    if (width <= 0.0f || height <= 0.0f)
        return;

    CoglFramebuffer* framebuffer = clutter_paint_context_get_framebuffer(paint_context);
    const float scale = std::clamp(gnoblin_plugin_paint_scale(paint_context), 1.0f, 4.0f);
    gnoblin_imgui_upload_font(overlay, framebuffer, scale);

    ImGui::SetCurrentContext(overlay->context);
    ImGuiIO& io = ImGui::GetIO();
    io.DisplaySize = ImVec2(width, height);
    io.DisplayFramebufferScale = ImVec2(1.0f, 1.0f);
    ImGui::NewFrame();
    overlay->draw_callback(overlay, overlay->draw_user_data);
    ImGui::Render();
    /* The pointer shape comes from Gnoblin's cursor system, which resolves it from the actor under
     * the pointer: map what ImGui asks for (a text cursor over an input) onto the overlay actor. */
    {
        ClutterCursorType cursor = CLUTTER_CURSOR_DEFAULT;
        switch (ImGui::GetMouseCursor()) {
        case ImGuiMouseCursor_TextInput:
            cursor = CLUTTER_CURSOR_TEXT;
            break;
        case ImGuiMouseCursor_Hand:
            cursor = CLUTTER_CURSOR_POINTER;
            break;
        case ImGuiMouseCursor_ResizeEW:
            cursor = CLUTTER_CURSOR_EW_RESIZE;
            break;
        case ImGuiMouseCursor_ResizeNS:
            cursor = CLUTTER_CURSOR_NS_RESIZE;
            break;
        case ImGuiMouseCursor_NotAllowed:
            cursor = CLUTTER_CURSOR_NOT_ALLOWED;
            break;
        default:
            break;
        }
        if (clutter_actor_get_cursor_type(actor) != cursor)
            clutter_actor_set_cursor_type(actor, cursor);
    }
    /* ImGui trickles a quick press/release across two frames. Ensure the release
     * is processed even when both input events preceded one compositor paint. */
    /* ImGui also trickles queued keys and characters one event per frame. A
     * fast typist can queue several before the next paint, and with nothing
     * else invalidating the overlay the remainder would sit unprocessed until
     * the next keypress, which drops or reorders text. Keep painting until the
     * queue drains. */
    if (!overlay->input_tick_id && (io.MouseDown[0] || io.MouseDown[1] || io.MouseDown[2] ||
                                    ImGui::GetCurrentContext()->InputEventsQueue.Size > 0))
        overlay->input_tick_id = g_timeout_add(16, gnoblin_imgui_input_tick, overlay);
    gnoblin_imgui_render_draw_data(overlay, framebuffer, ImGui::GetDrawData());
}

static void gnoblin_imgui_content_interface_init(ClutterContentInterface* iface) {
    iface->paint_content = gnoblin_imgui_content_paint;
}

GnoblinImGuiOverlay* gnoblin_imgui_overlay_new(ClutterActor* stage) {
    GnoblinImGuiOverlay* overlay = g_new0(GnoblinImGuiOverlay, 1);
    overlay->actor = clutter_actor_new();
    clutter_actor_set_reactive(overlay->actor, TRUE);
    clutter_actor_set_x_expand(overlay->actor, TRUE);
    clutter_actor_set_y_expand(overlay->actor, TRUE);
    overlay->content = CLUTTER_CONTENT(g_object_new(gnoblin_imgui_content_get_type(), nullptr));
    reinterpret_cast<GnoblinImGuiContent*>(overlay->content)->overlay = overlay;
    clutter_actor_set_content(overlay->actor, overlay->content);
    overlay->context = ImGui::CreateContext();
    ImGui::SetCurrentContext(overlay->context);
    ImGui::GetIO().BackendRendererName = "gnoblin-cogl";
    ImGui::GetIO().BackendPlatformName = "gnoblin-clutter";
    ImGui::GetIO().BackendFlags |= ImGuiBackendFlags_RendererHasVtxOffset;
    /* No imgui.ini or log file: panels keep no layout state between runs. */
    ImGui::GetIO().IniFilename = nullptr;
    /* A misuse of the ImGui API in a panel must never take the compositor down. Recoverable errors
     * are repaired and logged instead of aborting, and no error tooltip is drawn. */
    ImGui::GetIO().ConfigErrorRecovery = true;
    ImGui::GetIO().ConfigErrorRecoveryEnableAssert = false;
    ImGui::GetIO().ConfigErrorRecoveryEnableTooltip = false;
    ImGui::GetIO().LogFilename = nullptr;
    gnoblin_imgui_apply_style();
    if (stage != nullptr) {
        g_return_val_if_fail(CLUTTER_IS_STAGE(stage), overlay);
        /* Keep the diagnostics actor directly on the stage. Window groups can
         * contain layer-shell overlay clients, which must not obscure recovery or
         * the console. */
        clutter_actor_add_constraint(overlay->actor,
                                     clutter_bind_constraint_new(stage, CLUTTER_BIND_SIZE, 0.0f));
        clutter_actor_add_child(stage, overlay->actor);
    }
    return overlay;
}

void gnoblin_imgui_overlay_free(GnoblinImGuiOverlay* overlay) {
    if (overlay == nullptr)
        return;
    if (overlay->input_tick_id)
        g_source_remove(overlay->input_tick_id);
    if (overlay->draw_destroy != nullptr)
        overlay->draw_destroy(overlay->draw_user_data);
    if (overlay->font_texture != nullptr)
        g_object_unref(overlay->font_texture);
    if (overlay->context != nullptr)
        ImGui::DestroyContext(overlay->context);
    g_clear_object(&overlay->content);
    g_clear_pointer(&overlay->actor, clutter_actor_destroy);
    g_free(overlay);
}

ClutterActor* gnoblin_imgui_overlay_get_actor(GnoblinImGuiOverlay* overlay) {
    return overlay != nullptr ? overlay->actor : nullptr;
}

ImGuiContext* gnoblin_imgui_overlay_get_context(GnoblinImGuiOverlay* overlay) {
    return overlay != nullptr ? overlay->context : nullptr;
}

void gnoblin_imgui_overlay_set_draw_callback(GnoblinImGuiOverlay* overlay,
                                             GnoblinImGuiOverlayDrawFunc callback, void* user_data,
                                             GDestroyNotify destroy) {
    if (overlay->draw_destroy != nullptr)
        overlay->draw_destroy(overlay->draw_user_data);
    overlay->draw_callback = callback;
    overlay->draw_user_data = user_data;
    overlay->draw_destroy = destroy;
    gnoblin_imgui_overlay_queue_redraw(overlay);
}

void gnoblin_imgui_overlay_set_visible(GnoblinImGuiOverlay* overlay, gboolean visible) {
    if (overlay == nullptr)
        return;
    if (visible) {
        clutter_actor_show(overlay->actor);
        gnoblin_imgui_overlay_raise(overlay);
    } else
        clutter_actor_hide(overlay->actor);
}

gboolean gnoblin_imgui_overlay_is_visible(GnoblinImGuiOverlay* overlay) {
    return overlay != nullptr && clutter_actor_is_visible(overlay->actor);
}

void gnoblin_imgui_overlay_raise(GnoblinImGuiOverlay* overlay) {
    if (overlay == nullptr || !clutter_actor_is_visible(overlay->actor))
        return;

    ClutterActor* parent = clutter_actor_get_parent(overlay->actor);
    if (parent != nullptr)
        clutter_actor_set_child_above_sibling(parent, overlay->actor, nullptr);
}

void gnoblin_imgui_overlay_queue_redraw(GnoblinImGuiOverlay* overlay) {
    if (overlay != nullptr) {
        gnoblin_imgui_overlay_raise(overlay);
        clutter_content_invalidate(overlay->content);
    }
}

void gnoblin_imgui_overlay_set_pointer(GnoblinImGuiOverlay* overlay, float x, float y,
                                       gboolean left_down, gboolean right_down,
                                       gboolean middle_down) {
    if (overlay == nullptr)
        return;
    ImGui::SetCurrentContext(overlay->context);
    ImGuiIO& io = ImGui::GetIO();
    io.AddMousePosEvent(x, y);
    io.AddMouseButtonEvent(0, left_down);
    io.AddMouseButtonEvent(1, right_down);
    io.AddMouseButtonEvent(2, middle_down);
    gnoblin_imgui_overlay_queue_redraw(overlay);
}

void gnoblin_imgui_overlay_add_scroll(GnoblinImGuiOverlay* overlay, float dx, float dy) {
    if (overlay == nullptr)
        return;
    ImGui::SetCurrentContext(overlay->context);
    ImGui::GetIO().AddMouseWheelEvent(dx, dy);
    gnoblin_imgui_overlay_queue_redraw(overlay);
}

void gnoblin_imgui_overlay_add_text(GnoblinImGuiOverlay* overlay, const char* utf8) {
    if (overlay == nullptr || utf8 == nullptr)
        return;
    ImGui::SetCurrentContext(overlay->context);
    ImGui::GetIO().AddInputCharactersUTF8(utf8);
    gnoblin_imgui_overlay_queue_redraw(overlay);
}
