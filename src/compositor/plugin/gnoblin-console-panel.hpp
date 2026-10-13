/*
 * Compositor-owned Dear ImGui developer console.
 *
 * The panel is presentation and interaction code only. It never evaluates
 * Lua or reads or writes configuration itself. The
 * compositor coordinator supplies those capabilities through Callbacks. In
 * particular, inspection is rendered solely from InspectorNode
 * data returned by the coordinator; expanding an object must not invoke a
 * getter in the compositor UI thread.
 */

#pragma once

#include <functional>
#include <memory>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

#include <clutter/clutter.h>

#include "gnoblin-imgui-overlay.hpp"

enum class GnoblinConsoleControl {
    ReloadConfiguration,
};

struct GnoblinConsoleInspectorNode {
    std::string label;
    std::string value;
    std::vector<GnoblinConsoleInspectorNode> children;
    bool has_more = false;
    /* Entries past the cap that were not sent; 0 when the number is unknown. */
    std::uint32_t more = 0;
};

struct GnoblinConsoleResult {
    bool success = true;
    std::string value;
    std::string error;
    std::vector<GnoblinConsoleInspectorNode> inspection;
};

struct GnoblinConsoleCompletion {
    std::string label;
    std::string detail;
    std::string documentation;
};

/* Evaluation may complete asynchronously.  The coordinator must call done on
 * the compositor thread while the panel remains alive.  It owns the language
 * runtime and supplies the Lua evaluator and validated runtime controls. */
using GnoblinConsoleEvaluateFunc =
    std::function<void(std::string_view source, std::function<void(GnoblinConsoleResult)> done)>;

/* Handles :reload through the existing validated config service. */
using GnoblinConsoleControlFunc = std::function<void(
    GnoblinConsoleControl control, std::function<void(GnoblinConsoleResult)> done)>;

/* Async completion reads names from the runtime without evaluating input.
 * The coordinator completes requests on the compositor thread. */
using GnoblinConsoleCompleteFunc = std::function<void(
    std::string_view source, std::function<void(std::vector<GnoblinConsoleCompletion>)> done)>;

/* Persistence is injected because settings storage is session policy.  The
 * UI caps the stored command history at 200 entries. */
using GnoblinConsoleLoadHistoryFunc = std::function<std::vector<std::string>()>;
using GnoblinConsoleSaveHistoryFunc = std::function<void(const std::vector<std::string>&)>;
/* The coordinator owns shared overlay visibility. It receives this notification
 * after the console's panel state changes and recomputes overlay visibility
 * alongside recovery state. */
using GnoblinConsoleVisibilityChangedFunc = std::function<void()>;

struct GnoblinConsoleCallbacks {
    GnoblinConsoleEvaluateFunc evaluate;
    GnoblinConsoleControlFunc control;
    GnoblinConsoleCompleteFunc complete;
    GnoblinConsoleLoadHistoryFunc load_history;
    GnoblinConsoleSaveHistoryFunc save_history;
    GnoblinConsoleVisibilityChangedFunc visibility_changed;
};

class GnoblinConsolePanel {
  public:
    explicit GnoblinConsolePanel(GnoblinImGuiOverlay* overlay,
                                 GnoblinConsoleCallbacks callbacks = {});
    ~GnoblinConsolePanel();

    GnoblinConsolePanel(const GnoblinConsolePanel&) = delete;
    GnoblinConsolePanel& operator=(const GnoblinConsolePanel&) = delete;

    /* Visibility is panel state. The coordinator owns shared overlay visibility
     * because recovery and console draw into one overlay. */
    void set_visible(bool visible);
    bool visible() const;
    void toggle();
    void clear();

    /* Called by the coordinator's single overlay draw callback. The overlay
     * already made its ImGui context current for this frame. */
    void draw_frame(GnoblinImGuiOverlay* overlay);

    /* Route text and key events captured by the compositor while the panel is
     * open. add_text() forwards UTF-8 input to the overlay's ImGui context.
     * Return true when the console consumed a key. */
    void add_text(const char* utf8);
    bool handle_key(guint keyval, ClutterModifierType modifiers, bool pressed,
                    bool repeated = false);

    /* Useful for runtime log forwarding.  The transcript is bounded to 200
     * rows, independently of persisted command history. */
    void append_log(std::string_view text, bool error = false);

  private:
    struct Row;
    void draw();
    void submit();
    void submit_control(GnoblinConsoleControl control, std::string_view source);
    void complete();
    void accept_completion();
    void append_result(std::string source, GnoblinConsoleResult result);
    void append_history(const std::string& source);
    void history_step(int delta);
    void draw_node(const GnoblinConsoleInspectorNode& node, const std::string& id);
    void draw_completions();
    void queue_redraw();

    GnoblinImGuiOverlay* overlay_;
    GnoblinConsoleCallbacks callbacks_;
    std::vector<char> input_;
    std::vector<Row> rows_;
    std::vector<std::string> history_;
    std::size_t history_index_ = 0;
    std::string history_draft_;
    std::vector<GnoblinConsoleCompletion> completions_;
    int completion_index_ = 0;
    bool completion_visible_ = false;
    std::shared_ptr<std::uint64_t> completion_generation_ = std::make_shared<std::uint64_t>(0);
    std::string completion_source_;
    bool scroll_to_bottom_ = true;
    /* Whether the transcript was scrolled to its end last frame; new output follows only then. */
    bool at_bottom_ = true;
    int scroll_request_pages_ = 0;
    /* The input text completions were last requested for, so typing asks once the text settles. */
    std::string completion_requested_;
    /* Set once the user moves in the list; Enter then accepts, otherwise it submits. */
    bool completion_chosen_ = false;
    float transcript_height_ = 0.0f;
    /* The input takes keyboard focus when the console opens and after each
     * action, so typing works without a click first. ImGui keeps its own copy of
     * the text while the box is active, so a buffer rewritten by submit, history
     * or completion must be reloaded into it. */
    bool focus_input_ = true;
    bool reload_input_ = false;
    /* Enter and Tab act on the input only after ImGui has applied every queued
     * character, or text typed just before them would be lost. */
    bool enter_requested_ = false;
    bool tab_requested_ = false;
    /* Where the completion list is drawn: above the input, inside the screen. */
    bool completion_scroll_ = false;
    float input_anchor_x_ = 0.0f;
    float input_anchor_y_ = 0.0f;
    float input_width_ = 0.0f;
    bool pending_ = false;
    bool visible_ = false;
};
