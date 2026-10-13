/* Annotations for the Lua API implemented by src/config/lua/gnoblin-lua-state-queries.c,
 * gnoblin-lua-operations.c, and gnoblin-lua-variant.c, with permission and portal records built by
 * src/portal/backend/gnoblin-portal-policy.c and src/portal/grants/gnoblin-control-grants.c. */

/*** lua
---@alias PermissionCapability "screen-cast"|"remote-desktop"|"input-capture"|"screenshot"|"access"
---@alias PermissionLevel "inherit"|"default"|"ask"|"allow"|"deny"
---@alias PermissionReportedLevel "inherit"|"ask"|"allow"|"deny"
---@alias PermissionDecisionLevel "inherit"|"default"|"ask"|"allow"|"deny"
---@alias PortalGrantKind "screen-cast"|"remote-desktop"
---@alias RemoteDesktopDevice "keyboard"|"pointer"|"touchscreen"

---@class PermissionRule
---@field name string Unique rule name of at most 80 ASCII letters, digits, underscore, dot, or
hyphen.
---@field match string Nonempty valid Lua pattern of at most 512 UTF-8 bytes, matched against the
permission identity.
---@field capabilities PermissionCapability[] Nonempty capabilities selected by this rule.
---@field level PermissionLevel Rule decision level.
---@field monitors? string[] Nonempty monitor names; only for screen-cast or remote-desktop
capabilities.
---@field devices? RemoteDesktopDevice[] Nonempty devices; only for remote-desktop capability.
---@field clipboard? boolean Clipboard access; only for remote-desktop capability.

---@class PermissionPolicy
---@field default PermissionReportedLevel Default decision. The deprecated "default" spelling is
reported as "inherit".
---@field rules PermissionRule[] Permission rules in configured order.
---@field revision integer Policy revision.

---@class PermissionListSnapshot
---@field policy {default: PermissionReportedLevel, rules: PermissionRule[]} Committed policy
values.
---@field capabilities PermissionCapability[] Capabilities accepted by the policy.
---@field levels PermissionReportedLevel[] Levels accepted for the default policy.
---@field path string Config path, or an empty string when not supplied.

---@class PermissionDecision
---@field level PermissionDecisionLevel Effective decision level.
---@field rule string Matching rule name, or an empty string when no rule matched.
---@field monitors string[] Monitor restrictions selected by the effective rule.
---@field devices RemoteDesktopDevice[] Input device restrictions selected by the effective rule.
---@field clipboard boolean Whether clipboard access is allowed by the effective rule.
---@field revision integer Policy revision used to evaluate the decision.

---@class Capability
---@field id string Stable capability identifier.
---@field description string Human-readable capability description.
---@field available boolean Whether the capability is available in this session.
---@field reason? string Present when unavailable; a machine-readable reason.
---@field revision integer Capability snapshot revision.

---@class Grant
---@field id string Opaque portal grant identifier.
---@field kind PortalGrantKind Grant type.
---@field requester string Requester identity reported by the portal.
---@field devices integer Bit mask: keyboard is 1, pointer is 2, and touchscreen is 4.
---@field clipboard boolean Whether clipboard access was granted.
---@field screenStreams boolean Whether the grant has a saved screen stream selection.

---@class PortalGrant
---@field id string Opaque portal grant identifier.
---@field kind PortalGrantKind Grant type.
---@field requester string Requester identity reported by the portal.
---@field devices RemoteDesktopDevice[] Granted input devices.
---@field clipboard boolean Whether clipboard access was granted.
---@field has_screen_streams boolean Whether the grant has active screen streams.
---@field created_at integer Grant creation time in Unix milliseconds.
---@field revision integer Grant snapshot revision.

---@class PermissionCheckArgs
---@field capability PermissionCapability Capability to evaluate.
---@field identity string Permission identity, normally prefixed by "app-id:" or "host-exe:/".

---@class PortalGrantFilter
---@field kind? PortalGrantKind Return only grants of this kind.

---@class GrantRevokeArgs
---@field kind PortalGrantKind Type of the grant to revoke.
---@field id string Nonempty opaque grant identifier.
---@field created_at? integer Nonnegative creation time in Unix milliseconds, used to guard against
revoking a replaced grant.

---@class GrantRevokeResult
---@field ok boolean Whether the grant was revoked.
---@field id string Identifier of the revoked grant.

---List capabilities available in the current session.
---@return Capability[]
function gnoblin.capabilities.list() end

---Read the committed permission policy and its supported values.
---@return PermissionListSnapshot
function gnoblin.permissions.list() end

---Read the effective committed permission policy.
---@return PermissionPolicy
function gnoblin.permissions.policy() end

---Evaluate a permission for a capability and application identity. Pass a PermissionCheckArgs table
or a capability and identity string.
---@param request PermissionCheckArgs|PermissionCapability
---@param identity? string Required when request is a capability string.
---@return PermissionDecision
function gnoblin.permissions.check(request, identity) end

---List portal grants, optionally filtered by grant kind.
---@param filter? PortalGrantFilter
---@return PortalGrant[]
function gnoblin.portals.grants(filter) end

---@class GrantListResult
---@field grants Grant[] Portal grants returned by the backend.

---List portal grants through an asynchronous operation.
---@return Operation<GrantListResult>
function gnoblin.grant.list() end

---Revoke a portal grant by its kind and opaque identifier.
---@param grant GrantRevokeArgs
---@return Operation<GrantRevokeResult>
function gnoblin.grant.revoke(grant) end

---Revoke this portal grant.
---@return Operation<GrantRevokeResult>
function PortalGrant:revoke() end
*/
