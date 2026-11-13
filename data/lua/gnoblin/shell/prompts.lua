-- data/lua/gnoblin/shell/prompts.lua: the stock consent and passphrase dialogs.
--
-- Gnoblin has built-in dialogs, which are the default and the fallback. This module claims the two slots a shell can
-- draw, `portal.ui` (an app's portal request asks the user to allow or deny) and `prompt.secret` (the keyring asks
-- for its password), and draws both as nodes, so they look like the rest of the shell. A dialog that cannot be
-- drawn or answered hands the request back and the compositor's own dialog serves it.
--
-- To turn them off, assign `seat.data.prompts = false` in a shell file, or `{ portal = false }` or
-- `{ secret = false }` to keep one. The option is read once, when the shell has loaded its files. To draw your own,
-- turn the stock one off and call `slots.claim` yourself (see node.lua).
--
-- What the consent dialog guarantees, each with its test in tests/lua/prompts_test.lua:
--   * Deny is the focused button when it opens, and Escape denies.
--   * Tab, Left and Right move between the buttons, and Return or Space presses the focused one.
--   * Allow does nothing for the first ARM_SECONDS, so a click or a key already in progress cannot accept it. The
--     compositor refuses an allow inside the same window as a refusal, so nothing here may send one early.
--   * It shows who asks: `offer.subject` is the app id the portal backend verified. It never shows text the app wrote
--     as the identity.
--   * It sits above every window, takes the pointer and the keys while it is open, is `secure` so a View capture masks
--     it, and puts the focus back where it was when it closes.
-- The passphrase dialog types into an entry that shows asterisks, answers the text once, on Return, and declines on
-- Escape or Cancel.
--
-- A passphrase lives in the entry's Lua string until the dialog closes, and Lua cannot wipe a string. Like the lock
-- screen's password, it is never put in a node property, a report or a log, and it leaves this module once: as the
-- answer to the compositor.
local M = {}

-- Seconds a dialog ignores an allow. The compositor applies the same 500 ms from the moment it sent the offer (and
-- treats an earlier allow as a refusal), so waiting here the same time from when this shell saw the offer is enough.
M.ARM_SECONDS = 0.5

local WIDTH, PAD, GAP, LINE = 480, 20, 10, 20
local CHARS_PER_LINE = 54
local LEFT = 272 -- BTN_LEFT

-- Text from a peer is for display only: no control characters, no invalid UTF-8, and no more than `max` characters.
function M.clean(text, max)
    text = tostring(text or "")
    if not utf8.len(text) then
        return "(unreadable)"
    end
    text = text:gsub("%c", " ")
    if utf8.len(text) > max then
        text = text:sub(1, utf8.offset(text, max + 1) - 1) .. "…"
    end
    return text
end

-- The lines `text` takes at `width` characters a line, broken at spaces. A word longer than a line is cut.
function M.wrap(text, width)
    local lines, line = {}, ""
    for token in text:gmatch("%S+") do
        local word = token
        while utf8.len(word) > width do
            if line ~= "" then
                lines[#lines + 1], line = line, ""
            end
            local cut = utf8.offset(word, width + 1) - 1
            lines[#lines + 1], word = word:sub(1, cut), word:sub(cut + 1)
        end
        if line == "" then
            line = word
        elseif utf8.len(line) + 1 + utf8.len(word) <= width then
            line = line .. " " .. word
        else
            lines[#lines + 1], line = line, word
        end
    end
    if line ~= "" then
        lines[#lines + 1] = line
    end
    return lines
end

-- Who asked, as the dialog says it. An empty subject is a caller the portal could not name.
function M.identity(subject)
    if subject == nil or subject == "" then
        return "an application that could not be identified"
    end
    return M.clean(subject, 64)
end

Box:extend("PromptButton")({
    label = "",
    focused = false,
    enabled = true,
    radius = 10,
    height = 44,
    fill = function(b)
        if not b.enabled then
            return "#2a2a3a"
        end
        return b.focused and "#89b4fa" or "#45475a"
    end,
})
function PromptButton:build()
    return {
        Text({
            text = function()
                return self.label
            end,
            color = function()
                if not self.enabled then
                    return "#6c7086"
                end
                return self.focused and "#11111b" or "#cdd6f4"
            end,
            x = function(t)
                return (self.width - t.width) // 2
            end,
            y = function(t)
                return (self.height - t.height) // 2
            end,
        }),
    }
end

-- The dark layer over the desk. It takes every press, so nothing under a dialog is clicked while it is open.
Box:extend("PromptScrim")({ fill = "#000000aa" })

-- Both dialogs: a column of Text lines, then the buttons. `chosen` is emitted once: with what the user chose, with
-- `nil` when they cancelled, or with `false` when the request ended without them.
Box:extend("PromptDialog")({
    width = WIDTH,
    radius = 16,
    fill = "#1e1e2e",
    padding = PAD,
    gap = GAP,
    direction = "column",
    secure = true,
    armed = false,
    index = 1,
    choices = {},
    chosen = signal,
})

-- The button the keyboard presses.
function PromptDialog:select(index)
    self.index = index
    for i, button in ipairs(self.choices) do
        button.focused = i == index
    end
end

-- Lets the dialog accept an allow, or a passphrase.
function PromptDialog:arm()
    self.armed = true
    if self.choices[2] then
        self.choices[2].enabled = true
    end
end

local function stop(e)
    if e.stop then
        e:stop()
    end
end

-- The heading, who asks, and the question: the rows both dialogs start with. Returns the rows and the height they take
-- with the gap that follows each.
local function heading(offer)
    local rows, height = {}, 0
    rows[#rows + 1] = Text({ text = M.clean(offer.title, 60), size = 17, color = "#cdd6f4", height = 26 })
    if offer.slot == "portal.ui" then
        local named = offer.subject ~= nil and offer.subject ~= ""
        rows[#rows + 1] = Text({
            text = "Asked by " .. M.identity(offer.subject),
            size = 13,
            color = named and "#a6adc8" or "#f38ba8",
            height = LINE,
        })
    end
    for _, line in ipairs(M.wrap(M.clean(offer.message, 400), CHARS_PER_LINE)) do
        rows[#rows + 1] = Text({ text = line, size = 14, color = "#cdd6f4", height = LINE })
    end
    for _, row in ipairs(rows) do
        height = height + row.height + GAP
    end
    return rows, height
end

-- Two buttons side by side, as the rows and the pair of buttons. Other dialogs of the shell (alerts.lua) use it too.
function M.button_row(first, second)
    local row = Box({ direction = "row", gap = 12, padding = 0, height = 44 })
    local a = row:add(PromptButton({ label = first, grow = 1 }))
    local b = row:add(PromptButton({ label = second, grow = 1 }))
    return row, { a, b }
end

-- ConsentDialog: Deny and Allow.
PromptDialog:extend("ConsentDialog")({})
function ConsentDialog:activate(index)
    local answer = index == 1 and "deny" or "allow"
    if answer == "allow" and not self.armed then
        return
    end
    self.chosen:emit(answer)
end
function ConsentDialog:on_key(e)
    if e.pressed == false then
        return
    end
    local key = e.key
    if key == "Escape" then
        self.chosen:emit("deny")
    elseif key == "Tab" or key == "ISO_Left_Tab" or key == "Left" or key == "Right" then
        self:select(self.index % 2 + 1)
    elseif key == "Return" or key == "KP_Enter" or key == "space" then
        self:activate(self.index)
    else
        return
    end
    stop(e)
end

-- PassphraseDialog: an entry, Cancel and Unlock.
PromptDialog:extend("PassphraseDialog")({ entry = Entry })
function PassphraseDialog:activate(index)
    if index == 1 then
        self.chosen:emit(nil)
    elseif self.armed and self.entry.text ~= "" then
        self.chosen:emit(self.entry.text)
    end
end
function PassphraseDialog:on_key(e)
    if e.pressed == false then
        return
    end
    if e.key == "Escape" then
        self.chosen:emit(nil)
    elseif e.key == "Return" or e.key == "KP_Enter" then
        self:activate(2)
    else
        return
    end
    stop(e)
end

-- Builds the consent dialog and its scrim on `d`. Returns the scrim (destroy it to close) and the dialog.
function M.build_consent(d, offer)
    local rows, height = heading(offer)
    local row, buttons = M.button_row("Deny", "Allow")
    buttons[2].enabled = false -- until the dialog is armed
    local total = 2 * PAD + height + 44
    local scrim = d:add(PromptScrim({ x = 0, y = 0, width = d.width, height = d.height }))
    local dialog = scrim:add(ConsentDialog({
        x = (d.width - WIDTH) // 2,
        y = math.max(0, (d.height - total) // 3),
        height = total,
        choices = buttons,
    }))
    for _, r in ipairs(rows) do
        dialog:add(r)
    end
    dialog:add(row)
    dialog:select(1) -- Deny is the default
    return scrim, dialog
end

-- Builds the passphrase dialog and its scrim on `d`.
function M.build_passphrase(d, offer)
    local rows, height = heading(offer)
    local entry = Entry({ secret = true, height = 44, radius = 10, fill = "#313244", size = 15 })
    local row, buttons = M.button_row("Cancel", "Unlock")
    buttons[2].enabled = false
    local total = 2 * PAD + height + 44 + GAP + 44
    local scrim = d:add(PromptScrim({ x = 0, y = 0, width = d.width, height = d.height }))
    local dialog = scrim:add(PassphraseDialog({
        x = (d.width - WIDTH) // 2,
        y = math.max(0, (d.height - total) // 3),
        height = total,
        choices = buttons,
        entry = entry,
    }))
    for _, r in ipairs(rows) do
        dialog:add(r)
    end
    dialog:add(entry)
    dialog:add(row)
    dialog:select(1)
    return scrim, dialog
end

-- One prompt is on screen at a time, in the order the compositor offered them. A later offer waits its turn, and
-- gives up when its request ends first.
local busy = false
local function take_turn(offer)
    while busy do
        if offer.finished then
            return false
        end
        await(0.1)
    end
    busy = not offer.finished
    return busy
end

-- Shows the dialog `build(d, offer)` makes until the user chooses or the request ends. Returns what `chosen` was
-- emitted with: an answer, `nil` for a cancel, `false` when the request ended first.
local function present(offer, build)
    local d = desk()
    if not d then
        offer:decline() -- no screen to draw on
        return false
    end
    if not take_turn(offer) then
        return false
    end
    local before = seat.focus
    local made, scrim, dialog = pcall(build, d, offer)
    if not made then
        busy = false
        error(scrim, 0)
    end
    local closed = false
    local function close()
        if closed then
            return
        end
        closed = true
        scrim:destroy()
        busy = false
        if before then
            pcall(function()
                seat.focus = before
            end)
        end
    end
    -- The compositor ended the request (its requester left, it timed out): nobody is waiting for an answer, so the
    -- handler is woken to close the dialog.
    offer:on_finished(function()
        if not closed then
            dialog.chosen:emit(false)
        end
    end)
    scrim.press:connect(function(_, e)
        stop(e)
        seat.focus = dialog.armed and dialog.entry or dialog
    end)
    for i, button in ipairs(dialog.choices) do
        button.press:connect(function(_, e)
            stop(e)
            if e.button == nil or e.button == LEFT then
                dialog:select(i)
                dialog:activate(i)
            end
        end)
    end
    seat.focus = dialog
    go(function()
        await(M.ARM_SECONDS)
        if not closed then
            dialog:arm()
            if dialog.entry then
                seat.focus = dialog.entry -- keys typed before this went nowhere
            end
        end
    end)
    local chosen = await(dialog.chosen)
    close()
    return chosen
end

function M.consent(offer)
    local choice = present(offer, M.build_consent)
    if (choice == "allow" or choice == "deny") and not offer.finished then
        offer:answer(choice)
    end
end

function M.passphrase(offer)
    local text = present(offer, M.build_passphrase)
    if offer.finished then
        return
    end
    if type(text) == "string" then
        offer:answer(text)
    elseif text == nil then
        offer:decline() -- cancelled: the keyring stays locked, as it does when nothing draws the prompt
    end
end

-- Which prompts to draw: `seat.data.prompts` is `false`, or a table that turns one off with `portal = false` or
-- `secret = false`. Anything else, including nothing, draws both.
function M.wanted()
    local option = seat and seat.data.prompts
    if option == false then
        return { portal = false, secret = false }
    end
    option = type(option) == "table" and option or {}
    return { portal = option.portal ~= false, secret = option.secret ~= false }
end

-- Claims the slots `M.wanted()` names. A claim the compositor refuses (another shell holds it) is reported, and the
-- compositor's dialog keeps serving that prompt.
function M.claim()
    local wanted = M.wanted()
    for _, slot in ipairs({
        { "portal.ui", "portal", M.consent },
        { "prompt.secret", "secret", M.passphrase },
    }) do
        local name, key, handler = slot[1], slot[2], slot[3]
        if wanted[key] and not slots.claimed(name) then
            local ok, err = pcall(slots.claim, name, handler)
            if not ok then
                report(("prompts: %s was not claimed (%s); the built-in dialog serves it"):format(name, tostring(err)))
            end
        end
    end
end

function M.release()
    slots.release("portal.ui")
    slots.release("prompt.secret")
end

-- The shell's own files have not loaded when this module does, so the claim waits for the first frame, after they have
-- and `seat.data.prompts` is what the user set. A shell with no seat (the unit tests of other modules) draws no prompts.
if seat then
    go(function()
        await(0)
        M.claim()
    end)
end

return M
