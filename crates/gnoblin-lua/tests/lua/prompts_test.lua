-- tests/lua/prompts_test.lua: the stock consent and passphrase dialogs (data/lua/gnoblin/shell/prompts.lua) over the
-- fake core. The fake's `host.offer` plays the compositor offering a request to a slot the shell claimed, and
-- `host.answers` and `host.declines` are what the shell sent back.
local H = require("harness")

local LEFT = 272

-- A shell with one Screen (1000 by 800) and one Seat, loaded and not yet started. `option` is what a shell file would
-- assign to `seat.data.prompts`.
local function loaded(option)
    local core, host = require("fake_core")()
    H.boot(core, host)
    root:add(Seat({}))
    root:add(Screen({ width = 1000, height = 800 }))
    H.load_shell()
    H.publish_seat(host, { screen = host.ids("Screen")[1] })
    if option ~= nil then
        seat.data.prompts = option
    end
    return host
end

-- The same, started: the first frame has run, so the claim has been made.
local function started(option)
    local host = loaded(option)
    H.start_shell(host)
    return host
end

local function key(node, name, text)
    node.key:emit({ key = name, text = text or "", pressed = true })
end

-- The texts a node shows, in tree order.
local function texts(node)
    local out = {}
    for _, t in ipairs(node:all(Text)) do
        out[#out + 1] = t.text
    end
    return out
end

local function shows(node, wanted)
    for _, t in ipairs(texts(node)) do
        if t == wanted then
            return true
        end
    end
    return false
end

-- Offers a consent request and runs the frame that draws it. Returns the dialog.
local function consent(host, id, spec)
    spec = spec or {}
    spec.request = id
    host.offer(spec)
    H.frame(host, spec.at or 0.1)
    return desk():all(ConsentDialog)[1]
end

return {
    {
        "the stock shell claims portal.ui and prompt.secret on its first frame",
        function()
            local host = started()
            H.check(host.claimed["portal.ui"], "portal.ui must be claimed")
            H.check(host.claimed["prompt.secret"], "prompt.secret must be claimed")
        end,
    },
    {
        "seat.data.prompts = false claims nothing",
        function()
            local host = started(false)
            H.check(not next(host.claimed), "no slot may be claimed")
        end,
    },
    {
        "seat.data.prompts can turn off one prompt and keep the other",
        function()
            local host = started({ portal = false })
            H.check(not host.claimed["portal.ui"] and host.claimed["prompt.secret"], "only the secret prompt is drawn")
            host = started({ secret = false })
            H.check(host.claimed["portal.ui"] and not host.claimed["prompt.secret"], "only the consent dialog is drawn")
        end,
    },
    {
        "a claim the compositor refuses is reported and the shell goes on",
        function()
            local host = loaded()
            host.refuse = "denied: portal.ui is held by another shell"
            H.start_shell(host)
            H.check(not next(host.claimed), "nothing is held")
            local reported = false
            for _, r in ipairs(host.reports) do
                reported = reported or r:find("built-in dialog serves", 1, true) ~= nil
            end
            H.check(reported, "the refusal must be reported, and say the built-in dialog serves")
        end,
    },
    {
        "a consent offer draws a dialog that names the request, the caller and the question",
        function()
            local host = started()
            local d = consent(host, 7, { title = "Screenshot", message = "Allow a screenshot of the whole screen?" })
            H.check(d, "the dialog must exist")
            H.check(shows(d, "Screenshot"), "the title is the interface that asked")
            H.check(shows(d, "Asked by org.example.Requester"), "the identity is the app id the portal verified")
            H.check(shows(d, "Allow a screenshot of the whole screen?"), "the question is shown")
            H.check(shows(d, "Deny") and shows(d, "Allow"), "both buttons are shown")
            H.check(host.request_claims[1] == 7, "the shell claimed the request")
            H.check(d.secure, "the dialog is secure, so a View capture masks it")
            H.check(d.parent.parent == desk(), "it is drawn on the desk, above its windows")
        end,
    },
    {
        "a caller the portal could not name is said so in the dialog",
        function()
            local host = started()
            local d = consent(host, 7, { subject = "" })
            H.check(
                shows(d, "Asked by an application that could not be identified"),
                "an empty identity must be said, not hidden"
            )
        end,
    },
    {
        "text from the peer is shown cleaned, never as markup or control characters",
        function()
            local host = started()
            local d = consent(host, 7, { title = "Screen\nshot", subject = "evil\27[31m.app", message = "ok" })
            for _, t in ipairs(texts(d)) do
                H.check(not t:find("%c"), "no control character may reach the screen: " .. t)
            end
            host.offer({ request = 8, subject = "\255\254" })
            H.frame(host, 0.2) -- bytes that are not UTF-8 are queued behind the first dialog and must not break it
        end,
    },
    {
        "a long message wraps onto lines the dialog has room for",
        function()
            local host = started()
            local long = string.rep("share ", 40)
            local d = consent(host, 7, { message = long })
            local lines = 0
            for _, t in ipairs(texts(d)) do
                H.check(utf8.len(t) <= 80, "a line must fit the dialog")
                if t:find("^share") then
                    lines = lines + 1
                end
            end
            H.check(lines >= 4, "the message must take several lines, got " .. lines)
        end,
    },
    {
        "Deny is the focused button when the dialog opens and the dialog holds the keyboard",
        function()
            local host = started()
            local d = consent(host, 7)
            H.check(d.index == 1, "the first button is Deny")
            H.check(d.choices[1].label == "Deny" and d.choices[1].focused, "Deny is focused")
            H.check(not d.choices[2].focused, "Allow is not")
            H.check(seat.focus == d, "the dialog holds the seat's focus, so keys reach it")
        end,
    },
    {
        "Escape denies at once, closes the dialog and gives the focus back",
        function()
            local host = started()
            local keeper = root:add(Node({}))
            seat.focus = keeper
            local d = consent(host, 7)
            key(d, "Escape")
            H.check(#host.answers == 1 and host.answers[1].text == "deny", "Escape must answer deny")
            H.check(#desk():all(ConsentDialog) == 0, "the dialog must close")
            H.check(seat.focus == keeper, "the focus must return to what held it")
        end,
    },
    {
        "Return on the default button denies, even after the dialog is armed",
        function()
            local host = started()
            local d = consent(host, 7)
            H.frame(host, 0.7)
            key(d, "Return")
            H.check(host.answers[1] and host.answers[1].text == "deny", "Deny is what Return presses by default")
        end,
    },
    {
        "Tab moves to Allow, and Allow does nothing until the dialog is armed",
        function()
            local host = started()
            local d = consent(host, 7, { at = 0.1 })
            key(d, "Tab")
            H.check(d.index == 2 and d.choices[2].focused and not d.choices[1].focused, "Tab must focus Allow")
            H.check(not d.choices[2].enabled, "Allow looks disabled while it is not armed")
            key(d, "Return")
            H.check(#host.answers == 0, "an allow inside the arming window must send nothing, not even a refusal")
            H.check(#desk():all(ConsentDialog) == 1, "and the dialog stays")
            H.frame(host, 0.3)
            key(d, "space")
            H.check(#host.answers == 0, "still inside the window at 0.3 s")
            H.frame(host, 0.7)
            H.check(d.choices[2].enabled, "armed after half a second")
            key(d, "Return")
            H.check(#host.answers == 1 and host.answers[1].text == "allow", "Allow answers allow once armed")
            H.check(#desk():all(ConsentDialog) == 0, "and the dialog closes")
        end,
    },
    {
        "Tab twice, or Left and Right, move between the two buttons",
        function()
            local host = started()
            local d = consent(host, 7)
            key(d, "Right")
            H.check(d.index == 2, "Right")
            key(d, "Left")
            H.check(d.index == 1, "Left")
            key(d, "ISO_Left_Tab")
            key(d, "Tab")
            H.check(d.index == 1, "two steps come back to Deny")
        end,
    },
    {
        "a key coming up does nothing",
        function()
            local host = started()
            local d = consent(host, 7)
            d.key:emit({ key = "Escape", pressed = false })
            H.check(#host.answers == 0 and #desk():all(ConsentDialog) == 1, "only a press acts")
        end,
    },
    {
        "other keys are left alone",
        function()
            local host = started()
            local d = consent(host, 7)
            key(d, "a", "a")
            H.check(#host.answers == 0 and d.index == 1, "typing does nothing in a consent dialog")
        end,
    },
    {
        "a click on Allow inside the arming window is ignored, and a click on Deny denies",
        function()
            local host = started()
            local d = consent(host, 7)
            d.choices[2].press:emit({ button = LEFT })
            H.check(#host.answers == 0, "an early click must not answer")
            d.choices[1].press:emit({ button = LEFT })
            H.check(host.answers[1] and host.answers[1].text == "deny", "Deny is always live")
        end,
    },
    {
        "a click on Allow after the arming window allows",
        function()
            local host = started()
            local d = consent(host, 7)
            H.frame(host, 0.7)
            d.choices[2].press:emit({ button = LEFT })
            H.check(host.answers[1] and host.answers[1].text == "allow", "the click must allow")
        end,
    },
    {
        "a right click on a button does nothing",
        function()
            local host = started()
            local d = consent(host, 7)
            H.frame(host, 0.7)
            d.choices[2].press:emit({ button = 273 })
            H.check(#host.answers == 0, "only the left button presses")
        end,
    },
    {
        "the compositor ending the request closes the dialog without an answer",
        function()
            local host = started()
            local d = consent(host, 7)
            host.finish(7, "ended")
            H.frame(host, 0.2)
            H.check(#desk():all(ConsentDialog) == 0, "nobody waits for the dialog any more")
            H.check(#host.answers == 0 and #host.declines == 0, "and nothing is sent for a request that is over")
        end,
    },
    {
        "a second request waits for the first dialog and is drawn when it closes",
        function()
            local host = started()
            local first = consent(host, 7)
            host.offer({ request = 8, message = "Second question?" })
            H.frame(host, 0.2)
            H.check(#desk():all(ConsentDialog) == 1, "one dialog at a time")
            H.check(not shows(first, "Second question?"), "the first dialog is still the first")
            key(first, "Escape")
            H.frame(host, 0.4)
            local second = desk():all(ConsentDialog)[1]
            H.check(second and shows(second, "Second question?"), "the next request is drawn once the first closes")
            key(second, "Escape")
            H.check(host.answers[2] and host.answers[2].request == 8, "and answered under its own request")
        end,
    },
    {
        "a request that ends while it waits is never drawn",
        function()
            local host = started()
            local first = consent(host, 7)
            host.offer({ request = 8 })
            H.frame(host, 0.2)
            host.finish(8, "timed-out")
            H.frame(host, 0.3)
            key(first, "Escape")
            H.frame(host, 0.6)
            H.check(#desk():all(ConsentDialog) == 0, "the second request is over, so it has no dialog")
        end,
    },
    {
        "a dialog that cannot be built declines the request, and the next one is still served",
        function()
            local host = started()
            local build = require("gnoblin.shell.prompts").build_consent
            local prompts = require("gnoblin.shell.prompts")
            prompts.build_consent = function()
                error("no room")
            end
            host.offer({ request = 7 })
            H.frame(host, 0.1)
            H.check(host.declines[1] == 7, "an undrawable prompt must be declined so the built-in dialog serves it")
            prompts.build_consent = build
            local d = consent(host, 8, { at = 0.2 })
            H.check(d, "a failure must not leave the next request without a dialog")
        end,
    },
    {
        "with no screen to draw on the request is declined",
        function()
            local _, host = H.fresh()
            root:add(Seat({}))
            H.load_shell()
            H.start_shell(host) -- a seat and no screen: there is no desk to draw on
            host.offer({ request = 7 })
            H.frame(host, 0.1)
            H.check(host.declines[1] == 7, "no desk, no dialog: decline")
        end,
    },
    {
        "the wrap helper breaks at spaces and cuts a word that is longer than a line",
        function()
            local P = require("gnoblin.shell.prompts")
            local lines = P.wrap("one two three four", 9)
            H.check(
                #lines == 3 and lines[1] == "one two" and lines[2] == "three" and lines[3] == "four",
                "wrapped at spaces"
            )
            local cut = P.wrap(string.rep("x", 25), 10)
            H.check(
                #cut == 3 and cut[1] == string.rep("x", 10) and cut[3] == string.rep("x", 5),
                "an unbroken word is cut"
            )
            H.check(#P.wrap("", 10) == 0, "nothing wraps to nothing")
        end,
    },
    {
        "the clean helper drops control characters, cuts long text on a character boundary and names bad UTF-8",
        function()
            local P = require("gnoblin.shell.prompts")
            H.check(P.clean("a\nb\27c", 10) == "a b c", "controls become spaces")
            H.check(P.clean(string.rep("é", 10), 4) == "éééé…", "cut on a character, not a byte")
            H.check(P.clean("\255", 10) == "(unreadable)", "invalid UTF-8 is not shown")
        end,
    },
    {
        "a passphrase offer draws an entry that shows asterisks, and Return answers the text once",
        function()
            local host = started()
            host.offer({
                request = 9,
                slot = "prompt.secret",
                title = "Unlock the keyring",
                message = "Enter your login password.",
                subject = "",
            })
            H.frame(host, 0.1)
            local d = desk():all(PassphraseDialog)[1]
            H.check(d, "the dialog must exist")
            H.check(shows(d, "Unlock the keyring") and shows(d, "Enter your login password."), "title and message")
            H.check(
                not shows(d, "Asked by an application that could not be identified"),
                "a keyring prompt names no caller"
            )
            H.frame(host, 0.7) -- armed: the entry now holds the focus
            H.check(seat.focus == d.entry, "typing goes to the entry once the dialog is armed")
            for _, c in ipairs({ "s", "e", "c", "r", "e", "t" }) do
                d.entry.key:emit({ key = c, text = c, pressed = true })
            end
            H.check(d.entry.text == "secret", "the entry holds what was typed")
            local shown = {}
            for _, t in ipairs(texts(d.entry)) do
                shown[#shown + 1] = t
            end
            H.check(
                table.concat(shown) == "******",
                "the entry shows asterisks, never the text: " .. table.concat(shown)
            )
            key(d, "Return")
            H.check(
                #host.answers == 1 and host.answers[1].text == "secret" and host.answers[1].request == 9,
                "Return answers the passphrase"
            )
            H.check(#desk():all(PassphraseDialog) == 0, "and closes the dialog")
        end,
    },
    {
        "typing in a passphrase dialog before it is armed goes nowhere",
        function()
            local host = started()
            host.offer({ request = 9, slot = "prompt.secret", message = "Password?" })
            H.frame(host, 0.1)
            local d = desk():all(PassphraseDialog)[1]
            H.check(seat.focus == d, "the dialog, not the entry, holds the keys while it is not armed")
            key(d, "Return")
            H.check(#host.answers == 0, "Return before arming answers nothing")
        end,
    },
    {
        "an empty passphrase is not sent",
        function()
            local host = started()
            host.offer({ request = 9, slot = "prompt.secret", message = "Password?" })
            H.frame(host, 0.1)
            H.frame(host, 0.7)
            local d = desk():all(PassphraseDialog)[1]
            key(d, "Return")
            H.check(#host.answers == 0 and #desk():all(PassphraseDialog) == 1, "Return on an empty entry does nothing")
        end,
    },
    {
        "Escape and the Cancel button decline the passphrase request",
        function()
            local host = started()
            host.offer({ request = 9, slot = "prompt.secret", message = "Password?" })
            H.frame(host, 0.1)
            local d = desk():all(PassphraseDialog)[1]
            key(d, "Escape")
            H.check(host.declines[1] == 9 and #host.answers == 0, "Escape must decline, never answer")
            H.check(#desk():all(PassphraseDialog) == 0, "and close")
            host.offer({ request = 10, slot = "prompt.secret", message = "Password?" })
            H.frame(host, 0.3)
            d = desk():all(PassphraseDialog)[1]
            d.choices[1].press:emit({ button = LEFT })
            H.check(host.declines[2] == 10, "the Cancel button declines")
        end,
    },
    {
        "the passphrase is in no node property and no report, and the dialog's text is gone when it closes",
        function()
            local host = started()
            host.offer({ request = 9, slot = "prompt.secret", message = "Password?" })
            H.frame(host, 0.1)
            H.frame(host, 0.7)
            local d = desk():all(PassphraseDialog)[1]
            for _, c in ipairs({ "h", "u", "n", "t", "e", "r", "2" }) do
                d.entry.key:emit({ key = c, text = c, pressed = true })
            end
            H.frame(host, 0.8)
            local seen = host.dump()
            H.check(not seen:find("hunter2", 1, true), "the mirror must not hold the passphrase")
            key(d, "Return")
            H.frame(host, 0.9)
            for _, b in ipairs(host.batches) do
                for _, o in ipairs(b) do
                    local v = o.value
                    H.check(type(v) ~= "string" or not v:find("hunter", 1, true), "no write may carry the passphrase")
                end
            end
            for _, r in ipairs(host.reports) do
                H.check(not r:find("hunter", 1, true), "no report may carry the passphrase")
            end
            H.check(host.answers[1].text == "hunter2", "it went to the compositor as the answer")
        end,
    },
    {
        "the dialog's keys arrive through the real event path: a bubbled key event reaches the dialog from its entry",
        function()
            local host = started()
            host.offer({ request = 9, slot = "prompt.secret", message = "Password?" })
            H.frame(host, 0.1)
            H.frame(host, 0.7)
            local d = desk():all(PassphraseDialog)[1]
            local function press(name, text)
                H.frame(host, 0.8, {
                    {
                        target = d.entry.id,
                        type = "key",
                        bubbles = true,
                        args = {},
                        fields = { key = name, text = text, pressed = true },
                    },
                })
            end
            press("o", "o")
            press("k", "k")
            H.check(d.entry.text == "ok", "typed keys reach the entry")
            press("Return", "")
            H.check(host.answers[1] and host.answers[1].text == "ok", "and Return reaches the dialog above it")
        end,
    },
    {
        "a consent key event bubbled to the dialog is stopped, so nothing above it sees it",
        function()
            local host = started()
            local d = consent(host, 7)
            local above = false
            d.parent.key:connect(function()
                above = true
            end)
            H.frame(host, 0.2, {
                {
                    target = d.id,
                    type = "key",
                    bubbles = true,
                    args = {},
                    fields = { key = "Tab", text = "", pressed = true },
                },
            })
            H.check(d.index == 2, "the dialog handled Tab")
            H.check(not above, "a key the dialog handled does not bubble to the scrim")
        end,
    },
    {
        "a press on the scrim is taken: the focus stays on the dialog",
        function()
            local host = started()
            local d = consent(host, 7)
            seat.focus = nil
            d.parent.press:emit({ button = LEFT })
            H.check(seat.focus == d, "a click outside the dialog must not take the keyboard away")
        end,
    },
    {
        "M.release gives both slots back",
        function()
            local host = started()
            require("gnoblin.shell.prompts").release()
            H.check(not next(host.claimed), "nothing is claimed after release")
        end,
    },
}
