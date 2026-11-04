/* Describes output and layer Lua APIs registered in gnoblin-lua.c and implemented by
 * gnoblin-lua-queries.c, gnoblin-lua-objects.c and gnoblin-lua-animation.c. */

/*** lua
---@class MonitorRect
---@field x integer X coordinate in compositor layout.
---@field y integer Y coordinate in compositor layout.
---@field width integer Width in logical pixels.
---@field height integer Height in logical pixels.

---@class OutputPrivacyScreen
---@field available boolean Whether this output has a privacy screen that Gnoblin can control.
---@field enabled? boolean Whether the privacy screen is on, when available.
---@field locked? boolean Whether a hardware switch locks the privacy screen, when available.
---@field requested boolean Whether the session asks for privacy screens to be on. This one
request applies to every output.
---@field source "runtime"|"system"|"config" What sets the request: a runtime override from
gnoblin.settings.override("monitors.privacy_screen", value), the configuration, or the system
setting.

---@class LayerSurface
---@field id string Stable layer surface ID.
---@field alive boolean False once the surface has been removed. Every read of one surface returns
this same object, updated with each change.
---@field namespace string Layer surface namespace.
---@field title string Layer surface title.
---@field layer string Layer-shell layer value.
---@field monitor_id? string Monitor ID when assigned.
---@field revision integer State revision for this snapshot.
---@observable title layer monitor_id

---@class LayerAnimationPolicy
---@field namespace string Layer namespace.
---@field enter any Enter animation policy.
---@field exit any Exit animation policy.
---@field window_shadow any Shadow policy for windows matching the layer namespace.
---@field revision integer Configuration revision.

---Return the current layer surface snapshot. Select layer surfaces with
---`gnoblin.find("LayerSurface", filter)`.
---@return LayerSurface[]
function gnoblin.layers.list() end

---Call fn(layer, ...) on a signal of this layer surface: "changed" (with the list of changed
---fields) or "closed". Listening ends after "closed".
---@param signal "changed"|"closed"
---@param fn fun(layer: LayerSurface, changed?: string[])
---@return Subscription
function LayerSurface:on(signal, fn) end

---Call fn(layer, value, old) when one field of this layer surface changes.
---@param property string A field of LayerSurface, such as "title" or "layer".
---@param fn fun(layer: LayerSurface, value: any, old: any)
---@return Subscription
function LayerSurface:watch(property, fn) end

---Return the configured animation policy for a layer namespace.
---@param layer LayerSurface|string A layer surface, or a UTF-8 namespace from 1 to 128 bytes.
---@return LayerAnimationPolicy
function gnoblin.layers.animation_policy(layer) end

---@class OutputMode
---@field width integer Width in physical pixels.
---@field height integer Height in physical pixels.
---@field refresh_rate number Refresh rate in hertz.

---@class Renderer
---@field name string GL renderer string, such as "llvmpipe".
---@field vendor? string GL vendor string, when available.
---@field api string GL API in use, such as "gles2".
---@field accelerated boolean False for software rasterizers.

---@class Output
---@field id string Connector name, such as "DP-1". Stable per output.
---@field alive boolean False once the output has been removed. Every read of one output returns
this same object, updated with each change.
---@field kind "display" Kind of output; "virtual" is reserved.
---@field name? string Display name, when the hardware provides one.
---@field make? string Manufacturer, when available.
---@field model? string Model, when available.
---@field serial? string Serial number, when available.
---@field enabled boolean Whether the output is active in the current layout.
---@field primary boolean Whether this is the primary output.
---@field monitor_id? string ID of the logical monitor that shows this output: the lowest
connector name in it. Windows and layer surfaces name it in their monitor_id.
---@field index? integer Zero-based logical monitor index, when the output is enabled.
---@field mode OutputMode Current mode.
---@field scale number Compositing scale of the logical monitor.
---@field transform string Current transformation, such as "normal" or "90".
---@field x? integer Logical X position, when the output is enabled.
---@field y? integer Logical Y position, when the output is enabled.
---@field width? integer Logical width, when the output is enabled.
---@field height? integer Logical height, when the output is enabled.
---@field work_area? MonitorRect Area of the active workspace that windows can use: the logical
monitor minus the space that panels reserve with exclusive zones.
---@field privacy_screen OutputPrivacyScreen Privacy screen state of this output.
---@field renderer Renderer GPU path that renders this output.
---@field revision integer State revision for this snapshot.
---@observable enabled primary monitor_id index mode scale transform x y width height work_area
---@observable privacy_screen renderer

---Return all current outputs. The list is empty until the first output snapshot is published.
---@return Output[]
function gnoblin.outputs.list() end

---Return the primary output, or nil when there is none.
---@return Output|nil
function gnoblin.outputs.primary() end

---Return the output with this connector ID, or nil when it does not exist.
---@param id string Connector ID, such as "DP-1".
---@return Output|nil
function gnoblin.outputs.by_id(id) end

---Return the compositor renderer, or nil until the first output snapshot is published.
---@return Renderer|nil
function gnoblin.outputs.renderer() end

---Call fn(output, ...) on a signal of this output: "changed" (with the list of changed fields)
---or "closed" when the output is removed. Listening ends after "closed".
---@param signal "changed"|"closed"
---@param fn fun(output: Output, changed?: string[])
---@return Subscription
function Output:on(signal, fn) end

---Call fn(output, value, old) when one field of this output changes.
---@param property string A field of Output, such as "scale" or "privacy_screen".
---@param fn fun(output: Output, value: any, old: any)
---@return Subscription
function Output:watch(property, fn) end
*/
