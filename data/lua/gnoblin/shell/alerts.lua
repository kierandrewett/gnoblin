-- data/lua/gnoblin/shell/alerts.lua: the stock "not responding" alert.
--
-- The compositor pings every client that has a window. One that leaves a ping unanswered for longer than
-- `window-management.check-alive-timeout` (5000 ms by default; 0 turns the pings off) has its windows marked
-- `unresponsive`, and this module asks the user what to do about each:
--   * Wait, the default: the alert closes, and does not come back for that window until it has answered a ping and
--     then stopped answering again.
--   * Force quit: the window's process is killed (`Window:kill`). The compositor refuses the compositor's own process,
--     init and a process that is not the one that opened the window.
--
-- The alert is built from the consent dialog's parts (prompts.lua): Escape is Wait, Tab, Left and Right move between
-- the buttons, Return or Space presses the focused one, and Force quit does nothing for the first ARM_SECONDS, so a
-- click or a key already on its way when the alert appears cannot press it. It is `secure` when the window is, since
-- it shows the window's title. It closes by itself when the window answers again, closes, or the session locks, and
-- while the session is locked no alert opens. One alert is open for each window that is unresponsive.
--
-- The alert takes the keyboard, because an unresponsive window is the one the user was typing into, and gives it back
-- when it closes. It closes an open window menu first.
--
-- `unresponsive` is a property, and a script reads a property by asking or by binding; a binding cannot wait, so the
-- alert asks once a second. The panel's clock wakes the shell on the same second, so this adds no wakeup.
--
-- To turn it off, assign `seat.data.alerts = false` in a shell file. To draw your own, turn it off and read
-- `Window.unresponsive` yourself.
local Prompts = require("gnoblin.shell.prompts")
local Menus = require("gnoblin.shell.menus")
local M = {}

-- Seconds Force quit ignores a press after the alert opens.
M.ARM_SECONDS = 0.5
-- Seconds between looks at the windows.
M.INTERVAL = 1

local LEFT = 272 -- BTN_LEFT
local WIDTH, PAD, GAP = 480, 20, 10

-- The card open for each window, and the windows whose user chose Wait (or Force quit): no alert for them until they
-- have answered a ping.
local cards = setmetatable({}, { __mode = "k" })
local waited = setmetatable({}, { __mode = "k" })

-- The consent dialog with its answers renamed: its Deny is Wait and its Allow is Force quit.
ConsentDialog:extend("NotRespondingDialog")({})

-- The card for `w` on `d`. The window's title is the client's own text, so it is shown as text and never as the
-- identity of anything.
function M.build(d, w)
    local name = Prompts.clean(w.title, 60)
    if name == "" then
        name = "This application"
    end
    local row, buttons = Prompts.button_row("Wait", "Force quit")
    buttons[2].enabled = false -- until the alert is armed
    local rows = {
        Text({ text = Prompts.clean(name .. " is not responding", 80), size = 17, color = "#cdd6f4", height = 26 }),
        Text({ text = "You can wait for it, or force it to quit.", size = 14, color = "#cdd6f4", height = 20 }),
        Text({ text = "Anything it has not saved will be lost.", size = 14, color = "#a6adc8", height = 20 }),
    }
    local height = 0
    for _, r in ipairs(rows) do
        height = height + r.height + GAP
    end
    local total = 2 * PAD + height + 44
    local dialog = d:add(NotRespondingDialog({
        name = "not-responding",
        x = (d.width - WIDTH) // 2,
        y = math.max(0, (d.height - total) // 4),
        height = total,
        choices = buttons,
        secure = w.secure,
    }))
    for _, r in ipairs(rows) do
        dialog:add(r)
    end
    dialog:add(row)
    dialog:select(1) -- Wait is the default
    return dialog
end

-- Shows the alert for `w` until the user chooses or the window no longer needs it. Runs in a coroutine.
function M.show(w)
    local d = w:closest(Desk)
    if not d or cards[w] then
        return
    end
    Menus.close()
    local before = seat.focus
    local dialog = M.build(d, w)
    cards[w] = dialog
    dialog.press:connect(function(_, e)
        e:stop()
        seat.focus = dialog
    end)
    for i, button in ipairs(dialog.choices) do
        button.press:connect(function(_, e)
            e:stop()
            if e.button == nil or e.button == LEFT then
                dialog:select(i)
                dialog:activate(i)
            end
        end)
    end
    seat.focus = dialog
    go(function()
        await(M.ARM_SECONDS)
        if cards[w] == dialog then
            dialog:arm()
        end
    end)
    local answer = await(dialog.chosen)
    cards[w] = nil
    pcall(function()
        dialog:destroy()
    end)
    if before then
        pcall(function()
            seat.focus = before
        end)
    end
    if answer == "recovered" then
        return
    end
    waited[w] = true
    if answer == "allow" then
        w:kill()
    end
end

-- Looks at every window once: opens the alert for one that has just stopped answering, and closes the alert of one
-- that answers again, has gone, or lies under a lock.
function M.check()
    local blocked = Menus.locked()
    local live = {}
    for _, w in ipairs(root:all(Window)) do
        live[w] = true
        if w.unresponsive then
            if not waited[w] and not cards[w] and not blocked then
                go(function()
                    M.show(w)
                end)
            end
        else
            waited[w] = nil
            if cards[w] then
                cards[w].chosen:emit("recovered")
            end
        end
    end
    for w, card in pairs(cards) do
        if blocked or not live[w] then
            card.chosen:emit("recovered")
        end
    end
end

-- Whether the alert is on: `seat.data.alerts` is not `false`. Read on every look, so a shell file can turn it off.
function M.wanted()
    return not (seat and seat.data.alerts == false)
end

-- A shell with no seat (the unit tests of other modules) has no windows to watch. An error in one look is reported and
-- the next look still runs.
if seat then
    every(M.INTERVAL, function()
        if M.wanted() then
            local ok, err = pcall(M.check)
            if not ok then
                report("not-responding alert: " .. tostring(err))
            end
        end
    end)
end

return M
