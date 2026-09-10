import assert from 'node:assert/strict';
import {validate, evaluate, replacePolicy, serialise} from '../src/gnome-shell-overlay/js/ui/components/gnoblinPermissions.js';
const rule = {name: 'rustdesk', match: '^host-exe:/usr/bin/rustdesk$',
    capabilities: ['screen-cast', 'remote-desktop'], level: 'allow', monitors: ['primary'], devices: ['keyboard', 'pointer']};
const identity = 'host-exe:/usr/bin/rustdesk';
let policy = validate({default: 'ask', rules: [rule]});
assert.equal(evaluate(policy, 'screen-cast', identity).level, 'allow');
assert.equal(evaluate(policy, 'remote-desktop', identity).devices, 3);
assert.equal(evaluate(policy, 'remote-desktop', identity).clipboard, false);
for (const other of ['', 'rustdesk', 'host-exe:/tmp/rustdesk', 'app-id:rustdesk', `${identity}-fake`])
    assert.equal(evaluate(policy, 'screen-cast', other).level, 'ask');
assert.equal(evaluate(policy, 'screenshot', identity).level, 'ask');
const deny = {...rule, name: 'block', level: 'deny'};
for (const rules of [[rule, deny], [deny, rule]])
    assert.equal(evaluate(validate({rules}), 'screen-cast', identity).level, 'deny');
policy = validate({rules: [rule, {...rule, name: 'prompt', level: 'ask'}]});
assert.equal(evaluate(policy, 'screen-cast', identity).rule, 'prompt');
assert.equal(evaluate(validate({default: 'deny'}), 'screen-cast', '').level, 'deny');
for (const invalid of [null, [], {default: 'allow'}, {default: 'oops'}, {unknown: 1},
    {rules: [rule, rule]}, {rules: [{...rule, match: '['}]}, {rules: [{...rule, match: ''}]},
    {rules: [{...rule, capabilities: ['camera']}]}, {rules: [{...rule, devices: ['mouse']}]},
    {rules: [{...rule, clipboard: 'true'}]}, {rules: [{...rule, monitors: []}]},
    {rules: [{...rule, capabilities: ['access']}]}, {rules: Array(257).fill(rule)}])
    assert.throws(() => validate(invalid));
assert.throws(() => evaluate(policy, 'unknown', identity));
const text = '# keep\n[shell]\nosd = true\n[permissions]\ndefault = "ask"\n[[permissions.rules]]\nname = "old"\n[protocols]\nx = true\n';
const updated = replacePolicy(text, {rules: [rule]});
assert(updated.startsWith('# keep\n[shell]\nosd = true\n[protocols]\nx = true'));
assert(!updated.includes('"old"'));
assert(updated.includes('[[permissions.rules]]'));
assert(serialise({rules: [rule]}).includes('match = "^host-exe:/usr/bin/rustdesk$"'));
console.log('permission policy: precedence, identity, scope, validation and editing passed');
