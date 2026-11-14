-- data/lua/gnoblin/shell/focus.lua: the focus policy. Pure functions over an options table, so the
-- decisions are testable without a shell. Options are the window_management keys (docs: focus options).
-- An unknown key or value raises: a typo must not silently change focus behaviour.
local M = {}

local DEFAULTS = {
    focus_mode = "click",
    focus_new_windows = "prevent",
    raise_on_click = true,
    auto_raise = false,
    auto_raise_delay = 500,
    focus_change_on_pointer_rest = false,
}

local FOCUS_MODES = { click = true, ["hover"] = true, ["hover-strict"] = true }
local NEW_WINDOW_POLICIES = { allow = true, prevent = true }

-- A check for a boolean option: the key is captured so the message names it.
local function boolean(key)
    return function(v)
        if type(v) ~= "boolean" then
            error(("%s must be a boolean, got %s"):format(key, tostring(v)), 3)
        end
        return v
    end
end

local CHECKS = {
    focus_mode = function(v)
        if not FOCUS_MODES[v] then
            error(("focus_mode must be click, hover or hover-strict, got %s"):format(tostring(v)), 3)
        end
        return v
    end,
    focus_new_windows = function(v)
        if not NEW_WINDOW_POLICIES[v] then
            error(("focus_new_windows must be allow or prevent, got %s"):format(tostring(v)), 3)
        end
        return v
    end,
    raise_on_click = boolean("raise_on_click"),
    auto_raise = boolean("auto_raise"),
    focus_change_on_pointer_rest = boolean("focus_change_on_pointer_rest"),
    auto_raise_delay = function(v)
        if type(v) ~= "number" or v < 0 then
            error(("auto_raise_delay must be a non-negative number of milliseconds, got %s"):format(tostring(v)), 3)
        end
        return v
    end,
}

-- The full options table: the defaults, overridden by `user`. Unknown keys raise.
function M.settings(user)
    local out = {}
    for k, v in pairs(DEFAULTS) do
        out[k] = v
    end
    for k, v in pairs(user or {}) do
        local check = CHECKS[k]
        if not check then
            error(("unknown focus option %q"):format(tostring(k)), 2)
        end
        out[k] = check(v)
    end
    return out
end

-- Whether a new window takes focus. A window started from a launcher takes it only under `allow`.
function M.opens_focused(opts, from_launcher)
    return not from_launcher or opts.focus_new_windows == "allow"
end

-- Whether the pointer's window should take focus under the focus mode. `w` is nil over the desktop.
-- `entered` is true when the pointer crossed into `w` from outside it. hover-strict acts only on entry.
function M.hover_focus(opts, w, entered)
    if opts.focus_mode == "click" or w == nil then
        return false
    end
    if opts.focus_mode == "hover-strict" then
        return entered
    end
    return true
end

-- The auto-raise delay in seconds, for the frame clock.
function M.auto_raise_seconds(opts)
    return opts.auto_raise_delay / 1000
end

-- How long the pointer must rest over a window before focus_change_on_pointer_rest takes it, in seconds.
function M.rest_seconds()
    return 0.2
end

-- Whether a pointer rest focuses the window: a hover mode with focus_change_on_pointer_rest on.
function M.rest_focus(opts)
    return opts.focus_change_on_pointer_rest and opts.focus_mode ~= "click"
end

-- Whether the pointer leaving the focused window clears focus (hover-strict only).
function M.leave_clears(opts)
    return opts.focus_mode == "hover-strict"
end

return M
