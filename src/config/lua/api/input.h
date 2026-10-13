/* Annotations for input APIs implemented in src/config/lua/gnoblin-lua-state-queries.c,
 * src/config/lua/gnoblin-lua-contexts.c, src/config/lua/gnoblin-lua-input.c, and
 * src/control/gnoblin-control-runtime-channel.c. */

/*** lua
---@class InputDevice
---@field id string Stable ID for this compositor device instance.
---@field name string Device name; may be empty.
---@field device_type string Device category, such as keyboard, pointer, touchpad, tablet, or pen.
---@field seat? string Seat name.
---@field vendor_id? integer USB vendor ID when available.
---@field product_id? integer USB product ID when available.
---@field capabilities string[] Supported input capabilities.
---@field revision integer Input snapshot revision.

---@class InputSource
---@field id string Source ID used by select_source.
---@field type "xkb"|"ibus" Source implementation.
---@field short_name string Short display name.
---@field name string Display name.
---@field current boolean True when this source is selected.
---@field revision integer Input source snapshot revision.

---@class InputSourceSelector
---@field type "xkb"|"ibus" Source implementation.
---@field id string Non-empty source ID.

---@class OrientationLock
---@field available boolean Whether orientation lock is supported.
---@field locked boolean Whether screen orientation is currently locked.
---@field orientation string Current orientation, or "undefined" when unavailable.
---@field source "runtime"|"config"|"system" Which setting currently controls the lock.
---@field revision integer Orientation lock snapshot revision.

---@class InputHandlerMatch
---@field type "key"|"button"|"motion"|"scroll"|"touch"|"tablet"|"gesture" Required event type.
---@field device_id? string Match only this input device ID.
---@field phase? string Match only this event phase.
---@field modifiers? integer Match the event modifier mask.
---@field keycode? integer Match this hardware key code.
---@field button? integer Match this pointer button.
---@field accelerator? string Match this accelerator.

---@class InputEvent
---@field type string Input event type.
---@field device_id string Input device ID, or empty when unavailable.
---@field phase? string Input event phase.
---@field keycode? integer Hardware key code.
---@field button? integer Pointer button.
---@field modifiers? integer Modifier mask.

---@class InputHandlerResult
---@field action "forward"|"consume" Whether to pass the event onward or consume it.

---@class TextCaret
---@field x number Caret x coordinate.
---@field y number Caret y coordinate.
---@field width number Caret width.
---@field height number Caret height.

---@class TextTarget
---@field window_id? string Target window ID when available.
---@field caret? TextCaret Caret rectangle when available.

---@class OrientationLockValue
---@field value boolean|"inherit" Lock, unlock, or restore the configured/system value.

---Return available input devices.
---@return InputDevice[]
function gnoblin.input.devices() end

---Return available XKB layouts and IBus engines.
---@return InputSource[]
function gnoblin.input.sources() end

---Return the selected input source, or nil when there is no current source.
---@return InputSource|nil
function gnoblin.input.current_source() end

---Alias for input.sources.
---@return InputSource[]
function gnoblin.input.list() end

---Alias for input.current_source.
---@return InputSource|nil
function gnoblin.input.current() end

---Select an XKB layout or IBus engine.
---@param source InputSourceSelector
---@return Operation<InputSource>
function gnoblin.input.select_source(source) end

---Return the current orientation lock state.
---@return OrientationLock
function gnoblin.input.orientation_lock() end

---Set the runtime orientation lock override, or inherit the configured/system value.
---@param value boolean|"inherit" True locks orientation; false unlocks it; "inherit" clears the
runtime override.
---@return Operation<OrientationLock>
function gnoblin.input.set_orientation_lock(value) end

---Create a one-use text target from the active shortcut FocusContext.
---@param context FocusContext Context supplied to a shortcut event callback.
---@return Operation<TextTarget>
function gnoblin.input.text_target(context) end

---Register a Lua policy callback for matching input events.
---@param match InputHandlerMatch Event selectors. type is required; other selectors are optional.
---@param handler fun(event: InputEvent): InputHandlerResult|nil Return action="consume" to consume,
"forward" or nil to pass through.
---@return string Handler ID.
function gnoblin.input.on(match, handler) end

---Insert one-use text through this target during a runtime event callback.
---@param text string UTF-8 text from 1 to 256 bytes, without control characters.
---@return Operation<boolean>
function TextTarget:insert_text(text) end
*/
