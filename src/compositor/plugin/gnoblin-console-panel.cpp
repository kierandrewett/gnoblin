/* Compositor-owned ImGui developer console; see gnoblin-console-panel.hpp. */

#include "config.h"

#include "gnoblin-console-panel.hpp"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <utility>

#include "imgui.h"
#include "imgui_internal.h"

namespace {
constexpr std::size_t kMaxRows = 200;
constexpr std::size_t kInputCapacity = 16 * 1024;

std::string console_help() {
    return "Lua runtime console\n"
           "Runs a bounded Lua expression in the active configuration runtime.\n"
           "The configuration document is read-only here. Edit init.lua, then use :reload.\n"
           "Queued runtime operations are submitted through Gnoblin's normal API.\n\n"
           ":help  :reload";
}

ImGuiKey imgui_key_for_clutter(guint keyval) {
    switch (keyval) {
    case CLUTTER_KEY_BackSpace:
        return ImGuiKey_Backspace;
    case CLUTTER_KEY_Delete:
        return ImGuiKey_Delete;
    case CLUTTER_KEY_Left:
        return ImGuiKey_LeftArrow;
    case CLUTTER_KEY_Right:
        return ImGuiKey_RightArrow;
    case CLUTTER_KEY_Home:
        return ImGuiKey_Home;
    case CLUTTER_KEY_End:
        return ImGuiKey_End;
    case CLUTTER_KEY_Page_Up:
        return ImGuiKey_PageUp;
    case CLUTTER_KEY_Page_Down:
        return ImGuiKey_PageDown;
    default:
        return ImGuiKey_None;
    }
}
} // namespace

struct GnoblinConsolePanel::Row {
    std::string source;
    std::string text;
    std::string error;
    std::vector<GnoblinConsoleInspectorNode> inspection;
    bool pending = false;
    bool log = false;
};

GnoblinConsolePanel::GnoblinConsolePanel(GnoblinImGuiOverlay* overlay,
                                         GnoblinConsoleCallbacks callbacks)
    : overlay_(overlay), callbacks_(std::move(callbacks)), input_(kInputCapacity, '\0') {
    if (callbacks_.load_history)
        history_ = callbacks_.load_history();
    if (history_.size() > kMaxRows)
        history_.erase(history_.begin(), history_.end() - kMaxRows);
    history_index_ = history_.size();
}

GnoblinConsolePanel::~GnoblinConsolePanel() {
    completion_generation_.reset();
}

void GnoblinConsolePanel::set_visible(bool visible) {
    if (visible_ == visible)
        return;
    visible_ = visible;
    {
        /* Modifiers are tracked from key events; start each session without any held. */
        ImGui::SetCurrentContext(gnoblin_imgui_overlay_get_context(overlay_));
        ImGuiIO& io = ImGui::GetIO();
        io.AddKeyEvent(ImGuiMod_Ctrl, false);
        io.AddKeyEvent(ImGuiMod_Shift, false);
        io.AddKeyEvent(ImGuiMod_Alt, false);
    }
    if (!visible) {
        ++*completion_generation_;
        completion_visible_ = false;
    }
    if (visible) {
        scroll_to_bottom_ = true;
        focus_input_ = true;
        completion_requested_ = input_.data();
        preview_requested_ = input_.data();
    }
    ImGui::GetIO().MouseDrawCursor = visible;
    if (callbacks_.visibility_changed)
        callbacks_.visibility_changed();
    queue_redraw();
}

bool GnoblinConsolePanel::visible() const {
    return visible_;
}

void GnoblinConsolePanel::toggle() {
    set_visible(!visible());
}

void GnoblinConsolePanel::clear() {
    rows_.clear();
    scroll_to_bottom_ = true;
    queue_redraw();
}

void GnoblinConsolePanel::draw_frame(GnoblinImGuiOverlay* overlay) {
    if (overlay == overlay_)
        draw();
}

void GnoblinConsolePanel::add_text(const char* utf8) {
    if (visible())
        gnoblin_imgui_overlay_add_text(overlay_, utf8);
}

bool GnoblinConsolePanel::handle_key(guint keyval, ClutterModifierType modifiers, bool pressed,
                                     bool repeated) {
    if (!visible())
        return false;

    ImGui::SetCurrentContext(gnoblin_imgui_overlay_get_context(overlay_));
    ImGuiIO& io = ImGui::GetIO();
    /* A key event carries the modifiers held before it, so the release of a modifier still shows
     * it as held, and its press does not. Left stale, ImGui ignores clicks on result rows (it
     * refuses them while a modifier is down) after the console is opened with Alt+F2. */
    {
        const guint mask = [keyval]() -> guint {
            switch (keyval) {
            case CLUTTER_KEY_Shift_L:
            case CLUTTER_KEY_Shift_R:
                return CLUTTER_SHIFT_MASK;
            case CLUTTER_KEY_Control_L:
            case CLUTTER_KEY_Control_R:
                return CLUTTER_CONTROL_MASK;
            case CLUTTER_KEY_Alt_L:
            case CLUTTER_KEY_Alt_R:
            case CLUTTER_KEY_Meta_L:
            case CLUTTER_KEY_Meta_R:
                return CLUTTER_MOD1_MASK;
            default:
                return 0;
            }
        }();
        modifiers =
            static_cast<ClutterModifierType>(pressed ? (modifiers | mask) : (modifiers & ~mask));
    }
    io.AddKeyEvent(ImGuiMod_Ctrl, (modifiers & CLUTTER_CONTROL_MASK) != 0);
    io.AddKeyEvent(ImGuiMod_Shift, (modifiers & CLUTTER_SHIFT_MASK) != 0);
    io.AddKeyEvent(ImGuiMod_Alt, (modifiers & CLUTTER_MOD1_MASK) != 0);
    const ImGuiKey imgui_key = imgui_key_for_clutter(keyval);
    if (!pressed) {
        if (imgui_key != ImGuiKey_None)
            io.AddKeyEvent(imgui_key, false);
        queue_redraw();
        return true;
    }

    /* Alt+F2 opens the console, so it closes it too. A held key must not close it at once. */
    if (keyval == CLUTTER_KEY_F2 && (modifiers & CLUTTER_MOD1_MASK) != 0) {
        if (!repeated)
            set_visible(false);
        queue_redraw();
        return true;
    }
    if (keyval == CLUTTER_KEY_Page_Up || keyval == CLUTTER_KEY_Page_Down) {
        scroll_request_pages_ += keyval == CLUTTER_KEY_Page_Up ? -1 : 1;
        queue_redraw();
        return true;
    }
    if (keyval == CLUTTER_KEY_Escape) {
        if (completion_visible_)
            completion_visible_ = false;
        else
            set_visible(false);
        queue_redraw();
        return true;
    }
    if (keyval == CLUTTER_KEY_Return || keyval == CLUTTER_KEY_KP_Enter) {
        if ((modifiers & CLUTTER_SHIFT_MASK) != 0) {
            /* Enter is intercepted for submission, so pass Shift+Enter as text
             * instead of relying on InputTextMultiline observing a key event. */
            gnoblin_imgui_overlay_add_text(overlay_, "\n");
            queue_redraw();
            return true;
        }
        enter_requested_ = true;
        queue_redraw();
        return true;
    }

    if (imgui_key != ImGuiKey_None)
        io.AddKeyEvent(imgui_key, true);
    if (keyval == CLUTTER_KEY_Tab) {
        tab_requested_ = true;
        queue_redraw();
        return true;
    }
    if (keyval == CLUTTER_KEY_Up || keyval == CLUTTER_KEY_Down) {
        if (completion_visible_) {
            const int count = static_cast<int>(completions_.size());
            if (count > 0)
                completion_index_ =
                    (completion_index_ + (keyval == CLUTTER_KEY_Up ? count - 1 : 1)) % count;
            completion_scroll_ = true;
            completion_chosen_ = true;
        } else if (std::strchr(input_.data(), '\n') == nullptr ||
                   (modifiers & CLUTTER_MOD1_MASK) != 0)
            history_step(keyval == CLUTTER_KEY_Up ? -1 : 1);
        queue_redraw();
        return true;
    }
    if ((modifiers & CLUTTER_CONTROL_MASK) != 0 &&
        (keyval == CLUTTER_KEY_l || keyval == CLUTTER_KEY_L)) {
        clear();
        return true;
    }
    queue_redraw();
    return true;
}

void GnoblinConsolePanel::append_log(std::string_view text, bool error) {
    Row row;
    row.text = text;
    row.log = true;
    if (error)
        row.error = row.text;
    rows_.push_back(std::move(row));
    if (rows_.size() > kMaxRows)
        rows_.erase(rows_.begin());
    scroll_to_bottom_ = scroll_to_bottom_ || at_bottom_;
    queue_redraw();
}

namespace {
constexpr float kInset = 10.0f;
constexpr float kRowPadding = 5.0f;
constexpr float kPanelFraction = 0.5f;
constexpr int kMaxInputLines = 8;

ImVec4 rgb(unsigned hex, float alpha = 1.0f) {
    return ImVec4(((hex >> 16) & 0xff) / 255.0f, ((hex >> 8) & 0xff) / 255.0f,
                  (hex & 0xff) / 255.0f, alpha);
}

const ImVec4 kPromptColour = rgb(0x569cd6);
const ImVec4 kKeyColour = rgb(0xc586c0);
const ImVec4 kErrorColour = rgb(0xff8080);
const ImVec4 kErrorFill = rgb(0x290000);
const ImVec4 kErrorRule = rgb(0x5c0000);
const ImVec4 kQuietColour = rgb(0xb0b0b0);
/* Values are coloured by kind, as in a browser console. */
const ImVec4 kStringColour = rgb(0xe9a07a);
const ImVec4 kNumberColour = rgb(0xa6d189);
const ImVec4 kKeywordColour = rgb(0x6cb6ff);
const ImVec4 kFunctionColour = rgb(0x6fd7c8);

/* Every expandable row, and every leaf, highlights across the full width on hover. */
constexpr ImGuiTreeNodeFlags kTreeFlags = ImGuiTreeNodeFlags_SpanFullWidth;
constexpr ImGuiTreeNodeFlags kLeafFlags = ImGuiTreeNodeFlags_SpanFullWidth |
                                          ImGuiTreeNodeFlags_Leaf |
                                          ImGuiTreeNodeFlags_NoTreePushOnOpen;

bool starts_with(const std::string& text, const char* prefix) {
    return text.compare(0, std::strlen(prefix), prefix) == 0;
}

ImVec4 value_colour(const std::string& value) {
    if (value.empty())
        return kQuietColour;
    if (value[0] == '"' || value[0] == '\'')
        return kStringColour;
    if (value == "true" || value == "false")
        return kKeywordColour;
    if (value == "nil")
        return kQuietColour;
    if (starts_with(value, "<function") || starts_with(value, "<cfunction"))
        return kFunctionColour;
    if ((value[0] >= '0' && value[0] <= '9') || value[0] == '-')
        return kNumberColour;
    return ImGui::GetStyleColorVec4(ImGuiCol_Text);
}

std::string plain_key(const std::string& label) {
    if (label.size() >= 2 && label.front() == '"' && label.back() == '"')
        return label.substr(1, label.size() - 2);
    return label;
}

/* A table whose keys are 1..n in order is an array. */
bool is_array(const std::vector<GnoblinConsoleInspectorNode>& children) {
    std::size_t expected = 1;
    for (const auto& child : children) {
        if (child.label.empty() && child.has_more)
            continue;
        if (child.label != std::to_string(expected++))
            return false;
    }
    return expected > 1;
}

std::string table_preview(const std::vector<GnoblinConsoleInspectorNode>& children, bool more,
                          int depth);

std::string value_preview(const GnoblinConsoleInspectorNode& node, int depth) {
    if (!node.children.empty() || starts_with(node.value, "<table"))
        return table_preview(node.children, node.has_more, depth);
    if (starts_with(node.value, "<function") || starts_with(node.value, "<cfunction"))
        return "fn";
    return node.value;
}

/* A short inline summary, like a browser's "Object { a: 1, b: {…} }" and "[1, 2, 3]". Entries past
 * the cap are a childless node with no label. */
std::string table_preview(const std::vector<GnoblinConsoleInspectorNode>& children, bool more,
                          int depth) {
    std::vector<const GnoblinConsoleInspectorNode*> entries;
    for (const auto& child : children) {
        if (child.label.empty() && child.has_more)
            more = true;
        else
            entries.push_back(&child);
    }
    const bool array = is_array(children);
    const char* open = array ? "[" : "{";
    const char* close = array ? "]" : "}";
    if (entries.empty())
        return more ? std::string(open) + "…" + close : std::string(open) + close;
    if (depth > 0)
        return std::string(open) + "…" + close;
    std::string out = std::string(open) + " ";
    std::size_t shown = 0;
    for (const auto* child : entries) {
        if (shown == 5)
            break;
        if (shown > 0)
            out += ", ";
        out += array ? value_preview(*child, depth + 1)
                     : plain_key(child->label) + ": " + value_preview(*child, depth + 1);
        shown++;
    }
    if (shown < entries.size() || more)
        out += ", …";
    return out + " " + close;
}

/* True for text that only walks a table by name, such as "gnoblin.windows": evaluating it cannot
 * run anything, so the console shows its value as a preview while it is typed. */
bool is_plain_path(const std::string& text) {
    if (text.empty() || text.size() > 200)
        return false;
    bool segment_start = true;
    for (char c : text) {
        if (c == '.') {
            if (segment_start)
                return false;
            segment_start = true;
        } else if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '_' ||
                   (!segment_start && c >= '0' && c <= '9')) {
            segment_start = false;
        } else {
            return false;
        }
    }
    return !segment_start;
}

/* An error spans the full width on a dark red band, as in Chrome's console. */
void error_row(const char* text) {
    ImDrawList* list = ImGui::GetWindowDrawList();
    const ImVec2 origin = ImGui::GetCursorScreenPos();
    const ImVec2 window = ImGui::GetWindowPos();
    const float width = ImGui::GetWindowWidth();
    const ImVec2 size = ImGui::CalcTextSize(text, nullptr, false, ImGui::GetContentRegionAvail().x);
    const float top = origin.y - 2.0f;
    const float bottom = origin.y + size.y + 2.0f;
    list->AddRectFilled(ImVec2(window.x, top), ImVec2(window.x + width, bottom),
                        ImGui::GetColorU32(kErrorFill));
    list->AddLine(ImVec2(window.x, top), ImVec2(window.x + width, top),
                  ImGui::GetColorU32(kErrorRule));
    list->AddLine(ImVec2(window.x, bottom), ImVec2(window.x + width, bottom),
                  ImGui::GetColorU32(kErrorRule));
    ImGui::PushStyleColor(ImGuiCol_Text, kErrorColour);
    ImGui::TextWrapped("%s", text);
    ImGui::PopStyleColor();
    ImGui::Dummy(ImVec2(0.0f, 2.0f));
}

/* A line led by a one-character marker. */
void marked_row(const char* marker, const ImVec4& marker_colour, const char* text,
                const ImVec4* text_colour = nullptr) {
    ImGui::TextColored(marker_colour, "%s", marker);
    ImGui::SameLine(0.0f, 8.0f);
    if (text_colour != nullptr)
        ImGui::PushStyleColor(ImGuiCol_Text, *text_colour);
    ImGui::TextWrapped("%s", text);
    if (text_colour != nullptr)
        ImGui::PopStyleColor();
}
} // namespace

void GnoblinConsolePanel::draw() {
    if (!visible())
        return;

    /* A drop-down over the top of the screen, above everything including the
     * shell's own surfaces. */
    const ImGuiViewport* viewport = ImGui::GetMainViewport();
    const float width = viewport->WorkSize.x;
    const float height = std::floor(viewport->WorkSize.y * kPanelFraction);
    ImGui::SetNextWindowPos(viewport->WorkPos);
    ImGui::SetNextWindowSize(ImVec2(width, height));
    constexpr ImGuiWindowFlags flags =
        ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoMove |
        ImGuiWindowFlags_NoResize | ImGuiWindowFlags_NoSavedSettings |
        ImGuiWindowFlags_NoBringToFrontOnFocus | ImGuiWindowFlags_NoScrollbar |
        ImGuiWindowFlags_NoScrollWithMouse;
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0.0f, 0.0f));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.0f);
    const bool open = ImGui::Begin("Lua console", nullptr, flags);
    ImGui::PopStyleVar(2);
    if (!open) {
        ImGui::End();
        return;
    }

    ImDrawList* list = ImGui::GetWindowDrawList();
    const ImVec2 origin = ImGui::GetWindowPos();
    const ImU32 line_colour = ImGui::GetColorU32(ImGuiCol_Border);

    int lines = 1;
    for (const char* c = input_.data(); *c != '\0'; c++)
        if (*c == '\n')
            lines++;
    lines = std::min(lines, kMaxInputLines);
    const float input_height = lines * ImGui::GetTextLineHeight() + kRowPadding * 2;
    /* A greyed line under the input shows the value of what is typed, when that is safe to know. */
    const float preview_height =
        preview_text_.empty() ? 0.0f : ImGui::GetTextLineHeight() + kRowPadding;
    const float input_top = height - input_height - preview_height;

    /* Transcript. */
    ImGui::SetCursorPos(ImVec2(0.0f, 0.0f));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(kInset, kRowPadding));
    if (ImGui::BeginChild("##transcript", ImVec2(width, input_top - 1.0f),
                          ImGuiChildFlags_AlwaysUseWindowPadding)) {
        /* Output grows upwards from the input line: while it is shorter than the pane, push it
         * down. The height comes from the previous frame; a change draws one more frame. */
        const float pane_height = ImGui::GetContentRegionAvail().y;
        at_bottom_ = ImGui::GetScrollY() >= ImGui::GetScrollMaxY() - 2.0f;
        if (scroll_request_pages_ != 0) {
            ImGui::SetScrollY(ImGui::GetScrollY() + scroll_request_pages_ * pane_height * 0.9f);
            scroll_request_pages_ = 0;
        }
        const float spacer =
            std::max(0.0f, pane_height - transcript_height_ - ImGui::GetStyle().ItemSpacing.y);
        if (spacer > 0.0f)
            ImGui::Dummy(ImVec2(0.0f, spacer));
        const float content_top = ImGui::GetCursorPosY();
        ImDrawList* transcript_list = ImGui::GetWindowDrawList();
        const int first_vertex = transcript_list->VtxBuffer.Size;
        for (std::size_t index = 0; index < rows_.size(); index++) {
            const Row& row = rows_[index];
            ImGui::PushID(static_cast<int>(index));
            if (row.log) {
                if (!row.error.empty())
                    error_row(row.error.c_str());
                else
                    marked_row(" ", kQuietColour, row.text.c_str(), &kQuietColour);
            } else {
                marked_row(">", kPromptColour, row.source.c_str());
                if (row.pending)
                    marked_row("<", kQuietColour, "...", &kQuietColour);
                else if (!row.error.empty())
                    error_row(row.error.c_str());
                else if (!row.text.empty() && !row.inspection.empty() &&
                         row.text.find('\n') == std::string::npos) {
                    /* A result that can be expanded starts collapsed, as in a browser console. */
                    ImGui::TextColored(kQuietColour, "<");
                    ImGui::SameLine(0.0f, 8.0f);
                    const std::string summary = table_preview(row.inspection, false, 0);
                    const bool open = ImGui::TreeNodeEx("##result", kTreeFlags, "");
                    ImGui::SameLine(0.0f, 0.0f);
                    ImGui::TextColored(kKeywordColour, "table ");
                    ImGui::SameLine(0.0f, 0.0f);
                    ImGui::TextColored(kQuietColour, "%s", summary.c_str());
                    if (open) {
                        for (std::size_t child = 0; child < row.inspection.size(); child++)
                            draw_node(row.inspection[child],
                                      std::to_string(index) + ":" + std::to_string(child));
                        ImGui::TreePop();
                    }
                } else {
                    if (!row.text.empty())
                        marked_row("<", kQuietColour, row.text.c_str());
                    for (std::size_t child = 0; child < row.inspection.size(); child++) {
                        ImGui::Indent(16.0f);
                        draw_node(row.inspection[child],
                                  std::to_string(index) + ":" + std::to_string(child));
                        ImGui::Unindent(16.0f);
                    }
                }
            }
            ImGui::PopID();
        }
        const float measured = ImGui::GetCursorPosY() - content_top;
        if (std::fabs(measured - transcript_height_) > 0.5f) {
            /* The spacer above used last frame's height. Move what was just drawn to where it
             * belongs now, so a new row does not show for one frame in the wrong place; layout
             * catches up on the next frame. */
            const float wanted =
                std::max(0.0f, pane_height - measured - ImGui::GetStyle().ItemSpacing.y);
            const float shift = wanted - spacer;
            for (int vertex = first_vertex; vertex < transcript_list->VtxBuffer.Size; vertex++)
                transcript_list->VtxBuffer[vertex].pos.y += shift;
            transcript_height_ = measured;
            queue_redraw();
        }
        if (scroll_to_bottom_) {
            /* Only when the content really overflows; the spacer above used last frame's height and
             * can overflow for a frame without the content being too tall. */
            if (measured > pane_height)
                ImGui::SetScrollHereY(1.0f);
            scroll_to_bottom_ = false;
        }
    }
    ImGui::EndChild();
    ImGui::PopStyleVar();

    /* Input line. */
    list->AddLine(ImVec2(origin.x, origin.y + input_top - 1.0f),
                  ImVec2(origin.x + width, origin.y + input_top - 1.0f), line_colour);
    ImGui::SetCursorPos(ImVec2(kInset, input_top + kRowPadding));
    ImGui::TextColored(kPromptColour, ">");
    ImGui::SameLine(0.0f, 8.0f);
    ImGui::SetCursorPosY(input_top);

    if (reload_input_) {
        if (ImGuiInputTextState* state = ImGui::GetInputTextState(ImGui::GetID("##input")))
            state->ReloadUserBufAndMoveToEnd();
        reload_input_ = false;
    }
    /* Typing always goes to the input: clicking a result must not leave the console deaf. */
    if (!ImGui::IsAnyItemActive() && !ImGui::IsMouseDown(0))
        focus_input_ = true;
    if (focus_input_) {
        ImGui::SetKeyboardFocusHere();
        focus_input_ = false;
    }
    ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(0.0f, kRowPadding));
    ImGui::PushStyleVar(ImGuiStyleVar_FrameBorderSize, 0.0f);
    ImGui::InputTextMultiline("##input", input_.data(), input_.size(),
                              ImVec2(ImGui::GetContentRegionAvail().x - kInset, input_height),
                              ImGuiInputTextFlags_WordWrap);
    ImGui::PopStyleVar(2);
    input_anchor_x_ = ImGui::GetItemRectMin().x;
    input_anchor_y_ = ImGui::GetItemRectMax().y + preview_height;
    if (preview_height > 0.0f) {
        ImGui::SetCursorPos(ImVec2(kInset, input_top + input_height));
        ImGui::TextColored(kQuietColour, "<");
        ImGui::SameLine(0.0f, 8.0f);
        ImGui::TextColored(kQuietColour, "%s", preview_text_.c_str());
    }
    input_width_ = ImGui::GetItemRectSize().x;

    if ((enter_requested_ || tab_requested_) &&
        ImGui::GetCurrentContext()->InputEventsQueue.Size == 0) {
        const bool enter = enter_requested_;
        const bool tab = tab_requested_;
        enter_requested_ = tab_requested_ = false;
        /* Enter submits what was typed unless a suggestion was picked from the list; Tab takes the
         * highlighted suggestion. */
        if (completion_visible_ && completion_source_ == input_.data() &&
            (tab || completion_chosen_))
            accept_completion();
        else if (enter)
            submit();
        else
            complete();
    }
    if (completion_visible_ && completion_source_ != input_.data())
        completion_visible_ = false;
    /* Suggest as the user types: once every queued character is applied, ask for completions of
     * the text as it now stands. */
    if (ImGui::GetCurrentContext()->InputEventsQueue.Size == 0 &&
        completion_requested_ != input_.data()) {
        completion_requested_ = input_.data();
        if (completion_requested_.empty()) {
            completion_visible_ = false;
            ++*completion_generation_;
        } else
            complete();
    }
    if (ImGui::GetCurrentContext()->InputEventsQueue.Size == 0 &&
        preview_requested_ != input_.data()) {
        preview_requested_ = input_.data();
        request_preview();
    }
    ImGui::End();

    if (completion_visible_)
        draw_completions();
}

/* Matches the input line: a small list above it, no decoration. */
void GnoblinConsolePanel::draw_completions() {
    /* Below the input line, over the desktop under the console. */
    const float line = ImGui::GetTextLineHeightWithSpacing();
    const float room = ImGui::GetMainViewport()->WorkSize.y - input_anchor_y_ - 8.0f;
    ImGui::SetNextWindowPos(ImVec2(input_anchor_x_ - 4.0f, input_anchor_y_ + 2.0f),
                            ImGuiCond_Always, ImVec2(0.0f, 0.0f));
    ImGui::SetNextWindowSizeConstraints(
        ImVec2(std::min(360.0f, input_width_), line * 5 + 8.0f),
        ImVec2(std::max(input_width_ * 0.5f, 360.0f), std::min(line * 12 + 8.0f, room)));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(2.0f, 2.0f));
    ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(0.0f, 0.0f));
    constexpr ImGuiWindowFlags flags =
        ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings |
        ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoFocusOnAppearing;
    if (ImGui::Begin("##console-completions", nullptr, flags)) {
        for (std::size_t i = 0; i < completions_.size(); i++) {
            const bool selected = static_cast<int>(i) == completion_index_;
            if (ImGui::Selectable(completions_[i].label.c_str(), selected)) {
                completion_index_ = static_cast<int>(i);
                accept_completion();
            }
            if (selected && completion_scroll_)
                ImGui::SetScrollHereY();
        }
        completion_scroll_ = false;
    }
    ImGui::End();
    ImGui::PopStyleVar(2);
}

void GnoblinConsolePanel::submit() {
    if (pending_)
        return;
    std::string source(input_.data());
    const auto first = source.find_first_not_of(" \t\n\r");
    if (first == std::string::npos)
        return;
    source.erase(0, first);
    const auto last = source.find_last_not_of(" \t\n\r");
    source.erase(last + 1);
    append_history(source);
    std::fill(input_.begin(), input_.end(), '\0');
    reload_input_ = true;
    focus_input_ = true;
    completion_visible_ = false;
    preview_text_.clear();
    if (source == ":help") {
        append_result(source, {true, console_help(), {}, {}});
        return;
    }
    if (source == ":reload") {
        submit_control(GnoblinConsoleControl::ReloadConfiguration, source);
        return;
    }
    if (!callbacks_.evaluate) {
        append_result(
            source,
            {false, {}, "No console evaluator is attached to this compositor session.", {}});
        return;
    }
    Row row;
    row.source = source;
    row.pending = true;
    rows_.push_back(std::move(row));
    if (rows_.size() > kMaxRows)
        rows_.erase(rows_.begin());
    pending_ = true;
    scroll_to_bottom_ = true;
    callbacks_.evaluate(source, [this, source](GnoblinConsoleResult result) {
        pending_ = false;
        append_result(source, std::move(result));
    });
    queue_redraw();
}

void GnoblinConsolePanel::submit_control(GnoblinConsoleControl control, std::string_view source) {
    if (!callbacks_.control) {
        append_result(std::string(source),
                      {false, {}, "No live configuration controller is attached.", {}});
        return;
    }
    pending_ = true;
    callbacks_.control(control, [this, source = std::string(source)](GnoblinConsoleResult result) {
        pending_ = false;
        append_result(std::move(source), std::move(result));
    });
    queue_redraw();
}

void GnoblinConsolePanel::complete() {
    const std::string source(input_.data());
    completion_source_ = source;
    const std::uint64_t generation = ++*completion_generation_;
    const std::weak_ptr<std::uint64_t> lifetime = completion_generation_;
    if (callbacks_.complete)
        callbacks_.complete(source, [this, lifetime, generation,
                                     source](std::vector<GnoblinConsoleCompletion> result) {
            const auto current = lifetime.lock();
            if (!current || *current != generation)
                return;
            if (!visible_ || source != input_.data())
                return;
            completions_ = std::move(result);
            /* Names starting with an underscore are internal; list them last. */
            std::stable_partition(completions_.begin(), completions_.end(),
                                  [](const GnoblinConsoleCompletion& item) {
                                      const auto dot = item.label.find_last_of('.');
                                      const auto leaf = dot == std::string::npos ? 0 : dot + 1;
                                      return item.label.compare(leaf, 1, "_") != 0;
                                  });
            if (completions_.size() > 64)
                completions_.resize(64);
            completion_index_ = 0;
            completion_chosen_ = false;
            completion_scroll_ = true;
            completion_visible_ = !completions_.empty();
            queue_redraw();
        });
    queue_redraw();
}

void GnoblinConsolePanel::request_preview() {
    ++*preview_generation_;
    if (!is_plain_path(preview_requested_) || !callbacks_.evaluate || pending_) {
        if (!preview_text_.empty()) {
            preview_text_.clear();
            queue_redraw();
        }
        return;
    }
    const std::uint64_t generation = *preview_generation_;
    const std::weak_ptr<std::uint64_t> lifetime = preview_generation_;
    const std::string source = preview_requested_;
    callbacks_.evaluate(source, [this, lifetime, generation, source](GnoblinConsoleResult result) {
        const auto current = lifetime.lock();
        if (!current || *current != generation || !visible_ || source != input_.data())
            return;
        std::string text;
        if (result.success && !result.value.empty()) {
            if (!result.inspection.empty())
                text = "table " + table_preview(result.inspection, false, 0);
            else
                text = result.value.substr(0, result.value.find('\n'));
        }
        if (text != preview_text_) {
            preview_text_ = std::move(text);
            queue_redraw();
        }
    });
}

void GnoblinConsolePanel::accept_completion() {
    if (!completion_visible_ || completion_index_ < 0 ||
        completion_index_ >= static_cast<int>(completions_.size()))
        return;
    if (completion_source_ != input_.data()) {
        completion_visible_ = false;
        return;
    }
    const auto& completion = completions_[completion_index_];
    const std::string current(input_.data());
    const auto token = current.find_last_of(" \t\n;(){}[]");
    const std::string replacement = completion.label.substr(0, completion.label.find('('));
    const std::string updated =
        current.substr(0, token == std::string::npos ? 0 : token + 1) + replacement;
    std::snprintf(input_.data(), input_.size(), "%s", updated.c_str());
    completion_requested_ = updated;
    completion_chosen_ = false;
    reload_input_ = true;
    focus_input_ = true;
    completion_visible_ = false;
    queue_redraw();
}

void GnoblinConsolePanel::append_result(std::string source, GnoblinConsoleResult result) {
    auto pending = std::find_if(rows_.rbegin(), rows_.rend(), [&source](const Row& row) {
        return row.pending && row.source == source;
    });
    if (pending != rows_.rend()) {
        pending->pending = false;
        pending->text = std::move(result.value);
        pending->error = std::move(result.error);
        pending->inspection = std::move(result.inspection);
    } else {
        Row row;
        row.source = std::move(source);
        row.text = std::move(result.value);
        row.error = std::move(result.error);
        row.inspection = std::move(result.inspection);
        rows_.push_back(std::move(row));
    }
    if (rows_.size() > kMaxRows)
        rows_.erase(rows_.begin());
    /* Output follows only while the view is at the end, so reading back is not interrupted. */
    scroll_to_bottom_ = scroll_to_bottom_ || at_bottom_;
    queue_redraw();
}

void GnoblinConsolePanel::append_history(const std::string& source) {
    history_.erase(std::remove(history_.begin(), history_.end(), source), history_.end());
    history_.push_back(source);
    if (history_.size() > kMaxRows)
        history_.erase(history_.begin());
    history_index_ = history_.size();
    history_draft_.clear();
    if (callbacks_.save_history)
        callbacks_.save_history(history_);
}

void GnoblinConsolePanel::history_step(int delta) {
    if (history_.empty())
        return;
    if (history_index_ == history_.size())
        history_draft_ = input_.data();
    const int next =
        std::clamp(static_cast<int>(history_index_) + delta, 0, static_cast<int>(history_.size()));
    history_index_ = static_cast<std::size_t>(next);
    const std::string& value =
        history_index_ == history_.size() ? history_draft_ : history_[history_index_];
    std::snprintf(input_.data(), input_.size(), "%s", value.c_str());
    completion_requested_ = value;
    completion_visible_ = false;
    reload_input_ = true;
    focus_input_ = true;
}

void GnoblinConsolePanel::draw_node(const GnoblinConsoleInspectorNode& node,
                                    const std::string& id) {
    ImGui::PushID(id.c_str());
    const bool placeholder =
        node.children.empty() && node.value.empty() && node.label.empty() && node.has_more;
    ImGui::PushStyleColor(ImGuiCol_Text, placeholder ? kQuietColour : kKeyColour);
    /* A node cut off at the cap shows how many entries were left out. */
    const std::string shown = !placeholder    ? plain_key(node.label)
                              : node.more > 0 ? "… " + std::to_string(node.more) + " more"
                                              : "…";
    /* Leaves use the same row as expandable nodes, so labels line up and hover alike. */
    const bool expanded = ImGui::TreeNodeEx(
        "##node", node.children.empty() ? kLeafFlags : kTreeFlags, "%s", shown.c_str());
    ImGui::PopStyleColor();
    const bool table_like = !node.children.empty() || starts_with(node.value, "<table");
    if (table_like || !node.value.empty()) {
        ImGui::SameLine(0.0f, 8.0f);
        const std::string shown_value =
            table_like ? table_preview(node.children, node.has_more, 0)
            : (starts_with(node.value, "<function") || starts_with(node.value, "<cfunction"))
                ? std::string("fn")
                : node.value;
        ImGui::PushStyleColor(ImGuiCol_Text, table_like ? kQuietColour : value_colour(node.value));
        if (node.children.empty())
            ImGui::TextWrapped("%s", shown_value.c_str());
        else
            ImGui::TextUnformatted(shown_value.c_str());
        ImGui::PopStyleColor();
    }
    if (!node.children.empty() && expanded) {
        for (std::size_t i = 0; i < node.children.size(); i++)
            draw_node(node.children[i], id + ":" + std::to_string(i));
        if (node.has_more)
            ImGui::TextColored(kQuietColour, "%s",
                               node.more > 0 ? ("… " + std::to_string(node.more) + " more").c_str()
                                             : "…");
        ImGui::TreePop();
    }
    if (node.children.empty() && node.has_more && !placeholder)
        ImGui::TextColored(kQuietColour, "…");
    ImGui::PopID();
}

void GnoblinConsolePanel::queue_redraw() {
    gnoblin_imgui_overlay_queue_redraw(overlay_);
}
