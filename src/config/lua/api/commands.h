/* Annotations for command APIs implemented in src/config/lua/gnoblin-lua-operations.c,
 * src/control/gnoblin-control-command.c, and src/control/gnoblin-control-capture.c. */

/*** lua
---@class CommandCaptureArgs
---@field argv string[] Command and arguments; 1 to 64 entries with a non-empty command name; total
size at most 64 KiB.
---@field stdin? string UTF-8 input, at most 4096 bytes.
---@field timeout? integer Timeout in seconds, from 1 to 600. Default 60.

---@class CommandRunResult
---@field accepted boolean True when the command was queued for execution.

---@class CommandResult
---@field exit_code integer Exit status, or the negative signal number when terminated by a signal.
---@field stdout string Standard output, valid UTF-8 and at most 64 KiB.

---Queue a command for asynchronous execution.
---@param argv string[] Command and arguments; 1 to 64 UTF-8 entries, non-empty argv[1], total size
at most 64 KiB.
---@return Operation<CommandRunResult>
function gnoblin.commands.run(argv) end

---Run a command and return its standard output and exit code.
---@param request CommandCaptureArgs Command request. Standard error is discarded.
---@return Operation<CommandResult>
function gnoblin.commands.capture(request) end
*/
