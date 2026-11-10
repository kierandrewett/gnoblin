-- data/lua/gnoblin/shell/lock.lua: the stock lock screen. The host sends `requested` when the session locks; the
-- shell engages the Lock node. `unlock` asks the native core to check the password and waits for its answer.
function Lock:on_requested()
    self.engaged = true
end
function Lock:unlock(password)
    local ok = await(self:authenticate(password))
    if ok == "ok" then
        self.engaged = false
    end
    return ok
end
