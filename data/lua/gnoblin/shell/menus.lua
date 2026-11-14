-- data/lua/gnoblin/shell/menus.lua: the stock window menu.
--
-- The compositor asks for a window's menu in three ways, and each reaches the shell as a `menu` request on the
-- window's node (`Window:on_request`): a client's `xdg_toplevel.show_window_menu` (with a serial the compositor
-- issued), the keyboard's window-menu key, and the `window.request_menu` call of a program. The titlebar's right click
-- asks too: its action is the seat's `right_click` and defaults to `menu`. All of them end in `Window:show_menu(x, y)`.
-- A shell that never loads this file leaves every one of those requests unanswered, and nothing else changes.
--
-- The menu is a column of nodes, drawn on the Desk the window is on. What it guarantees, each with its test in
-- tests/lua/menus_test.lua and, on the real compositor, in crates/gnoblin-compositor/tests/shell_vm.rs:
--   * It opens with its corner at the anchor, moved back inside the screen when it would run off it.
--   * Up, Down, Home, End, Tab and Shift+Tab move the selection over the entries that are on. A letter jumps to the
--     next entry that starts with it. Return and Space run the selection, Right opens a submenu and Left or Escape
--     closes it, and Escape closes the menu. The pointer selects what it is over and a left click runs it.
--   * A press anywhere else closes it, and is not passed on: it is the click that dismissed the menu.
--   * One menu is open at a time. Asking for another closes the first. It closes when the window closes, when the
--     session locks, and when another dialog takes the keyboard.
--   * A menu for a `secure` window is `secure`, so a View capture masks it.
--   * The keys go to the menu while it is open, and the focus goes back to where it was when it closes.
--
-- The tree has no role or label properties yet, so a menu names itself the one way a node can: its `name`. The menu is
-- `window-menu`, and each entry is named for its label (`Maximise`), so a reader of the tree can find them. The Lua
-- tags `menu` and `menuitem` (and `disabled`, `checked`, `submenu` on an entry) are for selectors.
--
-- To change what the menu offers, replace `Window:menu_items`. To draw something else, replace `Window:show_menu`.
-- docs/guides/window-menu.md has a complete example of each.
local M = {}

local ITEM_H, SEPARATOR_H, PAD = 28, 9, 4
local EDGE = 8 -- the space inside an entry, left and right of its label
local MARK = 14 -- the width of the check and submenu marks
local MIN_WIDTH = 170

local COLOURS = {
    card = "#1e1e2e",
    hot = "#89b4fa",
    text = "#cdd6f4",
    hot_text = "#11111b",
    off = "#6c7086",
    line = "#45475a",
}

-- The menu now open, or nil. The module owns it: one menu at a time is this variable.
local current

-- The estimate Text itself uses until the compositor has measured a label: half the size a character. The answer
-- comes a frame later and replaces it (see ui.lua).
local function label_width(text)
    local known = text.measured
    if known.key ~= nil then
        return known.width
    end
    return (utf8.len(text.text) or #text.text) * text.size // 2
end

-- A tick or a chevron, drawn from lines so it never depends on the font having the glyph.
Box:extend("MenuMark")({
    kind = "",
    color = COLOURS.text,
    width = MARK,
    draw = function(m)
        local top = (m.height - 10) // 2
        local c = m.color
        if m.kind == "check" then
            return {
                { line = { 2, top + 5, 5, top + 9 }, width = 1.5, color = c },
                { line = { 5, top + 9, 12, top + 1 }, width = 1.5, color = c },
            }
        elseif m.kind == "arrow" then
            return {
                { line = { 4, top, 9, top + 5 }, width = 1.5, color = c },
                { line = { 9, top + 5, 4, top + 10 }, width = 1.5, color = c },
            }
        end
        return {}
    end,
})

-- The rule between groups of entries.
Box:extend("MenuSeparator")({
    height = SEPARATOR_H,
    draw = function(s)
        -- Before the card lays its entries out a separator is as wide as nothing, and the compositor refuses a negative size.
        return { { rect = { EDGE, s.height // 2, math.max(0, s.width - 2 * EDGE), 1 }, fill = COLOURS.line } }
    end,
})

-- One entry: a check column, the label, and a mark when it opens a submenu. `selected` is the entry the keyboard is
-- on. A disabled entry is shown dim and the selection skips it.
Box:extend("MenuItem")({
    label = "",
    enabled = true,
    checked = false,
    submenu = false,
    selected = false,
    spec = {},
    label_node = Text,
    radius = 6,
    direction = "row",
    gap = 4,
    height = ITEM_H,
    fill = function(item)
        return item.selected and item.enabled and COLOURS.hot or "#00000000"
    end,
})
function MenuItem:ink()
    if not self.enabled then
        return COLOURS.off
    end
    return self.selected and COLOURS.hot_text or COLOURS.text
end
function MenuItem:build()
    local item = self
    local label = Text({
        text = self.label,
        size = 14,
        grow = 1,
        color = function()
            return item:ink()
        end,
    })
    self.label_node = label
    return {
        Box({ width = EDGE }),
        MenuMark({
            kind = self.checked and "check" or "",
            color = function()
                return item:ink()
            end,
        }),
        label,
        MenuMark({
            kind = self.submenu and "arrow" or "",
            color = function()
                return item:ink()
            end,
        }),
        Box({ width = EDGE }),
    }
end
-- How wide the entry wants to be: its label, the marks, the gaps and the edges. The label's own width is whatever its
-- row gave it, so the measured width is read, which is what the painter lays the text out in.
function MenuItem:natural()
    return label_width(self.label_node) + 2 * EDGE + 2 * MARK + 4 * 4
end
function MenuItem:on_enter()
    local menu = self:closest(WindowMenu)
    if menu and self.enabled then
        menu:select(self)
    end
end
function MenuItem:on_press(e)
    e:stop()
    local menu = self:closest(WindowMenu)
    if menu and self.enabled and (e.button == nil or e.button == 272) then
        menu:select(self)
        menu:activate(self)
    end
end

-- A column of entries on a card. It takes the presses on its padding, so a press there is not a click away.
Box:extend("MenuPanel")({
    direction = "column",
    radius = 10,
    fill = COLOURS.card,
    padding = PAD,
    gap = 0,
    rows = {},
    needs = 0,
    width = function(panel)
        local widest = MIN_WIDTH
        for _, row in ipairs(panel.rows) do
            if row.natural then
                widest = math.max(widest, row:natural() + 2 * PAD)
            end
        end
        return widest
    end,
})
function MenuPanel:on_press(e)
    e:stop()
end

-- The whole menu: a transparent layer over the Desk that takes every press, so one outside the cards is a click away,
-- and that holds the keyboard focus. The cards are its children, so closing it closes them.
Box:extend("WindowMenu")({
    window = Window,
    before = Node,
    rows = {},
    sub_rows = {},
    panel = MenuPanel,
    sub = MenuPanel,
    index = 0,
    depth = 1,
})

-- The rows of the panel the keyboard is in.
function WindowMenu:active()
    return self.depth == 2 and self.sub_rows or self.rows
end

function WindowMenu:select(row)
    if self.depth == 2 and row.parent ~= self.sub then
        self:close_submenu() -- the pointer went back to the card the submenu opened from
    end
    for i, r in ipairs(self:active()) do
        r.selected = r == row
        if r == row then
            self.index = i
        end
    end
end

-- Moves the selection by `step` over the rows that can be run, wrapping at the ends. With nothing selected, a step
-- forward lands on the first row and a step back on the last.
function WindowMenu:move(step)
    local rows = self:active()
    local count = #rows
    local at = self.index
    if at == 0 then
        at = step > 0 and 0 or count + 1
    end
    for _ = 1, count do
        at = (at - 1 + step) % count + 1
        local row = rows[at]
        if row.enabled then
            self:select(row)
            return
        end
    end
end

-- The first row from `from` on, wrapping, whose label starts with `letter` (compared without case).
function WindowMenu:jump(letter)
    local rows = self:active()
    local count = #rows
    for step = 1, count do
        local row = rows[(self.index - 1 + step) % count + 1]
        if row.enabled and row.label:sub(1, 1):lower() == letter:lower() then
            self:select(row)
            return
        end
    end
end

-- Opens the submenu of `row` to the right of the card, or to the left when there is no room, and puts the selection on
-- its first entry.
function WindowMenu:open_submenu(row)
    local spec = row.spec
    local entries = type(spec.submenu) == "function" and spec.submenu(self.window) or spec.submenu
    if type(entries) ~= "table" or #entries == 0 then
        return
    end
    self:close_submenu()
    local panel = self.panel
    local at = row.y + panel.y
    local sub, rows = M.panel(self, entries, {
        x = function(p)
            local right = panel.x + panel.width - 2
            if right + p.width > self.width then
                return math.max(0, panel.x - p.width + 2)
            end
            return right
        end,
        y = math.max(0, math.min(at - PAD, self.height - M.height_of(entries))),
    })
    self.sub, self.sub_rows, self.depth = sub, rows, 2
    self.index = 0
    self:move(1)
end

function WindowMenu:close_submenu()
    if self.depth ~= 2 then
        return
    end
    self.sub:destroy()
    self.sub_rows, self.depth, self.index = {}, 1, 0
    for i, r in ipairs(self.rows) do
        if r.submenu and r.selected then
            self.index = i
        end
    end
end

-- Runs a row. A submenu opens; anything else closes the menu first and then runs, so what it does (a window that
-- moves, a dialog that wants the keyboard) starts from a desk with no menu on it.
function WindowMenu:activate(row)
    if not row.enabled or not row.spec then
        return
    end
    if row.submenu then
        self:open_submenu(row)
        return
    end
    local window, run = self.window, row.spec.run
    M.close()
    if run then
        run(window)
    end
end

function WindowMenu:on_press(e)
    e:stop()
    M.close()
end

-- The keys while the menu is open. Every key is taken: a menu is modal.
function WindowMenu:on_key(e)
    if e.pressed == false then
        return
    end
    local key = e.key
    local rows = self:active()
    if key == "Escape" then
        if self.depth == 2 then
            self:close_submenu()
        else
            M.close()
        end
    elseif key == "Down" or key == "Tab" then
        self:move(1)
    elseif key == "Up" or key == "ISO_Left_Tab" then
        self:move(-1)
    elseif key == "Home" then
        self.index = 0
        self:move(1)
    elseif key == "End" then
        self.index = 0
        self:move(-1)
    elseif key == "Return" or key == "KP_Enter" or key == "space" then
        if rows[self.index] then
            self:activate(rows[self.index])
        end
    elseif key == "Right" then
        local row = rows[self.index]
        if self.depth == 1 and row and row.submenu then
            self:open_submenu(row)
        end
    elseif key == "Left" then
        self:close_submenu()
    elseif e.text and #e.text == 1 and e.text:match("%S") then
        self:jump(e.text)
    end
    if e.stop then
        e:stop()
    end
end

-- The height the card for `entries` takes.
function M.height_of(entries)
    local h = 2 * PAD
    for _, spec in ipairs(entries) do
        h = h + (spec.separator and SEPARATOR_H or ITEM_H)
    end
    return h
end

-- Makes the card for `entries` on `menu`, with the extra properties `extra` (its place). Returns the card and its
-- selectable rows, in order. A separator is not a row.
function M.panel(menu, entries, extra)
    local spec = {
        name = "window-menu",
        tags = { "menu" },
        height = M.height_of(entries),
        secure = menu.window.secure,
    }
    for k, v in pairs(extra) do
        spec[k] = v
    end
    local panel = menu:add(MenuPanel(spec))
    local rows = {}
    for _, entry in ipairs(entries) do
        if entry.separator then
            panel:add(MenuSeparator({}))
        else
            local tags = { "menuitem" }
            for flag, on in pairs({
                disabled = entry.enabled == false,
                checked = entry.checked == true,
                submenu = entry.submenu ~= nil and entry.submenu ~= false,
            }) do
                if on then
                    tags[#tags + 1] = flag
                end
            end
            local row = panel:add(MenuItem({
                name = entry.label,
                tags = tags,
                label = entry.label,
                enabled = entry.enabled ~= false,
                checked = entry.checked == true,
                submenu = entry.submenu ~= nil and entry.submenu ~= false,
            }))
            row.spec = entry
            rows[#rows + 1] = row
        end
    end
    panel.rows = rows
    return panel, rows
end

-- Whether the session is locked or about to be: nothing opens over the lock screen.
local function locked()
    local l = root:find(Lock)
    return l ~= nil and (l.engaged or (l.state or "not-locked") ~= "not-locked")
end
M.locked = locked

-- Gives the seat's focus back to `node`, when it is still in the tree. A node that has gone cannot be focused, and the
-- compositor would refuse the whole transaction that said so, taking everything else the frame did with it.
function M.refocus(node)
    local alive, parent = pcall(function()
        return node.parent
    end)
    if alive and parent ~= nil then
        pcall(function()
            seat.focus = node
        end)
    end
end

-- Closes the menu, if one is open, and gives the keyboard back to where it was.
function M.close()
    local menu = current
    if not menu then
        return false
    end
    current = nil
    local before = menu.before
    pcall(function()
        menu:destroy()
    end)
    M.refocus(before)
    return true
end

-- The menu now open, or nil.
function M.current()
    return current
end

-- Opens the menu of `window` with its corner at (x, y), in output coordinates. Returns the menu, or nil when none
-- opened: the window is not showing, it has no Desk, or the session is locked.
function M.open(window, x, y)
    M.close()
    local d = window:closest(Desk)
    if not d or locked() or window.minimised or window:closest(Workspace) ~= d.active then
        return nil
    end
    local entries = window:menu_items()
    if type(entries) ~= "table" or #entries == 0 then
        return nil
    end
    local screen = d.screen
    local left, top = x - (screen and screen.screen_x or 0), y - (screen and screen.screen_y or 0)
    local menu = d:add(WindowMenu({
        name = "window-menu-layer",
        tags = { "menu-layer" },
        window = window,
        x = 0,
        y = 0,
        width = d.width,
        height = d.height,
    }))
    menu.before = seat.focus
    local height = M.height_of(entries)
    local panel, rows = M.panel(menu, entries, {
        x = function(p)
            return math.max(0, math.min(left, d.width - p.width))
        end,
        y = math.max(0, math.min(top, d.height - height)),
    })
    menu.panel, menu.rows = panel, rows
    current = menu
    menu.destroyed:connect(function()
        if current == menu then
            current = nil
        end
    end)
    seat.focus = menu
    return menu
end

-- What the window menu offers: a list of entries, each a table with a `label` and
--   run      = function(window)   what it does, after the menu has closed;
--   enabled  = false              shown dim and skipped by the keyboard (default true);
--   checked  = true               shown with a tick;
--   submenu  = { entries } or function(window) returning them, instead of `run`;
-- or `{ separator = true }`, a rule between groups. Replace this method to change the menu.
function Window:menu_items()
    local floating = not (self.maximised or self.fullscreen or self.tiled)
    local restores = self.maximised or self.fullscreen or self.data.snapped ~= nil
    local current_workspace = self:closest(Workspace)
    local entries = {
        restores and {
            label = "Restore",
            run = function(w)
                if w.fullscreen then
                    w:leave_fullscreen()
                elseif w.maximised then
                    w:unmaximise()
                else
                    w:unsnap()
                end
            end,
        } or {
            label = "Maximise",
            run = function(w)
                w:maximise()
            end,
        },
        {
            label = "Minimise",
            run = function(w)
                w:minimise()
            end,
        },
        { separator = true },
        {
            label = "Move",
            enabled = floating and self.move_with_keys ~= nil,
            run = function(w)
                w:move_with_keys()
            end,
        },
        {
            label = "Resize",
            enabled = floating and self.resize_with_keys ~= nil,
            run = function(w)
                w:resize_with_keys()
            end,
        },
        {
            label = "Always on top",
            checked = self.data.above == true,
            run = function(w)
                w:set_above(not w.data.above)
            end,
        },
        {
            label = "Move to workspace",
            enabled = not (self.fullscreen or self.parent_window),
            submenu = function(w)
                local entries = {}
                local d = w:closest(Desk)
                for _, ws in ipairs(d and d.space.children or {}) do
                    entries[#entries + 1] = {
                        label = ws.data.title or ws.name,
                        checked = ws == current_workspace,
                        enabled = ws ~= current_workspace,
                        run = function(window)
                            window:move_to(ws)
                        end,
                    }
                end
                return entries
            end,
        },
        { separator = true },
        {
            label = "Close",
            run = function(w)
                w:close()
            end,
        },
    }
    return entries
end

-- Opens the window's menu with its corner at (x, y) in output coordinates; the pointer when none is given. Replace
-- this method to draw a different menu: the requests above all end here.
function Window:show_menu(x, y)
    return M.open(self, x or seat.pointer_x, y or seat.pointer_y)
end

-- Always on top. Raising keeps such windows over the ones that are not (see Window:raise).
function Window:set_above(on)
    self.data.above = on and true or nil
    if on then
        self:raise()
    end
end

function Window:on_request(e)
    if e.request == "menu" then
        self:show_menu(e.x, e.y)
    end
end

-- A menu for a window that is going closes with it.
local function closes_with(window)
    if current and current.window == window then
        M.close()
    end
end
function Window:on_closed()
    closes_with(self)
end
function Window:on_destroyed()
    closes_with(self)
end
function Lock:on_requested()
    M.close()
end

return M
