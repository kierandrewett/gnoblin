gnoblin.configure({ auth = { polkit_agent = true } })

local function log(text)
    gnoblin.process.spawn({ "sh", "-c", "echo \"$1\" >> /tmp/doc-events.log", "sh", text })
end

gnoblin.on("gnoblin.auth.requested", function(event)
    log("requested " .. event.request.id)
    event.request:claim()
end)

gnoblin.on("gnoblin.auth.prompt", function(event)
    local request = event.request
    log("prompt " .. request.id)
    gnoblin.process
        .spawn({ "sh", "-c", "echo gnoblin-test" }, { stdout = "capture" })
        :wait()
        :on_complete(function(result, err)
            if err or result.exit_code ~= 0 then
                log("cancelling")
                request:cancel()
                return
            end
            local password = result.stdout:gsub("\n$", "")
            request:answer(password)
        end)
end)

gnoblin.on("gnoblin.auth.finished", function(event)
    log("finished " .. event.request_id .. " " .. event.result)
end)

gnoblin.on("gnoblin.config.reloaded", function(event)
    gnoblin.process.spawn({
        "sh",
        "-c",
        "rm -f /tmp/pk3.out; pkexec id -u > /tmp/pk3.out 2>&1; echo rc=$? >> /tmp/pk3.out",
    })
end)
