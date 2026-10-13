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

---@class ShortcutBinding
---@field id string Caller-supplied binding ID.
---@field accelerator string Accelerator accepted by Mutter.
---@field hold "none"|"super"|"control"|"alt" Modifier family held for the binding. Default "none".
---@field trigger "press"|"release" Event edge that activates the binding. Default "press".
---@field mode "passive"|"modal" Whether other input continues while held. Default "passive".
---@field capture_input boolean Whether the bare Super session captures subsequent key input.
Default false.

---@class ShortcutBindingArgs
---@field id string Unique ID, 1 to 64 ASCII letters, digits, underscores, or hyphens.
---@field accelerator string Valid accelerator. Bare "Super" requires capture_input=true,
trigger="release", and hold="none".
---@field hold? "none"|"super"|"control"|"alt" Modifier family held for the binding. Default "none".
---@field trigger? "press"|"release" Activation edge. Default "press".
---@field mode? "passive"|"modal" Modal requires a non-none hold. Default "passive".
---@field capture_input? boolean Only bare "Super" supports input capture. Default false.

---@class ShortcutIdArgs
---@field id string Binding ID owned by this Lua runtime.

---@class ShortcutSessionArgs
---@field id string Binding ID owned by this Lua runtime.
---@field session_id integer Positive ID of the active binding session.

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

---List the configured shortcut records.
---@return ShortcutState[]
function gnoblin.shortcuts.list() end

---Register a shortcut binding owned by the current Lua runtime.
---@param binding ShortcutBindingArgs
---@return Operation<ShortcutBinding>
function gnoblin.shortcuts.bind(binding) end

---Remove a shortcut binding owned by the current Lua runtime.
---@param binding ShortcutIdArgs
---@return Operation<ShortcutUnbindResult>
function gnoblin.shortcuts.unbind(binding) end

---Capture one normalized keyboard accelerator.
---@param options? ShortcutCaptureOptions
---@return Operation<ShortcutCaptureResult>
function gnoblin.shortcuts.capture(options) end

---End an active shortcut binding session.
---@param session ShortcutSessionArgs
---@return Operation<ShortcutEndSessionResult>
function gnoblin.shortcuts.end_session(session) end
*/
