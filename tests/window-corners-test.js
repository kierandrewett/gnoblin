// gjs -m tests/window-corners-test.js
import {defaults, validate, merge, geometry, enabled} from '../src/gnome-shell-overlay/js/ui/components/gnoblinCornerGeometry.js';
import {parseDocument, windowEffects} from '../src/gnome-shell-overlay/js/ui/components/gnoblinConfig.js';
function assert(ok, message) { if (!ok) throw new Error(message); }
for (const value of [{radius:-1}, {radius:Infinity}, {smoothing:1.1}, {mode:'guess'}, {padding:[0,0]}, {'border-color':'red'}, {shadow:{blur:101}}, {shadow:{typo:1}}, {typo:1}, {toString:true}, {'keep-tiled':1}, {shadow:true}, {shadow:{opacity:-1}}]) {
    let rejected=false; try {validate(value);} catch (_) {rejected=true;} assert(rejected, JSON.stringify(value));
}
validate({radius:14,smoothing:.6,padding:[1,2,3,4],'border-width':-2,'border-color':'#12345678',shadow:{blur:24,spread:-2,opacity:.5}});
const config = parseDocument({'window-rules':[
    {match:{type:'window'},corners:{radius:14,smoothing:.6,shadow:{blur:28,opacity:.5}}},
    {match:{focused:true},corners:{shadow:{opacity:.8}}},
    {match:{'app-id':'^skip$'},corners:{radius:0}},
]});
const effects = windowEffects({type:'window',focused:true,'app-id':'test'},config).corners;
assert(effects.radius===14 && effects.smoothing===.6 && effects.shadow.blur===28 && effects.shadow.opacity===.8, 'partial rule cascade');
assert(windowEffects({type:'window',focused:true,'app-id':'skip'},config).corners.radius===0, 'app exclusion wins');
const g=geometry({x:20,y:30,width:200,height:100},{x:0,y:0,width:240,height:160},480,320,{...defaults,radius:14,padding:[1,2,3,4]});
assert(JSON.stringify(g.bounds)==='[48,62,436,254]', 'buffer shadows and scale excluded using frame geometry');
assert(g.radius===28, 'logical radius scales with buffer');
const small=geometry({x:0,y:0,width:40,height:20},{x:0,y:0,width:40,height:20},40,20,{...defaults,radius:200,smoothing:1});
assert(small.radius===10, 'large smoothed corners clamp to both dimensions');
assert(geometry({x:0,y:0,width:10,height:10},{x:0,y:0,width:0,height:0},0,0,defaults)===null,'unmapped buffer bypass');
const settings={...defaults,radius:14};
assert(enabled(settings,{normal:true}), 'ordinary window');
for(const state of [{normal:false},{normal:true,fullscreen:true},{normal:true,maximized:true},{normal:true,tiled:true},{normal:true,adwaita:true}]) assert(!enabled(settings,state),'automatic exclusions');
assert(enabled({...settings,mode:'force'},{normal:true,adwaita:true}), 'explicit override');
assert(enabled({...settings,'keep-fullscreen':true},{normal:true,fullscreen:true}), 'fullscreen preference');
assert(!enabled({...settings,mode:'off'},{normal:true}), 'explicit disable');
print('PASS: corner schema, cascades, state policy, frame offsets and scaling');

const luaConfig = parseDocument({'window-rules': [
    {match: {type: 'window'}, corners: {radius: 14, smoothing: .6, padding: [1, 2, 3, 4], shadow: {blur: 24, opacity: .5}}},
    {match: {focused: true}, corners: {shadow: {opacity: .8}, 'border-color': '#12345678'}},
]});
const parsed = windowEffects({type:'window',focused:true},luaConfig).corners;
assert(parsed.radius === 14 && parsed.shadow.blur === 24 && parsed.shadow.opacity === .8 && parsed['border-color'] === '#12345678', 'Lua nested tables merge');
const inherited = parseDocument({'window-rules': [
    {match: {type: 'window'}, corners: {radius: 48, smoothing: 1, padding: [2, 3, 4, 5]}, borders: {'inner-width': 1}},
]});
const inheritedBorders = windowEffects({type: 'window'}, inherited).borders;
assert(inheritedBorders.radius === 48 && inheritedBorders.smoothing === 1 &&
    JSON.stringify(inheritedBorders.padding) === '[2,3,4,5]', 'borders inherit corner geometry');
const overridden = parseDocument({'window-rules': [
    {match: {type: 'window'}, corners: {radius: 48, smoothing: 1}, borders: {radius: 12, smoothing: 0}},
]});
const overriddenBorders = windowEffects({type: 'window'}, overridden).borders;
assert(overriddenBorders.radius === 12 && overriddenBorders.smoothing === 0, 'explicit border geometry wins');
print('PASS: Lua corner settings');
