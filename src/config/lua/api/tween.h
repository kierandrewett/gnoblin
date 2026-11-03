/* Annotations for easing and tweens, in src/config/lua/gnoblin-lua-tween.c and
 * src/control/gnoblin-control-tweens.c. */

/*** lua
---A timeline of steps that the compositor plays. Steps run one after another; parallel() makes
---the next step run with the previous one. Build it, then call play().
---@class Tween
---@field status "idle"|"playing"|"paused"|"finished"|"killed"|"cancelled"

---Return eased progress for t from 0 to 1.
---@param name string linear, sine, quad, cubic, quart, quint, expo, circ, back, elastic or bounce.
---@param t number Progress from 0 to 1; values outside are clamped.
---@param direction? "in"|"out"|"in_out" Default "in_out".
---@return number
function gnoblin.ease(name, t, direction) end

---Return a + (b - a) * t.
---@param a number
---@param b number
---@param t number
---@return number
function gnoblin.lerp(a, b, t) end

---Create an empty tween.
---@return Tween
function gnoblin.tween() end

---Add a step that changes a window's opacity, scale, rotation or offset to `to` over `seconds`,
---starting from the value the window has when the step begins. An offset takes {x, y}.
---@param target Window
---@param property "opacity"|"scale"|"rotation"|"offset"
---@param to number|{x: number, y: number}
---@param seconds number From 0 to 600.
---@param ease? string An ease name from gnoblin.ease. Default "linear".
---@param direction? "in"|"out"|"in_out" Default "in_out".
---@return Tween
function Tween:property(target, property, to, seconds, ease, direction) end

---Add a step that waits.
---@param seconds number From 0 to 600.
---@return Tween
function Tween:interval(seconds) end

---Add a step that calls fn(tween) when the timeline reaches it. The timeline does not wait for it.
---@param fn fun(tween: Tween)
---@return Tween
function Tween:callback(fn) end

---Make the next step run at the same time as the previous one.
---@return Tween
function Tween:parallel() end

---Call fn(tween, outcome) when the tween ends: "finished", "killed" or "cancelled".
---@param fn fun(tween: Tween, outcome: string)
---@return Tween
function Tween:on_finished(fn) end

---Start the tween, or resume it after pause(). The Operation succeeds with {tween_id, outcome}
---when the tween ends.
---@return Operation<{tween_id: string, outcome: string}>
function Tween:play() end

---Pause the tween where it is.
---@return Operation<{tween_id: string}>
function Tween:pause() end

---Jump to a time in seconds from the start. Callbacks before that time run.
---@param seconds number
---@return Operation<{tween_id: string}>
function Tween:seek(seconds) end

---Stop the tween. Windows keep the values they have; the outcome is "killed".
---@return Operation<{tween_id: string}>
function Tween:kill() end
*/
