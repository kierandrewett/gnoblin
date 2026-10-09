gnoblin.configure {auth = {polkit_agent = true}}

local function log(text)
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/doc-events.log', "sh", text})
end

gnoblin.on("gnoblin.auth.requested", function(event)
    log("requested " .. event.request_id)
    gnoblin.auth.begin {request_id = event.request_id}
end)

gnoblin.on("gnoblin.auth.prompt", function(event)
    log("prompt " .. event.request_id)
    gnoblin.commands.capture {
        argv = {"sh", "-c", "echo gnoblin-test"},
    }:on_complete(function(result, err)
        if err or result.exit_code ~= 0 then
            log("cancelling")
            gnoblin.auth.cancel {request_id = event.request_id}
            return
        end
        local password = result.stdout:gsub("\n$", "")
        gnoblin.auth.respond {request_id = event.request_id, response = password}
    end)
end)

gnoblin.on("gnoblin.auth.finished", function(event)
    log("finished " .. event.request_id .. " " .. event.result)
end)

gnoblin.on("gnoblin.config.reloaded", function(event)
    gnoblin.commands.run({"sh", "-c", "rm -f /tmp/pk3.out; pkexec id -u > /tmp/pk3.out 2>&1; echo rc=$? >> /tmp/pk3.out"})
end)
