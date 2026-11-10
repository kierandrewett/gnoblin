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

local function boolean(key, v)
    if type(v) ~= "boolean" then
        error(("%s must be a boolean, got %s"):format(key, tostring(v)), 3)
    end
    return v
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
    raise_on_click = boolean,
    auto_raise = boolean,
    focus_change_on_pointer_rest = boolean,
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

return M
