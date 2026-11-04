/* Annotations for input APIs implemented in src/config/lua/gnoblin-lua-state-queries.c,
 * src/config/lua/gnoblin-lua-contexts.c, src/config/lua/gnoblin-lua-input.c,
 * src/config/lua/gnoblin-lua-objects.c, src/control/gnoblin-control-runtime-channel.c and
 * src/control/gnoblin-control-grabs.c. */

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

---An XKB layout or IBus engine. The same object stands for one source (its type and id) for as
---long as the source is listed: every read returns it and the compositor's events update it.
---@class InputSource
---@field id string Source ID.
---@field alive boolean False once the source has left the configured list.
---@field type "xkb"|"ibus" Source implementation.
---@field short_name string Short display name.
---@field name string Display name.
---@field current boolean True when this source is selected.
---@field revision integer Input source snapshot revision.

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
---@field accelerator? string Match this chord, such as "<Super>q", or for type "button" a pointer
chord such as "<Super>Button1". Any other form is an error that names the form to write.

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

---Select this source: the compositor switches to this XKB layout or IBus engine. The Operation
---succeeds with this source once the switch is confirmed.
---@return Operation<InputSource>
function InputSource:select() end

---Call fn(source, ...) on a signal of this source: "changed" (with the list of changed fields),
---"selected" when it becomes the current source, or "closed" when it leaves the configured list.
---Listening ends after "closed". The compositor reports the current source with
---gnoblin.input.source-changed and the list with gnoblin.input.sources-changed.
---@param signal "changed"|"selected"|"closed"
---@param fn fun(source: InputSource, changed?: string[])
---@return {unsubscribe: fun()}
function InputSource:on(signal, fn) end

---Call fn(source, value, old) when one field of this source changes, such as "current".
---@param property string A field of InputSource.
---@param fn fun(source: InputSource, value: any, old: any)
---@return {unsubscribe: fun()}
function InputSource:watch(property, fn) end

---Return the current orientation lock state.
---@return OrientationLock
function gnoblin.input.orientation_lock() end

---Create a one-use text target in the focused window, and spend the "text_target" scope of a
---Grant. It works only while the window that had focus at the gesture still has it, so take it
---before "focus".
---@param grant Grant From a gesture with no modifier key held.
---@return Operation<TextTarget>
function gnoblin.input.text_target(grant) end

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

---A lease on the keyboard. While a Grab is alive, every key press and release goes to it: no
---application, no shortcut and no other handler sees a key. The Grab ends by itself when it has
---had no key and no `renew()` for its `timeout` seconds, so a script that stops cannot keep the
---keyboard. There is one Grab for the whole compositor: a new one ends the old one with
---"replaced". It is the same object for as long as it is alive.
---@class Grab
---@field id string Identifier, "grab-" and the number of the request that made it.
---@field alive boolean False once the grab has ended.
---@field timeout integer Seconds without a key or a renewal after which the grab expires.
---@field device "keyboard" What the grab holds.

---A key event that a Grab received.
---@class GrabKey
---@field key string Name of the keysym, such as "a", "A", "Return", "Escape" or "dead_acute".
---@field keysym integer The keysym number.
---@field keycode integer Hardware key code.
---@field modifiers integer Modifier mask when the key came, as in InputEvent (Shift 1, Control 4,
Alt 8, Super 67108864).
---@field pressed boolean True for a press, false for a release.
---@field repeated boolean True for a press made by key repeat.
---@field text? string What a press types: one character, or the composed character that a dead
key or Compose sequence completes. Absent on a release, on a key that types nothing (a modifier,
Return, Tab, an arrow) and on a key pressed with Control, Alt or Super.
---@field composing boolean True while a dead key or Compose sequence is in progress: the keys
inside the sequence have no `text`, and the key that completes it has the composed text.
---@field time integer Event time in milliseconds.

---@class GrabOptions
---@field timeout? integer Seconds without a key or a renewal after which the grab expires, from
1 to 60. Default 10.

---@class GrabRenewResult
---@field id string The grab that was renewed.
---@field renewed boolean True when the compositor restarted the timeout.

---@class GrabReleaseResult
---@field id string The grab that was released.
---@field released boolean True when the compositor gave the keyboard back.

---Take the keyboard and return the Grab. The Grab exists at once; if the compositor refuses it
---(the session is locked, or a modal or capture_input shortcut session or a shortcut capture
---holds the keyboard, or any key is held down), it ends and its "closed" listeners get the
---reason. A key held when a grab starts would have its release go to the grab, and the focused
---application would never see the key lift, so the compositor refuses a grab while a key is down:
---start it from a key release, or later. The grab starts with its timeout running, so renew it if
---the script has nothing to receive for a while. Call this while handling an event, as with every
---operation.
---@param options? GrabOptions
---@return Grab
function gnoblin.input.grab(options) end

---Restart the timeout of this grab. A key event does the same. A grab that has ended fails with
---`not_found`.
---@return Operation<GrabRenewResult>
function Grab:renew() end

---Give the keyboard back. The Grab ends, and its "closed" listeners run with the reason
---"released", when the compositor has done it. A grab that has ended fails with `not_found`.
---@return Operation<GrabReleaseResult>
function Grab:release() end

---Call fn(grab, key) for each key event, or fn(grab, reason) when the grab ends. The reason is
---"released", "expired" (the timeout ran out), "replaced" (another grab took the keyboard),
---"locked" (the session locked), "preempted" (Mutter gave the keyboard to a popup or other
---grab), "config_reloaded" (a reload committed), "runtime_stopped" (the Lua worker is shutting
---down), or the message of the compositor when it refused the grab. On "config_reloaded" and
---"runtime_stopped" the listener runs as the old runtime goes, so it can read but cannot start an
---operation. Listening ends after "closed".
---@param signal "key"|"closed"
---@param fn fun(grab: Grab, detail: GrabKey|string|nil)
---@return {unsubscribe: fun()}
function Grab:on(signal, fn) end
*/
