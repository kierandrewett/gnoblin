local function log(text)
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/location-events.log', "sh", text})
end

gnoblin.on("gnoblin.location.authorization-requested", function(event)
    log("requested id=" .. event.request_id .. " app=" .. event.app_id .. " accuracy=" .. event.requested_accuracy)
    gnoblin.location.authorize_app {request_id = event.request_id, allow = true, accuracy = event.requested_accuracy}
end)
