/* Annotations for idle watches implemented in src/config/lua/gnoblin-lua-state-queries.c,
 * src/config/lua/gnoblin-lua-objects.c, and
 * src/portal/grants/gnoblin-control-session-activity.c. */

/*** lua
---A watch on how long the session has had no input. It is the same object for as long as it is
---alive. It is not idle when it starts. When the user does nothing for `seconds`, `idle` becomes
---true and the "idle" signal runs; the next input makes `idle` false and runs "active". The
---watch repeats for every idle period until it is cancelled. Mutter's idle monitor counts the
---time, and an idle inhibitor of the session delays "idle" as it delays the screen blank.
---A runtime reload ends every watch.
---@class IdleWatch
---@field id string Identifier, "idle-" and the number of the request that made it.
---@field alive boolean False once the watch has ended.
---@field seconds number How long without input makes the session idle.
---@field idle boolean Whether the session has been idle for `seconds` and has not had input since.

---@class IdleCancelResult
---@field cancelled boolean True when the compositor removed the watch.

---Watch for the session being idle for `seconds` and return the IdleWatch. Any number of watches
---can exist, up to 256 for each runtime, each with its own time. The watch exists at once, not
---idle; if the compositor refuses it, it ends and its "closed" listeners get the reason.
---@param seconds number Idle time above 0 and at most 86400. Fractions are allowed.
---@return IdleWatch
function gnoblin.idle.watch(seconds) end

---Stop this watch. The IdleWatch ends, and its "closed" listeners run, when the compositor has
---removed it. A watch that has ended fails with `not_found`.
---@return Operation<IdleCancelResult>
function IdleWatch:cancel() end

---Call fn(watch) on a signal of this watch: "idle" when the session has been idle for its
---seconds, "active" for the first input after that, or "closed" when the watch ended. A watch
---that the compositor refused passes the reason as a second argument to "closed". Listening
---ends after "closed".
---@param signal "idle"|"active"|"closed"
---@param fn fun(watch: IdleWatch, reason?: string)
---@return {unsubscribe: fun()}
function IdleWatch:on(signal, fn) end

---Call fn(watch, value, old) when one field of this watch changes.
---@param property string A field of IdleWatch, such as "idle".
---@param fn fun(watch: IdleWatch, value: any, old: any)
---@return {unsubscribe: fun()}
function IdleWatch:watch(property, fn) end
*/
