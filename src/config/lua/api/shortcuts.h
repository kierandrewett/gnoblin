/* Annotations for shortcut APIs implemented in src/config/lua/gnoblin-lua-state-queries.c,
 * src/config/lua/gnoblin-lua-operations.c, src/cli/gnoblinctl-lua-shortcuts.c, and
 * src/control/gnoblin-control-runtime-channel.c. */

/*** lua
---@class ShortcutAction
---@field id string Action identifier.
---@field group "wm"|"mutter"|"wayland" Action group.
---@field key string Action key, with hyphens converted to underscores.
---@field description? string Action description.
---@field default_bindings string[] Default accelerator strings.

---@class ShortcutState
---@field name string Configured shortcut name.
---@field binding string|string[] One accelerator, or all configured accelerators.
---@field enabled boolean Whether the shortcut is enabled.
---@field trigger "press"|"release" Event edge that activates the shortcut.
---@field action? string Built-in action identifier.
---@field command? string[] Command argument vector.
---@field revision integer Shortcut snapshot revision.

---A global key binding that this Lua runtime owns. It is the same object for as long as it is
---bound: every read of one binding returns it. It ends when it is unbound or when the runtime
---stops; a binding the compositor refused ends at once.
---@class Shortcut
---@field id string Caller-supplied binding ID.
---@field alive boolean False once the binding has ended.
---@field accelerator string The chord in its written form, such as "<Alt><Super>F9".
---@field hold "none"|"super"|"control"|"alt" Modifier family held for the binding. Default "none".
---@field trigger "press"|"release" Event edge that activates the binding. Default "press".
---@field mode "passive"|"modal" Whether other input continues while held. Default "passive".
---@field capture_input boolean Whether the bare Super session captures subsequent key input.
Default false.

---@class ShortcutBindOptions
---@field hold? "none"|"super"|"control"|"alt" Modifier family held for the binding. Default "none".
---@field trigger? "press"|"release" Activation edge. Default "press".
---@field mode? "passive"|"modal" Modal requires a non-none hold. Default "passive".
---@field capture_input? boolean Only bare "Super" supports input capture. Default false.

---@class ShortcutUnbindResult
---@field id string Unbound binding ID.
---@field unbound boolean True when the binding was removed.

---@class ShortcutEndSessionResult
---@field id string Binding ID.
---@field session_id integer Ended session ID.
---@field ended boolean True when the session was ended.

---@class ShortcutCaptureOptions
---@field timeout? integer Capture timeout in seconds, from 1 to 60. Default 30.

---@class ShortcutCaptureResult
---@field accelerator string Captured normalized accelerator.

---List registered shortcut actions, optionally limited to a group.
---@param group? "wm"|"mutter"|"wayland" Action group.
---@return ShortcutAction[]
function gnoblin.shortcuts.actions(group) end

---List the shortcuts that the configuration declares. They are read-only records, not objects:
---gnoblin.configure.shortcuts changes them.
---@return ShortcutState[]
function gnoblin.shortcuts.configured() end

---List the shortcuts this runtime has bound and not unbound, in ID order. Each is the object
---that bind returned.
---@return Shortcut[]
function gnoblin.shortcuts.list() end

---Bind a global shortcut for the current Lua runtime and return it. The Shortcut exists at once
---with the values asked for, and its accelerator is the chord in its written form; if the
---compositor refuses the binding (the chord is taken, for example) it ends and its "closed"
---listeners get the reason. An ID that is already bound is an error, so unbind it first.
---@param id string Unique ID, 1 to 64 ASCII letters, digits, underscores, or hyphens.
---@param accelerator string A chord such as "<Super><Alt>F9" (see the chord syntax). Any other
form is an error that names the form to write. Bare "Super" requires capture_input=true,
trigger="release", and hold="none".
---@param options? ShortcutBindOptions
---@return Shortcut
function gnoblin.shortcuts.bind(id, accelerator, options) end

---Unbind this shortcut. The Shortcut ends, and its "closed" listeners run, when the compositor
---has removed the binding.
---@return Operation<ShortcutUnbindResult>
function Shortcut:unbind() end

---End the active session of this binding and release its keyboard capture. The binding stays
---bound. A stale session ID is rejected. The shortcut owns the session, so the call lives here
---and not on gnoblin.shortcuts.
---@param session_id integer Positive ID of the active session, from a session_activated event.
---@return Operation<ShortcutEndSessionResult>
function Shortcut:end_session(session_id) end

---Call fn(shortcut, event) on a signal of this shortcut. The event is the one the compositor
---sent (see the gnoblin.shortcut events), with a `grant` on a real activation.
---
---  - "activated": the accelerator was pressed, or released for a release trigger. A held
---    binding reports its first activation only; its later repeats are session events.
---  - "deactivated": a press-triggered binding was released.
---  - "session_activated", "session_key" and "session_ended": the session of a held or modal
---    binding starts, receives a key, and ends (the event has the reason).
---  - "closed": the binding ended. A refused binding passes the reason as a second argument.
---
---Listening ends after "closed".
---@param signal
"activated"|"deactivated"|"session_activated"|"session_key"|"session_ended"|"closed"
---@param fn fun(shortcut: Shortcut, event_or_reason?: table|string)
---@return {unsubscribe: fun()}
function Shortcut:on(signal, fn) end

---Capture one normalized keyboard accelerator.
---@param options? ShortcutCaptureOptions
---@return Operation<ShortcutCaptureResult>
function gnoblin.shortcuts.capture(options) end
*/
