/* Annotations for easing and tweens, in src/config/lua/gnoblin-lua-tween.c and
 * src/control/gnoblin-control-tweens.c. */

/*** lua
---A timeline of steps that the compositor plays. Steps run one after another; parallel() makes
---the next step run with the previous one. Build it, then call play(). A Tween is a live object:
---the same handle for one id, with `alive`, `on` and `watch`. It is alive from gnoblin.tween()
---until it has ended, and a config reload ends it too. The steps are private to the handle.
---@class Tween
---@field id string Identifier of this tween within the session.
---@field alive boolean False once the tween has ended: it finished, was killed, or was cancelled.
---@field status "idle"|"playing"|"paused"|"finished"|"killed"|"cancelled"
---@field revision integer Counts the changes of `status`.
---@observable status

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

---Add a step that changes a property of a live object to `to` over `seconds`, starting from the
---value the object has when the step begins. Each kind of object declares the properties a tween
---can drive; today only a Window does: opacity, scale, rotation and offset. A property that the
---target does not declare fails with an error that lists them. An offset takes {x, y}.
---@param target Window The live object to animate.
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

---Call fn(tween) on a signal of this tween. "finished" runs when the timeline reached its end.
---"closed" runs when the tween has ended however it ended, as fn(tween, outcome): the outcome is
---"finished", "killed", "cancelled", or "config_reloaded" when a reload ended it. "changed" runs
---with the list of changed fields. Listening ends after "closed".
---@param signal "changed"|"finished"|"closed"
---@param fn fun(tween: Tween, detail?: string|string[])
---@return Subscription
function Tween:on(signal, fn) end

---Call fn(tween, value, old) when `status` changes.
---@param property "status" The field of Tween to watch.
---@param fn fun(tween: Tween, value: any, old: any)
---@return Subscription
function Tween:watch(property, fn) end
*/
