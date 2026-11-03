/* Describes monitor and layer Lua APIs registered in gnoblin-lua-install.c and implemented by
 * gnoblin-lua-queries.c, gnoblin-lua-read.c, and gnoblin-lua-animation.c. */

/*** lua
---@class MonitorRect
---@field x integer X coordinate in compositor layout.
---@field y integer Y coordinate in compositor layout.
---@field width integer Width in logical pixels.
---@field height integer Height in logical pixels.

---@class Monitor
---@field id string Monitor ID.
---@field index integer Zero-based monitor index.
---@field x integer X coordinate in compositor layout.
---@field y integer Y coordinate in compositor layout.
---@field width integer Width in logical pixels.
---@field height integer Height in logical pixels.
---@field primary boolean Whether this is the primary monitor.
---@field scale number Display scale factor.
---@field enabled boolean Whether the monitor is enabled.
---@field name? string Display name when provided by the monitor.
---@field make? string Manufacturer when provided by the monitor.
---@field model? string Model when provided by the monitor.
---@field serial? string Serial number when provided by the monitor.
---@field refresh_rate? number Refresh rate in hertz when a current mode is available.
---@field transform? string Display transform, such as "normal" or "90".
---@field work_area? MonitorRect Work area after layer surfaces reserve exclusive zones.
---@field revision integer State revision for this snapshot.

---@class PrivacyScreenMonitor
---@field id string Monitor ID.
---@field available boolean Whether the monitor supports privacy screen control.
---@field enabled? boolean Whether privacy screen is enabled when available.
---@field locked? boolean Whether privacy screen state is locked when available.

---@class PrivacyScreenSnapshot
---@field requested_enabled boolean Requested global privacy screen state.
---@field source "runtime"|"system"|"config" Where the requested state comes from.
---@field revision integer Snapshot revision.
---@field monitors PrivacyScreenMonitor[] Per-monitor availability and state.

---@class LayerFilter
---@field monitor_id? string Exact monitor ID.
---@field namespace? string Exact layer namespace.
---@field layer? string Exact layer type.

---@class LayerSurface
---@field id string Stable layer surface ID.
---@field alive boolean False once the surface has been removed. Every read of one surface returns
this same object, which shows the newest state the runtime has seen.
---@field namespace string Layer surface namespace.
---@field title string Layer surface title.
---@field layer string Layer-shell layer value.
---@field monitor_id? string Monitor ID when assigned.
---@field revision integer State revision for this snapshot.

---@class LayerAnimationPolicy
---@field namespace string Layer namespace.
---@field enter any Enter animation policy.
---@field exit any Exit animation policy.
---@field window_shadow any Shadow policy for windows matching the layer namespace.
---@field revision integer Configuration revision.

---@class PrivacyScreenSetArgs
---@field value boolean|"inherit" true or false sets an override; "inherit" clears the runtime
override.

---Return the current monitor snapshot.
---@return Monitor[]
function gnoblin.monitors.list() end

---Return the primary monitor, or nil when none is available.
---@return Monitor|nil
function gnoblin.monitors.primary() end

---Return the current privacy screen settings and per-monitor states.
---@return PrivacyScreenSnapshot
function gnoblin.monitors.privacy_screen() end

---Set the runtime privacy screen override.
---@param value boolean|"inherit" true or false sets the override; "inherit" restores the configured
or system value.
---@return Operation<Json>
function gnoblin.monitors.set_privacy_screen(value) end

---Return the current layer surface snapshot, optionally filtered.
---@param filter? LayerFilter
---@return LayerSurface[]
function gnoblin.layers.list(filter) end

---Call fn(layer, ...) on a signal of this layer surface: "changed" (with the list of changed
---fields) or "closed". Listening ends after "closed".
---@param signal "changed"|"closed"
---@param fn fun(layer: LayerSurface, changed?: string[])
---@return {unsubscribe: fun()}
function LayerSurface:on(signal, fn) end

---Call fn(layer, value, old) when one field of this layer surface changes.
---@param property string A field of LayerSurface, such as "geometry".
---@param fn fun(layer: LayerSurface, value: any, old: any)
---@return {unsubscribe: fun()}
function LayerSurface:watch(property, fn) end

---Return the configured animation policy for a layer namespace.
---@param namespace string UTF-8 namespace from 1 to 128 bytes.
---@return LayerAnimationPolicy
function gnoblin.layers.animation_policy(namespace) end

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
---@field kind "display" Kind of output; "virtual" is reserved.
---@field name? string Display name, when the hardware provides one.
---@field make? string Manufacturer, when available.
---@field model? string Model, when available.
---@field serial? string Serial number, when available.
---@field enabled boolean Whether the output is active in the current layout.
---@field primary boolean Whether this is the primary output.
---@field monitor_id? string Matching logical monitor ID.
---@field mode OutputMode Current mode.
---@field scale number Compositing scale of the logical monitor.
---@field transform string Current transformation, such as "normal" or "90".
---@field x? integer Logical X position of a display output.
---@field y? integer Logical Y position of a display output.
---@field renderer Renderer GPU path that renders this output.
---@field revision integer State revision for this snapshot.

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
*/
