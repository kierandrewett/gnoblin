/* Annotations for the Lua API: shared types.
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
---of them is nil.
---@param callback fun(value: any, error: OperationError|nil)
function Operation:on_complete(callback) end
*/
