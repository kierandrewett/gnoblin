import {parseDocument, windowEffects} from '../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js';

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

const config = parseDocument({'window-rules': [
    {match: {type: 'layer'}, blur: 24, shader: 'shaders/tint.frag', 'shader-uniforms': {strength: .2}},
    {match: {layer: '^menu$'}, 'shader-uniforms': {strength: .5}},
    {match: {layer: '^capture$'}, shader: '', blur: 0},
]});
const menu = windowEffects({type: 'layer', layer: 'menu'}, config);
assert(menu.blur === 24 && menu.shader === 'shaders/tint.frag' && menu['shader-uniforms'].strength === 0.5,
    'later rules override named effects and retain earlier effects');
assert(windowEffects({type: 'layer', layer: 'capture'}, config).shader === '', 'explicit shader removal');
assert(windowEffects({type: 'window', layer: null}, config).shader === '', 'unmatched windows have no shader');
for (const extra of [
    {shader: 12}, {shader: 'bad\0path'}, {shader: 'x'.repeat(4097)},
    {'shader-uniforms': []}, {'shader-uniforms': {strength: '0.5'}},
    {'shader-uniforms': {strength: Infinity}}, {'shader-uniforms': {strength: 3.5e38}},
    {'shader-uniforms': {'bad-name': 1}}, {'shader-uniforms': {gnoblin_width: 1}},
    {'shader-uniforms': Object.fromEntries(Array.from({length: 65}, (_, i) => [`u${i}`, 1]))},
]) {
    let rejected = false;
    try { parseDocument({'window-rules': [{match: {type: 'layer'}, ...extra}]}); } catch { rejected = true; }
    assert(rejected, `invalid shader config accepted: ${JSON.stringify(extra)}`);
}
print('PASS: shader config, matching, precedence, removal and invalid values');

const shadows = parseDocument({'window-rules': [
    {match: {type: 'layer'}, 'blur-ignore-shadows': true},
    {match: {layer: '^black-glass$'}, 'blur-ignore-shadows': false},
]});
assert(windowEffects({type: 'layer', layer: 'panel'}, shadows)['blur-ignore-shadows'], 'layer shadow exclusion');
assert(!windowEffects({type: 'layer', layer: 'black-glass'}, shadows)['blur-ignore-shadows'], 'black glass can opt out');
assert(!windowEffects({type: 'window'}, shadows)['blur-ignore-shadows'], 'shadow exclusion defaults off');
for (const value of [0, 1, 'true', null, []]) {
    let rejected = false;
    try { parseDocument({'window-rules': [{match: {type: 'layer'}, 'blur-ignore-shadows': value}]}); } catch (_) { rejected = true; }
    assert(rejected, 'shadow exclusion requires a boolean');
}
print('PASS: shadow exclusion validation, opt-in and rule override');
