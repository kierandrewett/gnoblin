---@meta

--- The Gnoblin Lua API, generated from `crates/gnoblin-schema/schema` by
--- `crates/gnoblin-lua/build.rs`. Do not edit: declare a member in the schema
--- and rebuild.

--- A method call returns an `Operation`; its single completion arrives on the
--- host's next pump, so a handler never runs inside another handler.
---@class gnoblin.Operation
---@field id integer
---@field method string
---@field status "pending"|"succeeded"|"failed"
---@field code string|nil the schema error code after a failure
--- Callbacks receive `(value, error)`; `error` is nil on success and has `code` and `message` on failure.
local Operation = {}
---@param fn fun(value: any, error: table|nil)
---@return gnoblin.Subscription
function Operation:on_complete(fn) end
---@param fn fun(value: any, error: table|nil): any
---@return gnoblin.Operation
function Operation:map(fn) end
---@param seconds number
---@return gnoblin.Operation
function Operation:timeout(seconds) end
--- Stop waiting; false means the operation already finished.
---@return boolean
function Operation:cancel() end

--- One signal handler registration.
---@class gnoblin.Subscription
local Subscription = {}
--- Stop delivering the event.
function Subscription:unsubscribe() end

--- One timer on the caller's clock.
---@class gnoblin.Timer
local Timer = {}
--- Cancel the timer.
function Timer:cancel() end

--- A child process the runtime started.
---@class gnoblin.Process
---@field id integer
---@field pid integer|nil
---@field argv string[]
---@field alive boolean
---@field status string
---@field exit_code integer|nil
---@field signal string|nil
---@field revision integer
local Process = {}
--- The exit of the child, answered as an operation.
---@return gnoblin.Operation
function Process:wait() end
--- Send a signal to the child's process group; the default is term.
---@param signal? string
---@return gnoblin.Operation
function Process:kill(signal) end

--- Stop the process when it has ended, or hear the change of one field.
---@param event string
---@param callback fun(process: gnoblin.Process)
---@return gnoblin.Subscription
function Process:on(event, callback) end
--- Hear each change of one field (pid, status, exit_code or signal).
---@param field string
---@param callback fun(process: gnoblin.Process)
---@return gnoblin.Subscription
function Process:watch(field, callback) end
--- Declare configuration. Validated immediately; every problem is raised.
---@param settings table
function gnoblin.configure(settings) end

--- Append one window rule to the configured list. Load time only; later rules override the fields they set. Unknown keys and refused fields are raised by name.
---@param rule table
function gnoblin.window_rule(rule) end

--- Load configuration fragments matching a path or glob, relative to the calling file. Use `require` for a module that returns a value.
---@param path_or_glob string
---@return boolean
function gnoblin.load(path_or_glob) end

--- Subscribe to a schema-declared signal by its `namespace.object.signal` name. The handler receives an event table; the returned subscription can be unsubscribed.
---@param event string
---@param callback fun(table)
---@return gnoblin.Subscription
function gnoblin.on(event, callback) end

--- Subscribe to a schema-declared signal once. The handler receives an event table.
---@param event string
---@param callback fun(table)
---@return gnoblin.Subscription
function gnoblin.once(event, callback) end

--- Timers run on the caller's monotonic clock, measured in seconds.
--- Run `callback` after `seconds` seconds; the returned timer can be cancelled.
---@param seconds number
---@param callback fun()
---@return gnoblin.Timer
function gnoblin.timers.after(seconds, callback) end

--- The caller's monotonic clock in seconds.
---@return number
function gnoblin.timers.now() end

--- The build identity: an immutable record of string fields `gnoblin`, `gnome`, `mutter`, `lua`, `api`, `git_remote`, `git_sha` and `build_id`. Missing values are `"unknown"`; remote credentials are removed. Read on each call.
---@return table
function gnoblin.version() end

--- Child processes the runtime starts. `spawn` returns a `Process` at once; its end arrives later, through `wait()` or `kill()` operations.
--- Start the program named by the first string of `argv`, found on PATH, with the rest as arguments. Options are `cwd`, `env`, `stdin`, `stdout`, `stderr`, `output_limit` and `timeout`. A bad argv or option is raised by name.
---@param argv string[]
---@param options table|nil
---@return gnoblin.Process
function gnoblin.process.spawn(argv, options) end

--- Every process the runtime has started and not yet seen end, in id order.
---@return gnoblin.Process[]
function gnoblin.process.list() end

--- The process with this id, or nil when the id is unknown or the process was forgotten.
---@param id integer
---@return gnoblin.Process|nil
function gnoblin.process.by_id(id) end

--- The live windows, as interned objects. Each answer is as of the last pump; a window that closes leaves every answer at the same time.
--- Every live window, in identity order.
---@return gnoblin.window[]
function gnoblin.windows.list() end

--- The focused window, or nil when none has focus.
---@return gnoblin.window|nil
function gnoblin.windows.focused() end

--- The live window with this identity, or nil. A closed window has no answer here; a held object keeps `alive == false`.
---@param id integer
---@return gnoblin.window|nil
function gnoblin.windows.by_id(id) end

--- The live outputs, as interned objects. Each answer is as of the last pump.
--- Every live output, in identity order.
---@return gnoblin.output[]
function gnoblin.outputs.list() end

--- The primary output, or nil when none is marked primary.
---@return gnoblin.output|nil
function gnoblin.outputs.primary() end

--- The live output with this connector name, or nil. An output's identity is its connector name, not a number.
---@param name string
---@return gnoblin.output|nil
function gnoblin.outputs.by_id(name) end

--- The live workspaces, as interned objects. Each answer is as of the last pump.
--- Every live workspace, in identity order.
---@return gnoblin.workspace[]
function gnoblin.workspaces.list() end

--- The live workspace with this identity, or nil.
---@param id integer
---@return gnoblin.workspace|nil
function gnoblin.workspaces.by_id(id) end

--- The live objects of one kind that match a filter, as a Lua array in identity order. The kinds are `Window`, `Workspace` and `Output`. A table filter keeps the objects whose scalar fields equal every value given; a function filter keeps those it returns a truthy value for. An unknown kind or filter field is raised by name.
---@param kind string
---@param filter table|fun(object: table): boolean|nil
---@return table
function gnoblin.find(kind, filter) end

---@class gnoblin.capture
local capture = {}
---@param session integer
---@param owner string
---@param remote_session string
---@return gnoblin.Operation
function capture:open(session, owner, remote_session) end
---@param session integer
---@return gnoblin.Operation
function capture:close(session) end
---@class gnoblin.captureCollection
---@field by_id fun(id: integer): gnoblin.capture|nil
---@field all fun(): gnoblin.capture[]

---@class gnoblin.config
local config = {}
---@param name "reload_failed"
---@param callback fun(fields: { path: string, error: string, sequence: integer, time: integer })
---@return gnoblin.Subscription
function config:on(name, callback) end
---@class gnoblin.configCollection
---@field by_id fun(id: integer): gnoblin.config|nil
---@field all fun(): gnoblin.config[]

---@class gnoblin.effect
---@field kind string
---@field enabled boolean
---@field active boolean
---@field reason string
---@field downscale integer
---@field passes integer
---@field algorithm string
---@field noise number
---@field brightness number
---@field contrast number
---@field saturation number
---@field tint string
---@field offset_x number
---@field offset_y number
---@field blur_radius number
---@field spread number
---@field colour string
---@field source string
---@field entry string
local effect = {}
---@return gnoblin.Operation
function effect:remove() end
---@param name "closed"
---@param callback fun(fields: table)
---@return gnoblin.Subscription
function effect:on(name, callback) end
---@class gnoblin.effectCollection
---@field by_id fun(id: integer): gnoblin.effect|nil
---@field all fun(): gnoblin.effect[]

---@class gnoblin.eis
local eis = {}
---@param session integer
---@param owner string
---@param capabilities integer
---@param path string
---@return gnoblin.Operation
function eis:open(session, owner, capabilities, path) end
---@param session integer
---@return gnoblin.Operation
function eis:close(session) end
---@class gnoblin.eisCollection
---@field by_id fun(id: integer): gnoblin.eis|nil
---@field all fun(): gnoblin.eis[]

---@class gnoblin.idle
---@field idle_ms integer
---@field inhibitors integer
local idle = {}
---@param holder string
---@return gnoblin.Operation
function idle:inhibit(holder) end
---@param holder string
---@return gnoblin.Operation
function idle:uninhibit(holder) end
---@class gnoblin.idleCollection
---@field by_id fun(id: integer): gnoblin.idle|nil
---@field all fun(): gnoblin.idle[]

---@class gnoblin.layers
local layers = {}
---@param name string
---@param target integer
---@return gnoblin.Operation
function layers:register(name, target) end
---@class gnoblin.layersCollection
---@field by_id fun(id: integer): gnoblin.layers|nil
---@field all fun(): gnoblin.layers[]

---@class gnoblin.lock
---@field locked boolean
local lock = {}
---@return gnoblin.Operation
function lock:lock_session() end
---@class gnoblin.lockCollection
---@field by_id fun(id: integer): gnoblin.lock|nil
---@field all fun(): gnoblin.lock[]

---@class gnoblin.output
---@field name string
---@field make string
---@field model string
---@field modes string[]
---@field mode string
---@field scale number
---@field transform integer
---@field primary boolean
---@field enabled boolean
---@field x integer
---@field y integer
---@field width integer
---@field height integer
---@field layout string
local output = {}
---@param x integer
---@param y integer
---@param scale number
---@param transform integer
---@param primary boolean
---@param enabled boolean
---@param mode string
---@return gnoblin.Operation
function output:apply(x, y, scale, transform, primary, enabled, mode) end
---@param root string
---@param module string
---@param hash string
---@param options string
---@return gnoblin.Operation
function output:set_layout(root, module, hash, options) end
---@param name "closed"
---@param callback fun(fields: table)
---@return gnoblin.Subscription
function output:on(name, callback) end
---@class gnoblin.outputCollection
---@field by_id fun(id: integer): gnoblin.output|nil
---@field all fun(): gnoblin.output[]

---@class gnoblin.portal_ui
local portal_ui = {}
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param mode string
---@param title string
---@param multiple boolean
---@param filter_names string[]
---@param filter_patterns string[]
---@param current_filter integer
---@param current_folder string
---@param current_name string
---@param choice_ids string[]
---@param choice_labels string[]
---@return gnoblin.Operation
function portal_ui:file_chooser(
    request_id,
    app_id,
    deadline_ms,
    mode,
    title,
    multiple,
    filter_names,
    filter_patterns,
    current_filter,
    current_folder,
    current_name,
    choice_ids,
    choice_labels
)
end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param content_type string
---@param last_choice string
---@param filter_text string
---@param choice_ids string[]
---@param choice_labels string[]
---@return gnoblin.Operation
function portal_ui:app_chooser(
    request_id,
    app_id,
    deadline_ms,
    content_type,
    last_choice,
    filter_text,
    choice_ids,
    choice_labels
)
end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param title string
---@param choice_ids string[]
---@param choice_labels string[]
---@param copies integer
---@param max_copies integer
---@param pages string
---@return gnoblin.Operation
function portal_ui:print(
    request_id,
    app_id,
    deadline_ms,
    title,
    choice_ids,
    choice_labels,
    copies,
    max_copies,
    pages
)
end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param reason string
---@param name string
---@param avatar_uri string
---@return gnoblin.Operation
function portal_ui:account(request_id, app_id, deadline_ms, reason, name, avatar_uri) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param uri string
---@param options string
---@return gnoblin.Operation
function portal_ui:wallpaper(request_id, app_id, deadline_ms, uri, options) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param reason string
---@param autostart_requested boolean
---@return gnoblin.Operation
function portal_ui:background(request_id, app_id, deadline_ms, reason, autostart_requested) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param name string
---@param icon string
---@param desktop_file_id string
---@return gnoblin.Operation
function portal_ui:dynamic_launcher(request_id, app_id, deadline_ms, name, icon, desktop_file_id) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param source_types integer
---@param cursor_mode integer
---@param multiple boolean
---@return gnoblin.Operation
function portal_ui:screen_cast(request_id, app_id, deadline_ms, source_types, cursor_mode, multiple) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param device_types integer
---@return gnoblin.Operation
function portal_ui:remote_desktop(request_id, app_id, deadline_ms, device_types) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param capabilities integer
---@return gnoblin.Operation
function portal_ui:input_capture(request_id, app_id, deadline_ms, capabilities) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param interactive boolean
---@return gnoblin.Operation
function portal_ui:screenshot(request_id, app_id, deadline_ms, interactive) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@return gnoblin.Operation
function portal_ui:pick_color(request_id, app_id, deadline_ms) end
---@param request_id integer
---@param app_id string
---@param deadline_ms integer
---@param label string
---@return gnoblin.Operation
function portal_ui:secret(request_id, app_id, deadline_ms, label) end
---@param request_id integer
---@return gnoblin.Operation
function portal_ui:cancel(request_id) end
---@class gnoblin.portal_uiCollection
---@field by_id fun(id: integer): gnoblin.portal_ui|nil
---@field all fun(): gnoblin.portal_ui[]

---@class gnoblin.portal_outcome
local portal_outcome = {}
---@param request_id integer
---@param uris string[]
---@param choice string
---@param remember boolean
---@param copies integer
---@param pages string
---@param grant_bits integer
---@param cursor_mode integer
---@return gnoblin.Operation
function portal_outcome:handled(request_id, uris, choice, remember, copies, pages, grant_bits, cursor_mode) end
---@param request_id integer
---@param reason string
---@return gnoblin.Operation
function portal_outcome:cancelled(request_id, reason) end
---@param request_id integer
---@return gnoblin.Operation
function portal_outcome:declined(request_id) end
---@param request_id integer
---@param code string
---@param message string
---@return gnoblin.Operation
function portal_outcome:error(request_id, code, message) end
---@class gnoblin.portal_outcomeCollection
---@field by_id fun(id: integer): gnoblin.portal_outcome|nil
---@field all fun(): gnoblin.portal_outcome[]

---@class gnoblin.request
local request = {}
---@param request integer
---@return gnoblin.Operation
function request:claim(request) end
---@param request integer
---@param answer string
---@return gnoblin.Operation
function request:answer(request, answer) end
---@param request integer
---@return gnoblin.Operation
function request:decline(request) end
---@param name "offered"
---@param callback fun(fields: { request: integer, deadline: integer, slot: string, title: string, message: string, subject: string })
---@return gnoblin.Subscription
function request:on(name, callback) end
---@param name "finished"
---@param callback fun(fields: { request: integer, completion: string, reason: string })
---@return gnoblin.Subscription
function request:on(name, callback) end
---@class gnoblin.requestCollection
---@field by_id fun(id: integer): gnoblin.request|nil
---@field all fun(): gnoblin.request[]

---@class gnoblin.session
---@field version string
---@field slot_auth_polkit string
---@field slot_prompt_secret string
---@field slot_location_agent string
---@field slot_idle_policy string
local session = {}
---@param name "ready"
---@param callback fun(fields: table)
---@return gnoblin.Subscription
function session:on(name, callback) end
---@param name "bell"
---@param callback fun(fields: { count: integer })
---@return gnoblin.Subscription
function session:on(name, callback) end
---@param name "reload_requested"
---@param callback fun(fields: { generation: integer })
---@return gnoblin.Subscription
function session:on(name, callback) end
---@class gnoblin.sessionCollection
---@field by_id fun(id: integer): gnoblin.session|nil
---@field all fun(): gnoblin.session[]

---@class gnoblin.settings
---@field color_scheme integer
---@field accent_red number
---@field accent_green number
---@field accent_blue number
---@field contrast integer
---@field reduced_motion integer
---@field gtk_theme string
---@field icon_theme string
---@field cursor_theme string
---@field cursor_size integer
---@field font_name string
---@field document_font_name string
---@field monospace_font_name string
---@field text_scaling_factor number
---@field button_layout string
---@field enable_animations boolean
local settings = {}
---@class gnoblin.settingsCollection
---@field by_id fun(id: integer): gnoblin.settings|nil
---@field all fun(): gnoblin.settings[]

---@class gnoblin.window
---@field id integer
---@field title string
---@field app_id string
---@field focused boolean
---@field x integer
---@field y integer
---@field width integer
---@field height integer
---@field icon string
---@field tag string
---@field modal boolean
---@field unresponsive boolean
---@field effects gnoblin.effect[]
---@field pid integer
---@field process gnoblin.Process|nil
local window = {}
---@return gnoblin.Operation
function window:focus() end
---@return gnoblin.Operation
function window:present() end
---@return gnoblin.Operation
function window:kill() end
---@return gnoblin.Operation
function window:close() end
---@param value boolean
---@return gnoblin.Operation
function window:set_minimized(value) end
---@param value boolean
---@return gnoblin.Operation
function window:set_maximized(value) end
---@param value boolean
---@return gnoblin.Operation
function window:set_fullscreen(value) end
---@param x integer
---@param y integer
---@return gnoblin.Operation
function window:move(x, y) end
---@param width integer
---@param height integer
---@return gnoblin.Operation
function window:resize(width, height) end
---@param workspace gnoblin.workspace
---@return gnoblin.Operation
function window:set_workspace(workspace) end
---@param output gnoblin.output
---@return gnoblin.Operation
function window:set_output(output) end
---@param kind string
---@param downscale integer
---@param passes integer
---@param algorithm string
---@param noise number
---@param brightness number
---@param contrast number
---@param saturation number
---@param tint string
---@param offset_x number
---@param offset_y number
---@param blur_radius number
---@param spread number
---@param colour string
---@param source string
---@param entry string
---@return gnoblin.Operation
function window:add_effect(
    kind,
    downscale,
    passes,
    algorithm,
    noise,
    brightness,
    contrast,
    saturation,
    tint,
    offset_x,
    offset_y,
    blur_radius,
    spread,
    colour,
    source,
    entry
)
end
---@param x integer
---@param y integer
---@param width integer
---@param height integer
---@return gnoblin.Operation
function window:request_menu(x, y, width, height) end
---@param name "closed"
---@param callback fun(fields: table)
---@return gnoblin.Subscription
function window:on(name, callback) end
---@param name "menu_requested"
---@param callback fun(fields: { x: integer, y: integer, width: integer, height: integer, source: string, seat: string })
---@return gnoblin.Subscription
function window:on(name, callback) end
---@class gnoblin.windowCollection
---@field by_id fun(id: integer): gnoblin.window|nil
---@field all fun(): gnoblin.window[]

---@class gnoblin.workspace
---@field id integer
---@field layout string
local workspace = {}
---@param root string
---@param module string
---@param hash string
---@param options string
---@return gnoblin.Operation
function workspace:set_layout(root, module, hash, options) end
---@param name string
---@param params string
---@return gnoblin.Operation
function workspace:layout_command(name, params) end
---@param name "layout_changed"
---@param callback fun(fields: { layout: string })
---@return gnoblin.Subscription
function workspace:on(name, callback) end
---@param name "layout_failed"
---@param callback fun(fields: { layout: string, reason: string })
---@return gnoblin.Subscription
function workspace:on(name, callback) end
---@class gnoblin.workspaceCollection
---@field by_id fun(id: integer): gnoblin.workspace|nil
---@field all fun(): gnoblin.workspace[]

---@class gnoblin
---@field capture gnoblin.captureCollection
---@field config gnoblin.configCollection
---@field effect gnoblin.effectCollection
---@field eis gnoblin.eisCollection
---@field idle gnoblin.idleCollection
---@field layers gnoblin.layersCollection
---@field lock gnoblin.lockCollection
---@field output gnoblin.outputCollection
---@field portal_ui gnoblin.portal_uiCollection
---@field portal_outcome gnoblin.portal_outcomeCollection
---@field request gnoblin.requestCollection
---@field session gnoblin.sessionCollection
---@field settings gnoblin.settingsCollection
---@field window gnoblin.windowCollection
---@field workspace gnoblin.workspaceCollection
---Load a Lua module by dotted name. `a.b` is `a/b.lua` under the configuration directory, then under the shipped modules. A `gnoblin.` name loads from the shipped modules only.
---@param name string
---@return any
function require(name) end
