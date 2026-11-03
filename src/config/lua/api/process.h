/* Annotations for the Lua API: processes. Implemented in src/config/lua/gnoblin-lua-process.c and,
 * in the compositor, src/control/gnoblin-control-process.c. */

/*** lua
---Options for gnoblin.process.spawn. Every field is optional.
---@class ProcessOptions
---@field cwd? string Absolute working directory; the default is the compositor's.
---@field env? table<string, string> Variables added to the session environment, at most 128.
---@field stdin? string Bytes for standard input, at most 1 MiB; empty when unset.
---@field stdout? "inherit"|"discard"|"capture" Where standard output goes; "inherit" by default.
---@field stderr? "inherit"|"discard"|"capture" Where standard error goes; "inherit" by default.
---@field output_limit? integer Bytes kept per captured stream, 0 to 4194304; 65536 by default.
---@field timeout? number Seconds, up to 86400, before the process group is ended.

---A process started by gnoblin.process.spawn. Its fields are read-only and change as it runs.
---@class Process
---@field id integer Identifier for this process within the session.
---@field argv string[] The command line it was started with.
---@field pid integer|nil Process ID, once the process has started.
---@field status "starting"|"running"|"exited"|"failed" "failed" means it could not be started.
---@field exit_code integer|nil Exit status, once it exited normally.
---@field signal string|integer|nil Signal that ended it, such as "term", once it was killed.

---How a process ended, the value of Process:wait().
---@class ProcessExit
---@field exit_code integer|nil Exit status when the process exited normally.
---@field signal string|integer|nil Signal that ended the process, such as "term" or "kill".
---@field timed_out boolean True when the timeout option ended the process.
---@field stdout? string Captured standard output, as raw bytes.
---@field stdout_truncated? boolean True when standard output passed output_limit.
---@field stderr? string Captured standard error, as raw bytes.
---@field stderr_truncated? boolean True when standard error passed output_limit.

---Start a program. The process runs in its own session and process group. It keeps running when
---the config reloads, but the Process handle belongs to the runtime that started it.
---@param argv string[] Program and arguments: 1 to 64 strings, at most 64 KiB in total.
---@param options? ProcessOptions
---@return Process
function gnoblin.process.spawn(argv, options) end

---An Operation that succeeds with a ProcessExit when the process ends, whatever its exit status,
---and fails when the process could not be started. Every call returns the same Operation.
---@return Operation<ProcessExit>
function Process:wait() end

---Send a signal to the process group.
---@param signal? "term"|"kill"|"int"|"hup"|"usr1"|"usr2"|"stop"|"cont" The default is "term".
---@return Operation<{signal: string}>
function Process:kill(signal) end
*/
