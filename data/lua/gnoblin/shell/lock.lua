-- data/lua/gnoblin/shell/lock.lua: the stock lock screen. The host sends `requested` when the session locks; the
-- shell engages the Lock node. `unlock` asks the native core to check the password and waits for its answer.
-- It returns `"ok"` (and disengages) or `""` with the error `"denied"`; a rate-limited attempt returns the
-- wait in milliseconds as the result with the error `"rate-limited"`. Any other outcome keeps the lock engaged.
function Lock:on_requested()
    self.engaged = true
end
function Lock:unlock(password)
    local result, err = await(self:authenticate(password))
    if result == "ok" then
        self.engaged = false
    end
    return result, err
end
