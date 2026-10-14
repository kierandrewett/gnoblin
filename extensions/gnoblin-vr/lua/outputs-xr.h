/* LuaCATS for the optional extension-owned XR output helper. */

/*** lua
---@class XROutputView
---@field eye "left"|"right"|"view" View label supplied by OpenXR.
---@field width integer Recommended OpenXR swapchain image width for this view.
---@field height integer Recommended OpenXR swapchain image height for this view.

---@class XROutput
---@field id string Stable extension output identifier.
---@field kind "xr" Output kind for an OpenXR headset.
---@field name string Human-readable headset name.
---@field status "absent"|"stopped"|"running"|"error" Client state.
---@field available boolean Whether the client has an active OpenXR session.
---@field enabled boolean Whether the client has an active OpenXR session.
---@field diagnostic? string Last client diagnostic.
---@field views XROutputView[] Per-eye recommended image dimensions.

---Start the extension-owned supervised OpenXR client.
---@return Operation<CommandRunResult>
function gnoblin.outputs.xr.start() end

---Stop the extension-owned supervised OpenXR client.
---@return Operation<CommandRunResult>
function gnoblin.outputs.xr.stop() end

---Read the extension client status file.
---@return Operation<CommandResult>
function gnoblin.outputs.xr.status() end
*/
