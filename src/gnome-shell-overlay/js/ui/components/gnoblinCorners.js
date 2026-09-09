// Built-in window-rule effects. No extension hooks or polling.
// Superellipse parameterisation follows Rounded Window Corners Reborn:
// https://github.com/flexagoon/rounded-window-corners (GPL-3.0-or-later).
import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import Gdk from 'gi://Gdk?version=4.0';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import * as Geometry from './gnoblinCornerGeometry.js';

// Keep one result for both rings and the clip. Position/focus changes reuse it.
const visibleFrames = new WeakMap();
function windowGeometry(actor, surface, config) {
    const win = actor.meta_window;
    const frame = win.get_frame_rect(), buffer = win.get_buffer_rect();
    const width = surface.width || actor.width;
    const height = surface.height || actor.height;
    let visible = frame;
    const key = [frame.x-buffer.x, frame.y-buffer.y, frame.width-buffer.width,
        frame.height-buffer.height, actor.get_resource_scale(), win.fullscreen,
        win.maximized_horizontally, win.maximized_vertically].join(',');
    let cached = visibleFrames.get(actor);
    // Mapping precedes the first buffer for some clients (for example Spotify).
    // get_image() crashes in Mutter when the shaped texture has no buffer.
    const hasBuffer = Boolean(actor.get_texture()?.get_texture());
    // Explicit client geometry is authoritative. Only inspect clients which
    // include the entire buffer (including their shadow) in the frame.
    if (frame.width === buffer.width && frame.height === buffer.height &&
        !config.padding.some(Boolean) && actor.mapped && actor.opacity === 255 && hasBuffer && !win.minimized &&
        width > 0 && height > 0 && width * height * actor.get_resource_scale() ** 2 <= 16000000) {
        if (cached?.key !== key) {
            const effects = [...actor.get_effects(), ...surface.get_effects()];
            const enabled = effects.map(effect => effect.enabled);
            const decorations = actor.get_children().filter(child => child._gnoblinDecoration && child.visible);
            try {
                effects.forEach(effect => { effect.enabled = false; });
                decorations.forEach(child => child.hide());
                const image = actor.get_image(null);
                if (image) {
                    const iw = image.getWidth(), ih = image.getHeight();
                    const pixbuf = Gdk.pixbuf_get_from_surface(image, 0, 0, iw, ih);
                    const pixels = pixbuf.get_pixels(), stride = pixbuf.rowstride, channels = pixbuf.n_channels;
                    const alpha = (x,y) => channels === 4 ? pixels[y*stride+x*channels+3] : 255;
                    const edges = [[],[],[],[]];
                    for (const fraction of [.25,.5,.75]) {
                        const row = Array.from({length:iw}, (_,x) => alpha(x,Math.floor(ih*fraction)));
                        const column = Array.from({length:ih}, (_,y) => alpha(Math.floor(iw*fraction),y));
                        [column,row.slice().reverse(),column.slice().reverse(),row].forEach((line,i) => edges[i].push(Geometry.detectEdge(line)));
                    }
                    const insets = edges.map(samples => samples.every(n => n !== null && Math.abs(n-samples[0]) <= 1) ? Math.min(...samples) : null);
                    if (insets.every(n => n !== null)) {
                        cached = {key, insets: [insets[0]*buffer.height/ih,insets[1]*buffer.width/iw,
                            insets[2]*buffer.height/ih,insets[3]*buffer.width/iw]};
                        visibleFrames.set(actor,cached);
                    } else {
                        // Cache uncertain frames too: never scan on every focus
                        // change. A window state or scale change causes a fresh measurement.
                        cached = {key,insets:[0,0,0,0]};
                        visibleFrames.set(actor,cached);
                    }
                }
            } catch (error) {
                console.warn(`gnoblin-corners: frame detection failed: ${error.message}`);
                visibleFrames.set(actor,{key,insets:[0,0,0,0]});
            } finally {
                effects.forEach((effect,i) => { effect.enabled = enabled[i]; });
                decorations.forEach(child => child.show());
            }
        }
        if (cached?.key === key) {
            const [top,right,bottom,left] = cached.insets;
            visible = {x:frame.x+left,y:frame.y+top,width:frame.width-left-right,height:frame.height-top-bottom};
        }
    }
    return Geometry.geometry(visible, buffer, width, height, config);
}

const declarations = `
uniform vec4 bounds;
uniform vec2 dimensions;
uniform vec2 textureDimensions;
uniform vec2 textureOrigin;
uniform float pixelWidth;
uniform float radius;
uniform float exponent;
uniform float automatic;
uniform float borderWidth;
uniform vec4 borderColor;
uniform float replaceShadow;
float coverage(vec2 p, vec4 box, float r) {
    if (p.x < box.x || p.y < box.y || p.x > box.z || p.y > box.w) return 0.0;
    r = min(r, min(box.z-box.x, box.w-box.y)*0.5);
    if (r < 0.5) return 1.0;
    vec2 center = clamp(p, box.xy + vec2(r), box.zw - vec2(r));
    vec2 delta = abs(p-center);
    if (delta.x == 0.0 || delta.y == 0.0) return 1.0;
    vec2 unit = delta/r;
    float distance = exponent <= 2.0 ? length(delta) : r*pow(pow(unit.x, exponent)+pow(unit.y, exponent), 1.0/exponent);
    return clamp((r-distance)/pixelWidth+0.5, 0.0, 1.0);
}
float sourceAlpha(vec2 p) { return texture2D(cogl_sampler0, clamp((p-textureOrigin) / textureDimensions, vec2(0.0), vec2(1.0))).a; }
// Four samples only in the small corner regions. Compare with local body
// alpha so translucent rectangles work without mistaking shadows for edges.
bool squareCorner(vec2 p) {
    vec2 middle = (bounds.xy+bounds.zw)*0.5;
    vec2 direction = vec2(p.x < middle.x ? 1.0 : -1.0, p.y < middle.y ? 1.0 : -1.0);
    vec2 origin = vec2(p.x < middle.x ? bounds.x : bounds.z, p.y < middle.y ? bounds.y : bounds.w);
    float inset = min(radius, 8.0);
    float reference = sourceAlpha(origin + direction*vec2(inset));
    float diagonal = sourceAlpha(origin + direction*vec2(0.75));
    float horizontal = sourceAlpha(origin + direction*vec2(inset, 0.75));
    float vertical = sourceAlpha(origin + direction*vec2(0.75, inset));
    return reference > 0.02 && min(diagonal, min(horizontal, vertical)) >= reference*0.85;
}`;

const code = `
vec2 point = cogl_tex_coord0_in.xy * textureDimensions + textureOrigin;
bool outside = point.x < bounds.x || point.y < bounds.y || point.x > bounds.z || point.y > bounds.w;
vec2 cornerDistance = min(point-bounds.xy, bounds.zw-point);
bool corner = cornerDistance.x < radius && cornerDistance.y < radius;
bool apply = automatic < 0.5 || !corner || squareCorner(point);
if (outside && replaceShadow < 0.5 && borderWidth >= 0.0) apply = false;
if (outside && replaceShadow > 0.5) apply = true;
if (apply) {
    float outer = coverage(point, bounds, radius);
    if (abs(borderWidth) > 0.01) {
        vec4 inset = bounds + vec4(borderWidth, borderWidth, -borderWidth, -borderWidth);
        float inner = coverage(point, inset, max(0.0, radius-borderWidth));
        float stroke = abs(outer-inner)*borderColor.a;
        if (automatic > 0.5) stroke *= step(0.02, outside && borderWidth < 0.0 ? sourceAlpha(clamp(point, bounds.xy+vec2(1.0), bounds.zw-vec2(1.0))) : cogl_color_out.a);
        vec4 content = cogl_color_out * outer;
        cogl_color_out = content*(1.0-stroke) + vec4(borderColor.rgb*stroke, stroke);
    } else cogl_color_out *= outer;
}`;

const GeometryEffect = GObject.registerClass(class GnoblinCornerGeometryEffect extends Shell.GLSLEffect {
    _init() { super._init(); this.locations = new Map(); this.values = new Map(); }
    vfunc_paint_target(node, context) {
        const texture = this.get_texture();
        const pipeline = this.get_pipeline();
        if (this.blendPipeline !== pipeline) {
            pipeline.set_blend('RGBA = ADD (SRC_COLOR, DST_COLOR * (1-SRC_COLOR[A]))');
            this.blendPipeline = pipeline;
        }
        const actor = this.get_actor();
        const scale = Math.max(1, Math.ceil(actor.get_resource_scale()));
        this.uniform('pixelWidth', [1/scale]);
        // Clutter pads offscreen paint volumes for filtering. Texture UVs are
        // not window UVs: account for the quantised paint box, including HiDPI.
        // This matches _clutter_actor_box_enlarge_for_effects for a surface.
        this.uniform('textureDimensions', [texture.get_width()/scale, texture.get_height()/scale]);
        const [width,height] = this.logicalDimensions || [actor.width,actor.height];
        this.uniform('textureOrigin', [Math.ceil(width+.75)-Math.round(width)-3,
            Math.ceil(height+.75)-Math.round(height)-3]);
        super.vfunc_paint_target(node, context);
    }
    uniform(name, values) {
        if (name === 'dimensions') this.logicalDimensions = values;
        const key = values.join(',');
        if (this.values.get(name) === key) return;
        if (!this.locations.has(name)) this.locations.set(name, this.get_uniform_location(name));
        this.set_uniform_float(this.locations.get(name), values.length, values);
        this.values.set(name, key);
        this.queue_repaint();
    }
});

const CornersEffect = GObject.registerClass(class GnoblinCornersEffect extends GeometryEffect {
    vfunc_build_pipeline() { this.add_glsl_snippet(Cogl.SnippetHook.FRAGMENT, declarations, code, false); }
    update(g, config) {
        this.uniform('bounds', g.bounds);
        this.uniform('dimensions', [g.width, g.height]);
        this.uniform('radius', [g.radius]); this.uniform('exponent', [g.exponent]);
        this.uniform('automatic', [config.mode === 'auto' ? 1 : 0]);
        this.uniform('borderWidth', [config['border-width']*g.scale]);
        this.uniform('borderColor', rgba(config['border-color']));
        this.uniform('replaceShadow', [config.shadow ? 1 : 0]);
    }
});

function rgba(hex) {
    const digits = hex.slice(1).padEnd(8, 'f');
    return [0, 2, 4, 6].map(offset => parseInt(digits.slice(offset, offset+2), 16)/255);
}

// Analytic shadow: one shader pass, no CSS blur render targets or white
// silhouette to subtract. The window-shaped cutout preserves translucent bodies.
const ShadowEffect = GObject.registerClass(class GnoblinCornerShadow extends GeometryEffect {
    vfunc_build_pipeline() {
        this.add_glsl_snippet(Cogl.SnippetHook.FRAGMENT, declarations + `
uniform float fadeProgress;
${['A','B'].map(state => Array.from({length: 4}, (_, i) => `uniform vec4 shadowGeometry${state}${i};\nuniform vec4 shadowColor${state}${i};`).join('\n')).join('\n')}
float shapeDistance(vec2 p, float shadowSpread) {
    float r = max(0.0, radius + shadowSpread);
    vec2 halfSize = (bounds.zw-bounds.xy)*0.5 + vec2(shadowSpread);
    if (min(halfSize.x,halfSize.y) <= 0.0) return 100000.0;
    r = min(r, min(halfSize.x, halfSize.y));
    vec2 d = abs(p-(bounds.xy+bounds.zw)*0.5)-halfSize+vec2(r);
    vec2 q = max(d, vec2(0.0));
    float longest = max(q.x,q.y);
    float length = longest < 0.001 ? 0.0 : longest*pow(pow(q.x/longest,exponent)+pow(q.y/longest,exponent),1.0/exponent);
    return min(max(d.x,d.y),0.0)+length-r;
}

vec4 shadowLayer(vec2 point, vec4 geometry, vec4 color) {
    float distance = shapeDistance(point-geometry.xy, geometry.z);
    float density = geometry.w < 0.01 ? clamp(0.5-distance,0.0,1.0) : 1.0/(1.0+exp(clamp(3.4*distance/geometry.w,-30.0,30.0)));
    float alpha = color.a*density*(1.0-coverage(point,bounds,radius));
    return vec4(color.rgb*alpha,alpha);
}`, `
vec2 point = cogl_tex_coord0_in.xy * textureDimensions + textureOrigin;
${['A','B'].map(state => `vec4 result${state} = vec4(0.0);
${Array.from({length: 4}, (_, i) => `if (shadowColor${state}${i}.a > 0.0) {
    vec4 layer = shadowLayer(point,shadowGeometry${state}${i},shadowColor${state}${i});
    result${state} = layer + result${state}*(1.0-layer.a);
}`).join('\n')}`).join('\n')}
cogl_color_out = mix(resultA,resultB,fadeProgress)*cogl_color_in;
`, false);
    }
    updateShadow(g, layers, animation) {
        this.geometry = g;
        this.uniform('bounds', g.bounds);
        this.uniform('radius', [g.radius]); this.uniform('exponent', [g.exponent]);
        const key = JSON.stringify(layers);
        if (key !== this.targetKey || (this.timeline && (animation.duration === 0 || !St.Settings.get().enable_animations))) {
            this.queued = {layers, animation};
            if (!this.timeline || animation.duration === 0 || !St.Settings.get().enable_animations)
                this.startFade();
        } else this.queued = null;
        this.uploadLayers();
    }
    startFade() {
        const {layers, animation} = this.queued;
        this.queued = null;
        this.timeline?.stop(); this.timeline = null;
        this.from = this.to || [];
        this.to = layers; this.targetKey = JSON.stringify(layers);
        const duration = St.Settings.get().enable_animations ? animation.duration : 0;
        this.uniform('fadeProgress', [duration > 0 ? 0 : 1]);
        this.uploadLayers();
        if (!duration) return;
        const modes = {linear:Clutter.AnimationMode.LINEAR,
            'ease-out-cubic':Clutter.AnimationMode.EASE_OUT_CUBIC,
            'ease-out-quad':Clutter.AnimationMode.EASE_OUT_QUAD,
            'ease-in-out-cubic':Clutter.AnimationMode.EASE_IN_OUT_CUBIC};
        const timeline = Clutter.Timeline.new_for_actor(this.get_actor(), duration);
        this.timeline = timeline;
        timeline.set_progress_mode(modes[animation.easing]);
        timeline.connect('new-frame', () => {this.uniform('fadeProgress',[timeline.get_progress()]);this.queue_repaint();});
        timeline.connect('completed', () => {
            this.timeline = null;
            this.uniform('fadeProgress',[1]);
            if (this.queued) this.startFade();
        });
        timeline.start();
    }
    uploadLayers() {
        const g = this.geometry;
        for (const [state,layers] of [['A',this.from || []],['B',this.to || []]]) {
            for (let i = 0; i < 4; i++) {
                const s = layers[i];
                this.uniform(`shadowGeometry${state}${i}`, s ? [s.x*g.scale,s.y*g.scale,s.spread*g.scale,s.blur*g.scale] : [0,0,0,0]);
                const color = s ? rgba(s.color) : [0,0,0,0];
                if (s) color[3] *= s.opacity;
                this.uniform(`shadowColor${state}${i}`, color);
            }
        }
    }
    stop() { this.timeline?.stop(); this.timeline = null; this.queued = null; }

});

// Asynchronous, shared detection, bounded by the number of live processes.
// No /proc reads or pixel downloads in a paint callback.
export class ToolkitCache {
    constructor() { this.entries = new Map(); }
    watch(pid, callback) {
        if (pid <= 0) return () => {};
        let entry = this.entries.get(pid);
        if (!entry) {
            entry = {callbacks: new Set(), value: {}, cancel: new Gio.Cancellable()};
            this.entries.set(pid, entry);
            Gio.File.new_for_path(`/proc/${pid}/maps`).load_contents_async(entry.cancel, (file, result) => {
                try {
                    const [, bytes] = file.load_contents_finish(result);
                    const maps = new TextDecoder().decode(bytes.subarray(0, 4*1024*1024));
                    entry.value = {adwaita: /\/libadwaita-1\.so/.test(maps), handy: /\/libhandy-1\.so/.test(maps)};
                } catch (_) { /* Flatpak PID access may be denied: retain GPU alpha detection. */ }
                for (const notify of entry.callbacks) notify(entry.value);
            });
        }
        entry.callbacks.add(callback);
        callback(entry.value);
        return () => {
            entry.callbacks.delete(callback);
            if (!entry.callbacks.size) { entry.cancel.cancel(); this.entries.delete(pid); }
        };
    }
    destroy() { for (const entry of this.entries.values()) entry.cancel.cancel(); this.entries.clear(); }
}

export class WindowCorners {
    constructor(actor, surface, cache, changed) {
        this.actor = actor; this.surface = surface; this.effect = null; this.shadow = null;
        this.toolkit = {}; this.config = null; this.signals = []; this.pending = 0;
        const win = actor.meta_window;
        const schedule = () => {
            if (!this.pending) this.pending = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                this.pending = 0; changed(); return GLib.SOURCE_REMOVE;
            });
        };
        // Geometry/state changes only. Coalesce resize bursts into one update.
        for (const name of ['size-changed', 'position-changed', 'notify::fullscreen', 'notify::maximized-horizontally', 'notify::maximized-vertically'])
            this.signals.push([win, win.connect(name, schedule)]);
        this.signals.push([actor, actor.connect('first-frame', () => { visibleFrames.delete(actor); schedule(); })]);
        this.signals.push([actor, actor.connect('notify::opacity', () => { if (actor.opacity === 255) schedule(); })]);
        this.unwatch = cache.watch(win.get_pid(), value => { this.toolkit = value; schedule(); });
    }
    update(config) {
        this.config = config;
        const win = this.actor.meta_window;
        const state = {...this.toolkit,
            normal: [Meta.WindowType.NORMAL, Meta.WindowType.DIALOG, Meta.WindowType.MODAL_DIALOG].includes(win.get_window_type()) && !win.is_override_redirect(),
            fullscreen: win.fullscreen,
            maximized: win.maximized_horizontally && win.maximized_vertically,
            tiled: Boolean(win.get_tile_match()) || Boolean(win.maximized_horizontally) !== Boolean(win.maximized_vertically)};
        const g = windowGeometry(this.actor, this.surface, config);
        if (!Geometry.enabled(config, state) || !g) { this.remove(); return; }
        if (!this.effect) { this.effect = new CornersEffect(); this.surface.add_effect_with_name('gnoblin-window-corners', this.effect); }
        const shadow = config.shadow && ((!state.fullscreen && !state.maximized && !state.tiled) || config['keep-shadow']);
        this.effect.update(g, {...config, shadow: !!shadow});
        this.updateShadow(g, shadow ? config : null);
    }
    updateShadow(g, config) {
        if (!config) { this.shadow?.destroy(); this.shadow = null; return; }
        const layers = (Array.isArray(config.shadow) ? config.shadow : [config.shadow])
            .map(s => ({x: 0, y: 4, blur: 28, spread: 4, opacity: .6, color: '#000000', ...s}));
        let pad = Math.ceil(Math.max(...layers.map(s => s.blur*2 + Math.abs(s.spread) + Math.max(Math.abs(s.x), Math.abs(s.y)) + 2))*g.scale);
        // Retain enough space for the outgoing style during a crossfade.
        pad = Math.max(pad, this.shadow ? this.shadowPad : 0);
        this.shadowPad = pad;
        const [left, top, right, bottom] = g.bounds;
        const width = right-left, height = bottom-top;
        if (!this.shadow) {
            this.shadow = new St.Widget({reactive: false, style: 'background-color: white;'});
            this.shadow._gnoblinDecoration = true;
            this.shadowEffect = new ShadowEffect(); this.shadow.add_effect(this.shadowEffect);
            const effect = this.shadowEffect;
            this.shadow.connect('destroy', () => effect.stop());
            this.actor.insert_child_at_index(this.shadow, 0);
        }
        this.shadow.set_position(this.surface.x+left-pad, this.surface.y+top-pad);
        this.shadow.set_size(width+2*pad, height+2*pad);
        this.shadowEffect.updateShadow({...g, bounds:[pad,pad,pad+width,pad+height]},layers,
            {...Geometry.defaults['shadow-animation'], ...config['shadow-animation']});
    }

    remove() {
        if (this.effect) this.surface.remove_effect(this.effect);
        this.effect = null; this.shadow?.destroy(); this.shadow = null;
    }
    destroy() {
        if (this.pending) GLib.source_remove(this.pending);
        this.pending = 0; this.unwatch();
        for (const [object, id] of this.signals) object.disconnect(id);
        this.signals = []; this.remove();
    }
}

const BorderEffect = GObject.registerClass(class GnoblinBorderEffect extends GeometryEffect {
    vfunc_build_pipeline() {
        this.add_glsl_snippet(Cogl.SnippetHook.FRAGMENT, declarations + `
uniform float innerWidth;
uniform float outerWidth;
uniform vec4 innerColor;
uniform vec4 outerColor;
`, `
vec2 p = cogl_tex_coord0_in.xy * textureDimensions + textureOrigin;
float edge = coverage(p, bounds, radius);
float inner = max(0.0, edge - coverage(p, bounds + vec4(innerWidth,innerWidth,-innerWidth,-innerWidth), max(0.0,radius-innerWidth)));
float outer = max(0.0, coverage(p, bounds + vec4(-outerWidth,-outerWidth,outerWidth,outerWidth), radius+outerWidth) - edge);
float ia = inner*innerColor.a;
float oa = outer*outerColor.a;
cogl_color_out = vec4(innerColor.rgb*ia + outerColor.rgb*oa, ia+oa)*cogl_color_in;
`, false);
    }
    update(g, config) {
        this.uniform('bounds', g.bounds);
        this.uniform('radius', [g.radius]);
        this.uniform('exponent', [g.exponent]);
        this.uniform('innerWidth', [config['inner-width']*g.scale]);
        this.uniform('outerWidth', [config['outer-width']*g.scale]);
        this.uniform('innerColor', rgba(config['inner-color']));
        this.uniform('outerColor', rgba(config['outer-color']));
    }
});

export class WindowBorders {
    constructor(actor, surface, changed) {
        this.actor = actor; this.surface = surface; this.widget = null; this.pending = 0;
        const schedule = () => {
            if (!this.pending) this.pending = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                this.pending = 0; changed(); return GLib.SOURCE_REMOVE;
            });
        };
        const win = actor.meta_window;
        this.signals = ['size-changed', 'position-changed', 'notify::fullscreen', 'notify::maximized-horizontally', 'notify::maximized-vertically']
            .map(name => [win, win.connect(name, schedule)]);
        this.signals.push([actor, actor.connect('first-frame', () => { visibleFrames.delete(actor); schedule(); })]);
        this.signals.push([actor, actor.connect('notify::opacity', () => { if (actor.opacity === 255) schedule(); })]);
    }
    update(config) {
        const win = this.actor.meta_window;
        const state = {
            normal: [Meta.WindowType.NORMAL, Meta.WindowType.DIALOG, Meta.WindowType.MODAL_DIALOG].includes(win.get_window_type()) && !win.is_override_redirect(),
            fullscreen: win.fullscreen,
            maximized: win.maximized_horizontally && win.maximized_vertically,
            tiled: Boolean(win.get_tile_match()) || Boolean(win.maximized_horizontally) !== Boolean(win.maximized_vertically),
        };
        const g = windowGeometry(this.actor, this.surface, config);
        if (!Geometry.bordersEnabled(config, state) || !g) { this.remove(); return; }
        const pad = Math.ceil(config['outer-width']*g.scale)+2;
        const [left, top, right, bottom] = g.bounds;
        if (!this.widget) {
            this.widget = new St.Widget({reactive: false, style: 'background-color: white;'});
            this.widget._gnoblinDecoration = true;
            this.effect = new BorderEffect();
            this.widget.add_effect(this.effect);
            this.actor.add_child(this.widget);
        }
        this.widget.set_position(this.surface.x+left-pad, this.surface.y+top-pad);
        this.widget.set_size(right-left+2*pad, bottom-top+2*pad);
        this.effect.update({...g, bounds: [pad,pad,pad+right-left,pad+bottom-top]}, config);
    }
    remove() { this.widget?.destroy(); this.widget = null; this.effect = null; }
    destroy() {
        if (this.pending) GLib.source_remove(this.pending);
        this.pending = 0;
        for (const [object, id] of this.signals) object.disconnect(id);
        this.signals = []; this.remove();
    }
}
