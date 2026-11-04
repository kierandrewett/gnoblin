/* Describes src/config/lua/gnoblin-lua-subscriptions.c and gnoblin-lua-dispatch.c. */

/*** lua
---Event payload passed to a subscription callback. Fields beyond `name` depend on the event.
---@class Event
---@field name string Name of the event that was dispatched.

---Handle returned by an event subscription.
---@class EventSubscription
---@field unsubscribe fun(self: EventSubscription) Stop receiving this event.

---Register a callback for an event. Event names are UTF-8 strings of 1 to 128 bytes.
---@param name string Event name, or `*` to receive every dispatched event.
---@param callback fun(event: Event) Called once for each matching event.
---@return EventSubscription
function gnoblin.on(name, callback) end

---Register a callback that is removed after its first event.
---@param name string Event name, or `*` to receive the next dispatched event.
---@param callback fun(event: Event) Called once for the first matching event.
---@return EventSubscription
function gnoblin.once(name, callback) end
*/
