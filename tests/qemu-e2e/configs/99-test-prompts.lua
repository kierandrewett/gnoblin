gnoblin.configure {prompts = {enabled = true}}

local function log(text)
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/prompt-events.log', "sh", text})
end

gnoblin.on("gnoblin.prompt.requested", function(event)
    log("requested id=" .. event.request_id .. " kind=" .. event.kind .. " title=[" .. tostring(event.title) .. "] message=[" .. tostring(event.message) .. "] new=" .. tostring(event.password_new))
    if event.kind == "password" then
        gnoblin.prompt.respond {request_id = event.request_id, password = "gnoblin-keyring-test"}
    else
        gnoblin.prompt.respond {request_id = event.request_id, confirmed = true}
    end
end)

gnoblin.on("gnoblin.prompt.finished", function(event)
    log("finished id=" .. event.request_id .. " result=" .. event.result)
end)
