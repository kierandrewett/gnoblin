-- Print the readiness of the Gnoblin session for the graphical VM test (gnoblinctl lua ready.lua), one key=value a
-- line. The session is ready when the Lua runtime and the session lifecycle both report "running".
local runtime = gnoblin.runtime.status()
local session = gnoblin.session.status()
print("runtime=" .. tostring(runtime.state))
print("session=" .. tostring(session.session_state or session.state))
