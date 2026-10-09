-- Logs every pointer locate request, so a test can check that the event reaches a Lua listener.
gnoblin.on("gnoblin.pointer.locate-requested", function(event)
    local text = string.format("locate x=%s y=%s monitor=%s", tostring(event.x), tostring(event.y), tostring(event.monitor_id))
    gnoblin.commands.run({"sh", "-c", 'echo "$1" >> /tmp/locate-events.log', "sh", text})
end)
