-- tests/lua/menus_test.lua: the stock window menu (data/lua/gnoblin/shell/menus.lua) over the fake core. A `menu`
-- request is what the compositor sends for a client's show_window_menu, the keyboard's window-menu key and a program's
-- request_menu; the titlebar's right click asks the same way. The keys and presses are delivered as the real path
-- delivers them: a key to the node that holds the seat focus, a press to the node it lands on.
local H = require("harness")

local LEFT, RIGHT = 272, 273

-- A started shell with one Screen (1000 by 800), one Seat and `n` windows. Returns the host, the windows in creation
-- order, and the menus module.
local function setup(n)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Screen({ width = 1000, height = 800 }))
    root:add(Lock({}))
    local wins = {}
    for i = 1, n or 1 do
        wins[i] = root:add(Window({ app_id = "foot", title = "win" .. i }))
    end
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    H.start_shell(host)
    H.frame(host, 1, {})
    return host, wins, require("gnoblin.shell.menus")
end

local clock = 1
local function frame(host, events)
    clock = clock + 1
    H.frame(host, clock, events or {})
end

-- The compositor's menu request for `win`, anchored at (x, y).
local function ask(host, win, x, y)
    frame(host, { { type = "request", bubbles = true, target = win.id, request = "menu", x = x, y = y } })
end

local function key(node, name, text)
    node.key:emit({ key = name, text = text or "", pressed = true })
end

local function press(host, node, button)
    frame(host, { { type = "press", bubbles = true, target = node.id, button = button or LEFT, x = 5, y = 5 } })
end

local function labels(rows)
    local out = {}
    for _, r in ipairs(rows) do
        out[#out + 1] = r.label
    end
    return out
end

local function selected(menu)
    for _, r in ipairs(menu:active()) do
        if r.selected then
            return r.label
        end
    end
end

local function row(menu, label)
    for _, r in ipairs(menu.rows) do
        if r.label == label then
            return r
        end
    end
    error("no entry " .. label)
end

return {
    {
        "a menu request opens the window menu with its corner at the anchor and the keys going to it",
        function()
            local host, wins, Menus = setup()
            local before = seat.focus
            ask(host, wins[1], 300, 200)
            local menu = Menus.current()
            H.check(menu ~= nil, "a menu request must open the menu")
            H.check(menu.window == wins[1], "the menu belongs to the window that asked")
            H.check(menu.panel.x == 300 and menu.panel.y == 200, "the corner is the anchor")
            H.check(seat.focus == menu, "the keys go to the menu")
            H.check(
                table.concat(labels(menu.rows), ",")
                    == "Maximise,Minimise,Move,Resize,Always on top,Move to workspace,Close",
                "the entries, got " .. table.concat(labels(menu.rows), ",")
            )
            H.check(menu.before == before, "the menu remembers where the keyboard was")
        end,
    },
    {
        "a menu near the screen's edge is moved back inside it",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 990, 790)
            local panel = Menus.current().panel
            H.check(panel.x + panel.width <= 1000, "the card ends inside the screen, got x " .. panel.x)
            H.check(panel.y + panel.height <= 800, "the card ends inside the screen, got y " .. panel.y)
            H.check(panel.x > 0 and panel.y > 0, "and still holds its whole card")
        end,
    },
    {
        "the card is as wide as its longest label and no narrower than the minimum",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 10, 10)
            local menu = Menus.current()
            H.check(menu.panel.width >= 170, "never narrower than the minimum, got " .. menu.panel.width)
            for _, r in ipairs(menu.rows) do
                H.check(r:natural() + 8 <= menu.panel.width, r.label .. " fits its card")
            end
        end,
    },
    {
        "Down and Return run Maximise: the window maximises, the menu closes and the keyboard goes back",
        function()
            local host, wins, Menus = setup()
            local before = seat.focus
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            key(menu, "Down")
            H.check(
                selected(menu) == "Maximise",
                "Down from nothing lands on the first entry, got " .. tostring(selected(menu))
            )
            key(menu, "Return")
            H.check(wins[1].maximised == true, "Return runs the selected entry")
            H.check(Menus.current() == nil, "running an entry closes the menu")
            H.check(#desk():all(WindowMenu) == 0, "and takes it off the desk")
            H.check(seat.focus == before, "the keyboard goes back to where it was")
        end,
    },
    {
        "Up from nothing lands on the last entry and Home and End go to the ends",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            key(menu, "Up")
            H.check(selected(menu) == "Close", "got " .. tostring(selected(menu)))
            key(menu, "Home")
            H.check(selected(menu) == "Maximise", "got " .. tostring(selected(menu)))
            key(menu, "End")
            H.check(selected(menu) == "Close", "got " .. tostring(selected(menu)))
            key(menu, "Tab")
            H.check(selected(menu) == "Maximise", "Tab goes on, and wraps, got " .. tostring(selected(menu)))
            key(menu, "ISO_Left_Tab")
            H.check(selected(menu) == "Close", "Shift+Tab goes back, got " .. tostring(selected(menu)))
        end,
    },
    {
        "a letter jumps to the next entry that starts with it",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            key(menu, "c", "c")
            H.check(selected(menu) == "Close", "c is Close, got " .. tostring(selected(menu)))
            key(menu, "m", "m")
            H.check(selected(menu) == "Maximise", "m wraps to Maximise, got " .. tostring(selected(menu)))
            key(menu, "m", "m")
            H.check(selected(menu) == "Minimise", "and on to the next m, got " .. tostring(selected(menu)))
        end,
    },
    {
        "Escape closes the menu and the keyboard goes back",
        function()
            local host, wins, Menus = setup()
            local before = seat.focus
            ask(host, wins[1], 50, 50)
            key(Menus.current(), "Escape")
            H.check(Menus.current() == nil, "Escape closes the menu")
            H.check(seat.focus == before, "the keyboard goes back")
            H.check(wins[1].maximised == false, "and nothing ran")
        end,
    },
    {
        "a key release does nothing, and a key the menu does not use does not leave it",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            menu.key:emit({ key = "Escape", text = "", pressed = false })
            H.check(Menus.current() == menu, "a release of Escape is not Escape")
            key(menu, "F5")
            H.check(Menus.current() == menu, "an unused key leaves the menu open")
        end,
    },
    {
        "a press outside the card closes the menu and is taken, and one on the card's padding does not",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            press(host, menu.panel)
            H.check(Menus.current() == menu, "a press on the card's padding is not a click away")
            press(host, menu)
            H.check(Menus.current() == nil, "a press on the layer closes the menu")
            H.check(wins[1].maximised == false, "and runs nothing")
            ask(host, wins[1], 50, 50)
            press(host, Menus.current(), RIGHT)
            H.check(Menus.current() == nil, "a right press outside closes it too")
        end,
    },
    {
        "a left press on an entry runs it and a right press does not",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            press(host, row(menu, "Maximise"), RIGHT)
            H.check(Menus.current() == menu and not wins[1].maximised, "a right press runs nothing")
            press(host, row(menu, "Maximise"), LEFT)
            H.check(wins[1].maximised == true, "a left press runs the entry")
            H.check(Menus.current() == nil, "and closes the menu")
        end,
    },
    {
        "the pointer over an entry selects it, and a disabled entry cannot be selected or run",
        function()
            local host, wins, Menus = setup()
            wins[1]:maximise()
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            frame(host, { { type = "enter", target = row(menu, "Minimise").id, x = 4, y = 4 } })
            H.check(selected(menu) == "Minimise", "the pointer selects what it is over")
            H.check(row(menu, "Move").enabled == false, "a maximised window cannot be moved")
            frame(host, { { type = "enter", target = row(menu, "Move").id, x = 4, y = 4 } })
            H.check(selected(menu) == "Minimise", "a disabled entry is not selected")
            press(host, row(menu, "Move"))
            H.check(Menus.current() == menu, "pressing a disabled entry runs nothing and keeps the menu")
        end,
    },
    {
        "the keyboard skips disabled entries",
        function()
            local host, wins, Menus = setup()
            wins[1]:maximise()
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            H.check(row(menu, "Restore") ~= nil and #menu.rows == 7, "a maximised window offers Restore")
            key(menu, "Down") -- Restore
            key(menu, "Down") -- Minimise
            key(menu, "Down") -- skips Move and Resize
            H.check(selected(menu) == "Always on top", "got " .. tostring(selected(menu)))
            key(menu, "Up")
            H.check(selected(menu) == "Minimise", "and back over them, got " .. tostring(selected(menu)))
        end,
    },
    {
        "Restore returns a maximised window to the rectangle it had",
        function()
            local host, wins, Menus = setup()
            local w = wins[1]
            local x, y, width, height = w.x, w.y, w.width, w.height
            w:maximise()
            ask(host, w, 50, 50)
            key(Menus.current(), "Down")
            key(Menus.current(), "Return")
            H.check(not w.maximised, "Restore leaves maximise")
            H.check(w.x == x and w.y == y and w.width == width and w.height == height, "and gets the rectangle back")
        end,
    },
    {
        "Minimise minimises the window",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            key(Menus.current(), "Down")
            key(Menus.current(), "Down")
            key(Menus.current(), "Return")
            H.check(wins[1].minimised == true, "Minimise minimises")
        end,
    },
    {
        "Close asks the window to close",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            key(Menus.current(), "End")
            key(Menus.current(), "Return")
            local closed = false
            for _, c in ipairs(host.calls) do
                closed = closed or (c.method == "close" and c.id == wins[1].id)
            end
            H.check(closed, "Close calls the window's close")
            H.check(Menus.current() == nil, "and the menu is gone")
        end,
    },
    {
        "Always on top keeps the window over one that is raised later, and takes it back off",
        function()
            local host, wins, Menus = setup(2)
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            H.check(row(menu, "Always on top").checked == false, "it starts unchecked")
            key(menu, "End")
            key(menu, "Up") -- Move to workspace
            key(menu, "Up") -- Always on top
            key(menu, "Return")
            H.check(wins[1].data.above == true, "the window is on top")
            wins[2]:raise()
            local kids = wins[1].parent.children
            H.check(kids[#kids] == wins[1], "a raised window lands under it")
            ask(host, wins[1], 50, 50)
            H.check(row(Menus.current(), "Always on top").checked == true, "the entry shows the tick")
            key(Menus.current(), "End")
            key(Menus.current(), "Up")
            key(Menus.current(), "Up")
            key(Menus.current(), "Return")
            H.check(wins[1].data.above == nil, "the entry takes it off again")
            wins[2]:raise()
            kids = wins[1].parent.children
            H.check(kids[#kids] == wins[2], "and a raised window lands on top again")
        end,
    },
    {
        "a fullscreen window stays over an always-on-top one",
        function()
            local host, wins = setup(3)
            wins[1]:set_above(true)
            wins[3]:enter_fullscreen()
            wins[2]:raise()
            local kids = wins[1].parent.children
            H.check(kids[#kids] == wins[3], "fullscreen keeps the top")
            H.check(kids[#kids - 1] == wins[1], "then the pinned window, over the raised one")
        end,
    },
    {
        "Move to workspace opens a submenu of the workspaces: Right opens it, Return moves the window, Left and Escape go back",
        function()
            local host, wins, Menus = setup()
            local w = wins[1]
            ask(host, w, 50, 50)
            local menu = Menus.current()
            key(menu, "End")
            key(menu, "Up")
            H.check(selected(menu) == "Move to workspace", "got " .. tostring(selected(menu)))
            key(menu, "Right")
            H.check(menu.depth == 2, "Right opens the submenu")
            H.check(#menu.sub_rows == 9, "one entry for each workspace, got " .. #menu.sub_rows)
            H.check(
                menu.sub_rows[1].enabled == false and menu.sub_rows[1].checked,
                "the workspace the window is on is ticked and cannot be chosen"
            )
            H.check(menu.sub.x >= menu.panel.x + menu.panel.width - 2, "the submenu opens beside the card")
            H.check(selected(menu) == menu.sub_rows[2].label, "the selection is on the first entry that can be chosen")
            key(menu, "Left")
            H.check(menu.depth == 1 and selected(menu) == "Move to workspace", "Left goes back to the card")
            key(menu, "Right")
            key(menu, "Escape")
            H.check(menu.depth == 1 and Menus.current() == menu, "Escape closes the submenu and not the menu")
            key(menu, "Right")
            key(menu, "Return")
            H.check(w:closest(Workspace).name == "2", "Return moves the window to that workspace")
            H.check(Menus.current() == nil, "and closes the menu")
        end,
    },
    {
        "Move to workspace is off for a fullscreen window",
        function()
            local host, wins, Menus = setup()
            wins[1]:enter_fullscreen()
            ask(host, wins[1], 50, 50)
            H.check(row(Menus.current(), "Move to workspace").enabled == false, "a fullscreen window stays where it is")
        end,
    },
    {
        "Move and Resize start the keyboard modes, and the menu has closed first",
        function()
            local host, wins, Menus = setup()
            local w = wins[1]
            ask(host, w, 50, 50)
            local menu = Menus.current()
            key(menu, "Down")
            key(menu, "Down")
            key(menu, "Down")
            H.check(selected(menu) == "Move", "got " .. tostring(selected(menu)))
            key(menu, "Return")
            H.check(Menus.current() == nil, "the menu is closed")
            local grab = desk():all(RepositionGrab)[1]
            H.check(grab ~= nil and seat.focus == grab, "and the mode holds the keyboard")
            local x = w.x
            key(grab, "Right")
            H.check(w.x == x + 16, "an arrow moves the window")
        end,
    },
    {
        "two menus never stack: asking for a second closes the first",
        function()
            local host, wins, Menus = setup(2)
            ask(host, wins[1], 50, 50)
            ask(host, wins[2], 400, 300)
            H.check(#desk():all(WindowMenu) == 1, "one menu on the desk, got " .. #desk():all(WindowMenu))
            H.check(Menus.current().window == wins[2], "the newer menu is the one that is open")
            ask(host, wins[2], 10, 10)
            H.check(#desk():all(WindowMenu) == 1, "asking again for the same window does not add one")
            H.check(Menus.current().panel.x == 10, "it opens where it was asked")
            H.check(seat.focus == Menus.current(), "the keys go to the newest menu")
        end,
    },
    {
        "the focus goes back to where it was before the first of two menus, not into the first",
        function()
            local host, wins, Menus = setup(2)
            local before = seat.focus
            ask(host, wins[1], 50, 50)
            ask(host, wins[2], 50, 50)
            key(Menus.current(), "Escape")
            H.check(
                seat.focus ~= nil and seat.focus:is(Window) == (before ~= nil and before:is(Window)),
                "a window or nothing"
            )
            H.check(#desk():all(WindowMenu) == 0, "no menu is left")
        end,
    },
    {
        "the menu closes when its window closes, and a window that closes later takes no menu with it",
        function()
            local host, wins, Menus = setup(2)
            ask(host, wins[1], 50, 50)
            host.close(wins[2].id)
            frame(host)
            H.check(Menus.current() ~= nil, "another window going leaves the menu")
            host.close(wins[1].id)
            frame(host)
            H.check(Menus.current() == nil, "its own window going closes it")
            H.check(#desk():all(WindowMenu) == 0, "and leaves nothing on the desk")
        end,
    },
    {
        "a menu for a secure window is secure and one for any other is not",
        function()
            local host, wins, Menus = setup(2)
            wins[1].secure = true
            ask(host, wins[1], 50, 50)
            H.check(Menus.current().panel.secure == true, "the card is secure")
            H.check(Menus.current().secure == false, "the transparent layer is not, or a capture would lose the desk")
            ask(host, wins[2], 50, 50)
            H.check(Menus.current().panel.secure == false, "a card for an ordinary window is not")
            ask(host, wins[1], 50, 50)
            row(Menus.current(), "Move to workspace")
            Menus.current():open_submenu(row(Menus.current(), "Move to workspace"))
            H.check(Menus.current().sub.secure == true, "nor is its submenu")
        end,
    },
    {
        "the menu names itself and its entries, and tags them for selectors",
        function()
            local host, wins, Menus = setup()
            wins[1].maximised = false
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            H.check(menu.panel.name == "window-menu", "the card is window-menu, got " .. menu.panel.name)
            local first = menu.rows[1]
            H.check(first.name == "Maximise", "an entry is named for its label, got " .. first.name)
            local function has(node, tag)
                for _, t in ipairs(node.tags) do
                    if t == tag then
                        return true
                    end
                end
                return false
            end
            H.check(has(first, "menuitem") and not has(first, "disabled"), "a live entry is a menuitem")
            H.check(has(row(menu, "Move to workspace"), "submenu"), "an entry with a submenu says so")
            wins[1]:maximise()
            ask(host, wins[1], 50, 50)
            H.check(has(row(Menus.current(), "Move"), "disabled"), "a disabled entry says so")
            H.check(desk():find("#window-menu") == Menus.current().panel, "the name finds the card")
        end,
    },
    {
        "the session locking closes the menu, and no menu opens while it is locked",
        function()
            local host, wins, Menus = setup()
            ask(host, wins[1], 50, 50)
            H.check(Menus.current() ~= nil, "a menu is open")
            host.external(lock.id, "state", "pending")
            frame(host, { { type = "requested", target = lock.id } })
            H.check(Menus.current() == nil, "the lock request closes it")
            lock.engaged = true
            ask(host, wins[1], 50, 50)
            H.check(Menus.current() == nil, "nothing opens over the lock screen")
            lock.engaged = false
            host.external(lock.id, "state", "not-locked")
            ask(host, wins[1], 50, 50)
            H.check(Menus.current() ~= nil, "and the menu is back once it has gone")
        end,
    },
    {
        "a menu for a window that is not showing is refused",
        function()
            local host, wins, Menus = setup()
            wins[1]:minimise()
            ask(host, wins[1], 50, 50)
            H.check(Menus.current() == nil, "a minimised window has no menu")
        end,
    },
    {
        "the titlebar's right click opens the menu at the pointer, and its middle click does not",
        function()
            local host, wins, Menus = setup()
            local seat_id = host.ids("Seat")[1]
            host.external(seat_id, "pointer_x", 120)
            host.external(seat_id, "pointer_y", 90)
            frame(host)
            local bar = wins[1]:all(Titlebar)[1]
            frame(host, { { type = "press", bubbles = true, target = bar.id, button = 274, x = 5, y = 5 } })
            H.check(Menus.current() == nil, "a middle click lowers, it does not open a menu")
            frame(host, { { type = "press", bubbles = true, target = bar.id, button = RIGHT, x = 5, y = 5 } })
            H.check(Menus.current() ~= nil, "the right click opens the menu")
            H.check(Menus.current().panel.x == 120 and Menus.current().panel.y == 90, "at the pointer")
        end,
    },
    {
        "the titlebar's right click asks the compositor for nothing when the seat's action is something else",
        function()
            local host, wins, Menus = setup()
            host.external(host.ids("Seat")[1], "right_click", "none")
            frame(host)
            local bar = wins[1]:all(Titlebar)[1]
            frame(host, { { type = "press", bubbles = true, target = bar.id, button = RIGHT, x = 5, y = 5 } })
            H.check(Menus.current() == nil, "right_click = none opens no menu")
        end,
    },
    {
        "a shell file that replaces menu_items changes what the menu offers",
        function()
            local host, wins, Menus = setup()
            local ran
            function Window:menu_items()
                return {
                    {
                        label = "Only",
                        run = function(w)
                            ran = w
                        end,
                    },
                    { separator = true },
                    { label = "Off", enabled = false },
                }
            end
            ask(host, wins[1], 50, 50)
            local menu = Menus.current()
            H.check(table.concat(labels(menu.rows), ",") == "Only,Off", "the shell's own entries")
            key(menu, "Down")
            key(menu, "Return")
            H.check(ran == wins[1], "and they run on the window")
        end,
    },
    {
        "a shell file that replaces show_menu draws its own menu, and every request reaches it",
        function()
            local host, wins = setup()
            local seen
            function Window:show_menu(x, y)
                seen = { self, x, y }
            end
            ask(host, wins[1], 70, 80)
            H.check(seen and seen[1] == wins[1] and seen[2] == 70 and seen[3] == 80, "a menu request ends in show_menu")
            H.check(#desk():all(WindowMenu) == 0, "and the stock menu is not drawn")
        end,
    },
    {
        "a menu entry that raises is reported and the menu is already closed",
        function()
            local host, wins, Menus = setup()
            function Window:menu_items()
                return {
                    {
                        label = "Boom",
                        run = function()
                            error("boom")
                        end,
                    },
                }
            end
            ask(host, wins[1], 50, 50)
            key(Menus.current(), "Down")
            key(Menus.current(), "Return")
            H.check(Menus.current() == nil, "the menu is closed before the entry runs")
            local found = false
            for _, r in ipairs(host.reports) do
                found = found or r:find("boom", 1, true) ~= nil
            end
            H.check(found, "the error is reported")
        end,
    },
    {
        "a request that names no anchor opens at the pointer",
        function()
            local host, wins, Menus = setup()
            local seat_id = host.ids("Seat")[1]
            host.external(seat_id, "pointer_x", 220)
            host.external(seat_id, "pointer_y", 140)
            frame(host)
            wins[1]:show_menu()
            H.check(Menus.current().panel.x == 220 and Menus.current().panel.y == 140, "at the pointer")
        end,
    },
}
