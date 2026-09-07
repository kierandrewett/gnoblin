import {parse, parseDocument, windowEffects} from '../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js';

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

const config = parse(`[[window-rules]]
match.type = "layer"
blur = 24
shader = "shaders/tint.frag"
shader-uniforms.strength = 0.2
[[window-rules]]
match.layer = "^menu$"
shader-uniforms.strength = 0.5
[[window-rules]]
match.layer = "^capture$"
shader = ""
blur = 0
`);
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
