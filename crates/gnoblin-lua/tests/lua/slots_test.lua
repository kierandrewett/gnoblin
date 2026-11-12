-- tests/lua/slots_test.lua: `slots.claim` and the offers a claimed slot receives (node.lua), over the fake core. The fake
-- stands in for the compositor's owner connection: `host.claimed`, `host.answers`, `host.declines`, `host.offer`,
-- `host.finish` and `host.lose` are its side of the exchange.
local H = require("harness")

local function fresh()
    local _, host = H.fresh()
    return host
end

-- An offer of request `id` to `slot`, delivered by a frame at `now`.
local function offer(host, id, slot, now)
    host.offer({ request = id, slot = slot or "portal.ui" })
    H.frame(host, now or 0.1)
end

return {
    {
        "claiming a slot claims it at the compositor, and a second claim by the same shell is an error",
        function()
            local host = fresh()
            slots.claim("portal.ui", function() end)
            H.check(host.claimed["portal.ui"], "the compositor must hold the claim")
            H.check(slots.claimed("portal.ui"), "the shell must know it claims the slot")
            local ok, err = pcall(slots.claim, "portal.ui", function() end)
            H.check(
                not ok and tostring(err):find("already claims", 1, true),
                "a second claim must raise: " .. tostring(err)
            )
        end,
    },
    {
        "a claim the compositor refuses raises its code and claims nothing",
        function()
            local host = fresh()
            host.refuse = "denied: portal.ui is held by another shell"
            local ok, err = pcall(slots.claim, "portal.ui", function() end)
            H.check(
                not ok and tostring(err):find("denied", 1, true),
                "the refusal's code must reach the script: " .. tostring(err)
            )
            H.check(not slots.claimed("portal.ui"), "a refused claim must not count as held")
        end,
    },
    {
        "a claim needs a slot name and a handler function",
        function()
            fresh()
            H.check(not pcall(slots.claim, "portal.ui"), "no handler must raise")
            H.check(not pcall(slots.claim, nil, function() end), "no name must raise")
        end,
    },
    {
        "an offer is claimed and reaches the handler with what the compositor sent",
        function()
            local host = fresh()
            local got
            slots.claim("portal.ui", function(o)
                got = o
            end)
            host.offer({
                request = 5,
                title = "ScreenCast",
                message = "Share a window?",
                subject = "org.a.B",
                deadline = 99,
            })
            H.frame(host, 0.1)
            H.check(got, "the handler must run")
            H.check(got.request == 5 and got.slot == "portal.ui", "the offer must name its request and slot")
            H.check(got.title == "ScreenCast" and got.message == "Share a window?", "title and message")
            H.check(got.subject == "org.a.B" and got.deadline == 99, "subject and deadline")
            H.check(host.request_claims[1] == 5, "the shell must claim the request before it draws")
            H.check(not got.finished, "an open offer is not finished")
        end,
    },
    {
        "answering sends the text once and finishes the offer",
        function()
            local host = fresh()
            local held
            slots.claim("portal.ui", function(o)
                held = o
                o:answer("deny")
            end)
            offer(host, 6)
            H.check(
                #host.answers == 1 and host.answers[1].request == 6 and host.answers[1].text == "deny",
                "the answer must reach the compositor"
            )
            H.check(held.finished and held.completion == "answered", "the offer is settled")
            local ok = pcall(held.answer, held, "allow")
            H.check(not ok, "a second answer must raise")
            H.check(#host.answers == 1, "and send nothing")
        end,
    },
    {
        "a handler that raises is reported and declines its offer",
        function()
            local host = fresh()
            slots.claim("portal.ui", function()
                error("boom")
            end)
            offer(host, 7)
            H.check(host.declines[1] == 7, "the compositor must be told this shell will not answer")
            local reported = false
            for _, r in ipairs(host.reports) do
                reported = reported or r:find("boom", 1, true) ~= nil
            end
            H.check(reported, "the failure must be reported")
            H.check(#host.answers == 0, "nothing may be answered on a failed handler")
        end,
    },
    {
        "an offer for a slot this shell has no handler for is declined, not left to time out",
        function()
            local host = fresh()
            slots.claim("portal.ui", function() end)
            offer(host, 8, "prompt.secret")
            H.check(host.declines[1] == 8, "an offer nobody here serves must be declined")
        end,
    },
    {
        "an offer another connection already took or that already ended never reaches the handler",
        function()
            local host = fresh()
            local ran = false
            slots.claim("portal.ui", function()
                ran = true
            end)
            host.finish(9)
            host.offer({ request = 9 })
            H.frame(host, 0.1)
            H.check(not ran, "a request the compositor refuses the claim of is not ours to draw")
            H.check(#host.declines == 0 and #host.answers == 0, "and nothing is sent for it")
        end,
    },
    {
        "a request the compositor ends finishes the offer and runs its on_finished callbacks once",
        function()
            local host = fresh()
            local held
            local calls = {}
            slots.claim("portal.ui", function(o)
                held = o
                o:on_finished(function(_, completion)
                    calls[#calls + 1] = completion
                end)
            end)
            offer(host, 10)
            H.check(not held.finished, "still open")
            host.finish(10, "timed-out")
            H.frame(host, 0.2)
            H.check(
                held.finished and held.completion == "timed-out",
                "the offer must finish with the compositor's reason"
            )
            H.check(#calls == 1 and calls[1] == "timed-out", "on_finished must run once with the reason")
            host.finish(10, "ended")
            H.frame(host, 0.3)
            H.check(#calls == 1, "a second end must not run it again")
            H.check(not pcall(held.answer, held, "allow"), "an answer to a finished offer must raise")
        end,
    },
    {
        "on_finished on an offer that is already over runs at once",
        function()
            local host = fresh()
            local held
            slots.claim("portal.ui", function(o)
                held = o
                o:decline()
            end)
            offer(host, 11)
            local seen
            held:on_finished(function(_, completion)
                seen = completion
            end)
            H.check(seen == "declined", "a late subscriber must still hear how it ended")
        end,
    },
    {
        "declining twice, or after answering, sends nothing more",
        function()
            local host = fresh()
            local held
            slots.claim("portal.ui", function(o)
                held = o
            end)
            offer(host, 12)
            held:decline()
            held:decline()
            H.check(#host.declines == 1, "one decline reaches the compositor")
        end,
    },
    {
        "a decline carries its reason to the compositor, and a plain one carries none",
        function()
            local host = fresh()
            local held = {}
            slots.claim("portal.ui", function(o)
                held[o.request] = o
            end)
            offer(host, 14)
            offer(host, 15)
            held[14]:decline("locked")
            held[15]:decline()
            H.check(host.decline_reasons[14] == "locked", "the named reason reaches the link")
            H.check(host.decline_reasons[15] == false, "no reason is none")
            offer(host, 16)
            H.check(not pcall(held[16].decline, held[16], "polite"), "an unknown reason is refused, not sent as none")
            H.check(not held[16].finished and #host.declines == 2, "and the offer stays open")
            held[16]:decline()
            H.check(host.decline_reasons[16] == false and #host.declines == 3, "so a plain decline still goes out")
        end,
    },
    {
        "a handler may await, and answers later",
        function()
            local host = fresh()
            slots.claim("portal.ui", function(o)
                await(0.5)
                o:answer("deny")
            end)
            offer(host, 13, "portal.ui", 0.1)
            H.frame(host, 0.3)
            H.check(#host.answers == 0, "not yet")
            H.frame(host, 0.7)
            H.check(#host.answers == 1 and host.answers[1].text == "deny", "after the wait the answer must be sent")
        end,
    },
    {
        "a lost connection finishes every open offer and forgets the claims",
        function()
            local host = fresh()
            local held
            slots.claim("portal.ui", function(o)
                held = o
            end)
            offer(host, 14)
            host.lose()
            H.frame(host, 0.2)
            H.check(held.finished and held.completion == "lost", "the open offer must end as lost")
            H.check(not slots.claimed("portal.ui"), "the compositor released the claim with the connection")
        end,
    },
    {
        "releasing a slot gives it back and ends the offers still open on it",
        function()
            local host = fresh()
            local held
            slots.claim("portal.ui", function(o)
                held = o
            end)
            offer(host, 15)
            slots.release("portal.ui")
            H.check(not host.claimed["portal.ui"], "the compositor must see the release")
            H.check(held.finished, "an open offer must end with the slot")
            slots.release("portal.ui") -- releasing what is not held is not an error
        end,
    },
    {
        "an answer the compositor refuses raises and finishes the offer",
        function()
            local host = fresh()
            local held
            slots.claim("portal.ui", function(o)
                held = o
            end)
            offer(host, 16)
            host.finish(16, "ended") -- ended at the compositor, not yet delivered to the shell
            local ok, err = pcall(held.answer, held, "allow")
            H.check(
                not ok and tostring(err):find("request-finished", 1, true),
                "the compositor's code must reach the script: " .. tostring(err)
            )
            H.check(held.finished, "a refused answer ends the offer")
        end,
    },
}
