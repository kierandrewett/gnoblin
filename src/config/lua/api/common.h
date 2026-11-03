/* Annotations for the Lua API: shared types, the Operation object and timers.
 *
 * The annotation blocks under src/config/lua/api/ describe the public gnoblin.* Lua API in LuaCATS,
 * the format Lua Language Server reads. scripts/gen-lua-api.py turns them into the console's
 * hints and completion table and into docs/public/gnoblin.d.lua. Each file's header names the C
 * code its annotations describe; update both together, then run scripts/gen-lua-api.py. */

/*** lua
---@alias Json nil|boolean|number|string|table

---Stable error codes carried by a failed Operation.
---@alias ErrorCode
"invalid_argument"|"not_found"|"unsupported"|"denied"|"busy"|"cancelled"|"timed_out"|"unavailable"|"internal"

---The failure of an Operation.
---@class OperationError
---@field code ErrorCode Stable machine-readable code.
---@field message string Human-readable explanation.

---Handle for a request that completes asynchronously. Mutating calls return one; read the result
---with on_complete.
---@class Operation<T>
---@field id integer Identifier of the request.
---@field method string Native method the call maps to.
---@field status "pending"|"succeeded"|"failed"
---@field value T|nil Result once the status is "succeeded".
---@field error OperationError|nil Failure once the status is "failed".
*/

/*** lua
---Call callback when the operation finishes. The callback receives the value and the error; one
---of them is nil. A callback added after the operation finished runs once, after the current
---handler returns.
---@param callback fun(value: any, error: OperationError|nil)
---@return OperationSubscription
function Operation:on_complete(callback) end

---A callback added with on_complete.
---@class OperationSubscription

---Stop calling this callback.
function OperationSubscription:unsubscribe() end

---A new Operation that succeeds with fn(value) when this one succeeds. When this one fails, or fn
---raises an error, the new one fails; an error raised by fn has the code "internal".
---@param fn fun(value: any): any
---@return Operation
function Operation:map(fn) end

---A new Operation that finishes with this one's result, or fails with the code "timed_out" after
---the given time. The timeout does not cancel this operation.
---@param seconds number From 0 to 86400.
---@return Operation
function Operation:timeout(seconds) end

---Stop waiting for this operation. It fails with the code "cancelled" and its callbacks run. A
---timer stops; an action already sent to the compositor still runs and its result is ignored.
---@return boolean cancelled False when the operation had already finished.
function Operation:cancel() end
*/

/*** lua
---An Operation that succeeds with nil after the given time. Use it from an event handler or an
---operation callback, not while the config file loads.
---@param seconds number From 0 to 86400.
---@return Operation<nil>
function gnoblin.timers.after(seconds) end

---Seconds on a monotonic clock. Only differences between two readings are meaningful.
---@return number
function gnoblin.timers.now() end
*/
