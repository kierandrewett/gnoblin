// Permission decisions contain no saved grants or portal implementation state.
export const CAPABILITIES = Object.freeze([
    'screen-cast', 'remote-desktop', 'input-capture', 'screenshot', 'access',
]);
export const LEVELS = Object.freeze(['default', 'ask', 'allow', 'deny']);
export const DEFAULT_POLICY = Object.freeze({default: 'default', rules: []});

function table(value) {
    return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function validate(policy = DEFAULT_POLICY) {
    if (!table(policy) || Object.keys(policy).some(key => !['default', 'rules'].includes(key)) ||
        !LEVELS.includes(policy.default ?? 'default') || !Array.isArray(policy.rules ?? []))
        throw new Error('permissions requires a default decision and an array of rules');
    // Global allow would authorise callers for which identity verification failed.
    if (policy.default === 'allow')
        throw new Error('permissions.default cannot allow; use an explicit app rule');
    const names = new Set();
    const rules = policy.rules ?? [];
    if (rules.length > 256) throw new Error('permissions supports at most 256 rules');
    for (const rule of rules) {
        if (!table(rule) || Object.keys(rule).some(key =>
            !['name', 'match', 'capabilities', 'level', 'monitors', 'devices', 'clipboard'].includes(key)) ||
            typeof rule.name !== 'string' || !/^[a-zA-Z0-9_.-]{1,80}$/.test(rule.name) || names.has(rule.name) ||
            !LEVELS.includes(rule.level) || !Array.isArray(rule.capabilities) || !rule.capabilities.length ||
            !rule.capabilities.every(cap => CAPABILITIES.includes(cap)) ||
            typeof rule.match !== 'string' || !rule.match.length || rule.match.length > 512)
            throw new Error('invalid permission rule: expected unique name, identity regex, capabilities and level');
        new RegExp(rule.match);
        names.add(rule.name);
        if (rule.monitors !== undefined && (!Array.isArray(rule.monitors) || !rule.monitors.length ||
            !rule.monitors.every(name => typeof name === 'string' && /^[A-Za-z0-9_.:-]{1,80}$/.test(name))))
            throw new Error('permission monitors must be primary or exact connector names');
        if (rule.devices !== undefined && (!Array.isArray(rule.devices) ||
            !rule.devices.every(name => ['keyboard', 'pointer', 'touchscreen'].includes(name))))
            throw new Error('permission devices must be keyboard, pointer or touchscreen');
        if (rule.clipboard !== undefined && typeof rule.clipboard !== 'boolean')
            throw new Error('permission clipboard must be boolean');
        if (rule.monitors !== undefined && !rule.capabilities.some(cap => ['screen-cast', 'remote-desktop'].includes(cap)) ||
            (rule.devices !== undefined || rule.clipboard !== undefined) && !rule.capabilities.includes('remote-desktop'))
            throw new Error('permission selection does not apply to these capabilities');
    }
    return {default: policy.default ?? 'default', rules: rules.map(rule => ({...rule}))};
}

export function evaluate(policy, capability, identity) {
    if (!CAPABILITIES.includes(capability)) throw new Error(`unsupported permission: ${capability}`);
    const result = {level: policy.default, rule: '', monitors: [], devices: 0, clipboard: false};
    // Namespaces come from portal credentials, never a window title or Wayland app_id.
    if (!/^(app-id:|host-exe:\/).+/.test(identity))
        return {...result, level: result.level === 'deny' ? 'deny' : 'ask', rule: 'unverified-identity'};
    for (const rule of policy.rules) {
        if (!rule.capabilities.includes(capability) || !new RegExp(rule.match).test(identity)) continue;
        Object.assign(result, {level: rule.level, rule: rule.name,
            monitors: rule.monitors ?? [],
            devices: (rule.devices ?? []).reduce((mask, device) => mask | {keyboard: 1, pointer: 2, touchscreen: 4}[device], 0),
            clipboard: rule.clipboard ?? false});
        // A blocklist cannot be overridden by a later whitelist.
        if (rule.level === 'deny') break;
    }
    return result;
}
