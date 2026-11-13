-- data/lua/gnoblin/shell/lock.lua: the stock lock screen. The host sends `requested` when the session locks; the shell
-- engages the Lock node and draws a password entry over the desk. `unlock` asks the native core to check the password
-- and waits for its answer. It returns `"ok"` (and disengages) or `""` with the error `"denied"`; a rate-limited
-- attempt returns the wait in milliseconds as the result with the error `"rate-limited"`. Any other outcome keeps the
-- lock engaged. The password is the entry's text and goes to the compositor as the call's secret text only: the screen
-- never prints it, and a connection that is not the session owner's has no tree to draw it in.
Box:extend("LockScrim")({ fill = "#11111bf2" })
Box:extend("LockCard")({
    width = 360,
    radius = 16,
    fill = "#1e1e2e",
    padding = 20,
    gap = 12,
    direction = "column",
    secure = true,
})

-- How often a request that waits for its desk looks again, in seconds on the shell's frame clock.
local POLL = 0.1
local shown = false -- the lock screen is up, or a request is waiting for its desk

-- Starts the lock screen once. A request can come before the shell has a desk, so the screen waits for it (`show`).
function Lock:engage()
    if shown then
        return
    end
    shown = true
    go(function()
        self:show()
    end)
end

function Lock:on_requested()
    self:engage()
end

-- The request is one-shot: a shell that attaches to a session already locked (a restart, or one that came up after the
-- request) hears nothing. Its snapshot carries the lock state, which is the request.
function Lock:on_ready()
    if self.state ~= "not-locked" then
        self:engage()
    end
end

local function feedback_for(result, err)
    if err == "rate-limited" then
        local seconds = math.ceil((tonumber(result) or 0) / 1000)
        return ("Too many attempts. Try again in %d s."):format(seconds)
    elseif err == "denied" then
        return "Wrong password."
    elseif err == "busy" then
        return "Checking the previous attempt."
    end
    return "The password could not be checked."
end

-- Draws the lock screen on the desk under the pointer and returns once the session is unlocked. The desk may not exist yet
-- (the shell attaches before its screen is published): the screen waits for it, and gives up if the session unlocks first.
-- The session can also unlock while the screen is up (another client unlocks it through the protocol): the screen comes
-- down then, and a later request draws it again.
function Lock:show()
    local d = desk()
    while self.state ~= "not-locked" and not d do
        await(POLL)
        d = desk()
    end
    if self.state == "not-locked" then -- unlocked or refused before its screen could be drawn
        shown = false
        return
    end
    local before = seat.focus
    local scrim = d:add(LockScrim({ x = 0, y = 0, width = d.width, height = d.height }))
    local card = scrim:add(LockCard({ x = (d.width - 360) // 2, y = d.height // 3, height = 150 }))
    card:add(Text({ text = "Locked", size = 18 }))
    local entry = card:add(Entry({ secret = true, height = 44, radius = 10, fill = "#313244", size = 15 }))
    local message = card:add(Text({ text = "Enter your password.", size = 13, color = "#a6adc8" }))
    seat.focus = entry
    self.engaged = true
    local submitted -- the text of the last Enter the loop has not taken yet
    entry.submit:connect(function(_, text)
        submitted = text
    end)
    local result
    while self.state ~= "not-locked" and result ~= "ok" do
        local text = submitted
        submitted = nil
        if text == nil then
            await(POLL)
        elseif text ~= "" then
            local err
            result, err = self:unlock(text)
            submitted = nil -- an Enter pressed while the native core was checking is not a second attempt
            entry.text = ""
            if result ~= "ok" then
                message.text = feedback_for(result, err)
                message.color = "#f38ba8"
            end
        end
    end
    shown = false
    self.engaged = false
    -- Focus goes back to the window that had it only if the screen still held it: a menu or another window that took
    -- focus while the screen was going down keeps it.
    local held = seat.focus == entry
    scrim:destroy()
    if held and before then
        pcall(function()
            seat.focus = before
        end)
    end
end

function Lock:unlock(password)
    local result, err = await(self:authenticate(password))
    if result == "ok" then
        self.engaged = false
    end
    return result, err
end
