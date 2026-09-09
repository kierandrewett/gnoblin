import Shell from 'gi://Shell';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import * as Config from './gnoblinConfig.js';
import {WindowCorners, WindowBorders, ToolkitCache} from './gnoblinCorners.js';
import {BackdropRedraw} from './gnoblinBackdropRedraw.js';

const supportsShadowMask = Shell.BlurEffect.list_properties().some(p => p.name === 'ignore-shadow-pixels');

function shaderSource(source) {
    return `uniform sampler2D gnoblin_texture;
uniform float gnoblin_width;
uniform float gnoblin_height;
${source}
void main() {
    vec2 uv = cogl_tex_coord_in[0].st;
    vec4 pixel = texture2D(gnoblin_texture, uv);
    vec4 straight = vec4(pixel.rgb / max(pixel.a, 0.00001), pixel.a);
    vec4 result = gnoblin_effect(straight, uv);
    float alpha = clamp(result.a, 0.0, pixel.a);
    cogl_color_out = vec4(clamp(result.rgb, 0.0, 1.0) * alpha, alpha) * cogl_color_in;
}`;
}

const WindowShader = GObject.registerClass(class GnoblinWindowShader extends Clutter.ShaderEffect {
    _init(source, uniforms) {
        super._init();
        this.set_shader_source(source);
        const sampler = new GObject.Value();
        sampler.init(GObject.TYPE_INT);
        sampler.set_int(0);
        this.set_uniform_value('gnoblin_texture', sampler);
        for (const [name, value] of Object.entries(uniforms))
            this.setFloat(name, value);
    }

    setFloat(name, number) {
        const value = new GObject.Value();
        value.init(GObject.TYPE_FLOAT);
        value.set_float(number);
        this.set_uniform_value(name, value);
    }
});

export class WindowRules {
    constructor() {
        this._config = Config.settings;
        this._actors = new Map();
        this._backdropRedraw = new BackdropRedraw();
        this._cornerToolkits = new ToolkitCache();
        this._sources = new Map();
        this._map = global.window_manager.connect('map', (_wm, actor) => this._apply(actor));
        this._focus = global.display.connect('notify::focus-window', () => this.refresh());
    }

    refresh(config = this._config) {
        this._config = config;
        for (const actor of global.get_window_actors())
            this._apply(actor);
        const paths = new Set(config['window-rules'].filter(rule => rule.shader).map(rule => this._shaderPath(rule.shader)));
        for (const [path, entry] of this._sources) {
            if (paths.has(path)) continue;
            entry.monitor?.cancel();
            if (entry.timer) GLib.source_remove(entry.timer);
            this._sources.delete(path);
        }
    }

    _shaderPath(path) {
        if (path.startsWith('~/')) return GLib.build_filenamev([GLib.get_home_dir(), path.slice(2)]);
        if (GLib.path_is_absolute(path)) return path;
        const override = GLib.getenv('GNOBLIN_CONFIG');
        const directory = override ? GLib.path_get_dirname(override) :
            GLib.build_filenamev([GLib.get_user_config_dir(), 'gnoblin']);
        return GLib.build_filenamev([directory, path]);
    }

    _shader(path) {
        if (this._sources.has(path)) return this._sources.get(path).source;
        const file = Gio.File.new_for_path(path);
        const entry = {source: null, monitor: null, timer: 0};
        this._sources.set(path, entry);
        const load = () => {
            try {
                if (file.query_info('standard::size', Gio.FileQueryInfoFlags.NONE, null).get_size() > 65536)
                    throw new Error('shader exceeds 64 KiB');
                const [, bytes] = file.load_contents(null);
                const source = shaderSource(new TextDecoder('utf-8', {fatal: true}).decode(bytes));
                Shell.gnoblin_validate_shader(source);
                entry.source = source;
            } catch (error) {
                console.warn(`gnoblin-shader: keeping previous effect for ${path}: ${error.message}`);
            }
        };
        try {
            entry.monitor = file.get_parent().monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
            entry.monitor.connect('changed', (_monitor, changed, other) => {
                if (!changed?.equal(file) && !other?.equal(file)) return;
                if (entry.timer) GLib.source_remove(entry.timer);
                entry.timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
                    entry.timer = 0;
                    load();
                    this.refresh();
                    return GLib.SOURCE_REMOVE;
                });
            });
        } catch (error) {
            console.warn(`gnoblin-shader: cannot watch ${path}: ${error.message}`);
        }
        load();
        return entry.source;
    }

    _apply(actor) {
        let entry = this._actors.get(actor);
        // Shadows are inserted before the client surface. A new rules owner
        // must never attach client effects or size listeners to that decoration.
        const surface = entry?.surface || actor.get_children().find(child => !child._gnoblinDecoration);
        if (!surface || !actor.meta_window) return;
        if (!entry) {
            const title = actor.meta_window.connect('notify::title', () => this._apply(actor));
            const destroy = actor.connect('destroy', () => {
                actor.meta_window?.disconnect(title);
                entry.corners?.destroy();
                entry.borders?.destroy();
                this._actors.delete(actor);
            });
            const width = surface.connect('notify::width', () => this._apply(actor));
            const height = surface.connect('notify::height', () => this._apply(actor));
            entry = {surface, title, destroy, width, height, opacity: surface.opacity, blur: null,
                shader: null, shaderKey: null, corners: null, borders: null};
            this._actors.set(actor, entry);
        }
        const effects = Config.windowEffects(Config.windowProperties(actor.meta_window), this._config);
        surface.opacity = Math.round(entry.opacity * effects.opacity);
        if (effects.blur > 0) {
            if (!entry.blur) {
                entry.blur = new Shell.BlurEffect({mode: Shell.BlurMode.BACKGROUND_MASKED, brightness: 1});
                // Keep backdrop capture outside the surface shader. Blur paints
                // the client twice (alpha mask and colour); each child paint
                // must start with a fresh shader effect chain.
                actor.add_effect_with_name('gnoblin-window-blur', entry.blur);
            }
            entry.blur.radius = effects.blur;
            if (actor._gnoblinBlurRegion && entry.blur.set_region)
                entry.blur.set_region(...actor._gnoblinBlurRegion);
            // Client alpha describes glass tint, not blur strength. Using it as
            // coverage mixes sharp pixels back into translucent panel interiors.
            entry.blur.mask_opacity = 0;
            if (supportsShadowMask) entry.blur.ignore_shadow_pixels = effects['blur-ignore-shadows'];
        } else if (entry.blur) {
            actor.remove_effect(entry.blur);
            entry.blur = null;
        }
        this._backdropRedraw.set(actor, !!entry.blur);
        const source = effects.shader ? this._shader(this._shaderPath(effects.shader)) : null;
        const key = source ? JSON.stringify([source, effects['shader-uniforms']]) : null;
        if ((!effects.shader || source) && entry.shaderKey !== key) {
            const shader = source ? new WindowShader(source, effects['shader-uniforms']) : null;
            if (entry.shader) surface.remove_effect(entry.shader);
            entry.shader = shader;
            entry.shaderKey = key;
            if (shader) surface.add_effect_with_name('gnoblin-window-shader', shader);
        }
        if (entry.shader) {
            entry.shader.setFloat('gnoblin_width', surface.width);
            entry.shader.setFloat('gnoblin_height', surface.height);
        }
        if (effects.borders['inner-width'] > 0 || effects.borders['outer-width'] > 0) {
            if (!entry.borders) entry.borders = new WindowBorders(actor, surface, () => this._apply(actor));
            entry.borders.update(effects.borders);
        } else if (entry.borders) {
            entry.borders.destroy(); entry.borders = null;
        }
        if (effects.corners.radius > 0 && effects.corners.mode !== 'off') {
            if (!entry.corners) entry.corners = new WindowCorners(actor, surface, this._cornerToolkits, () => this._apply(actor));
            entry.corners.update(effects.corners);
        } else if (entry.corners) {
            entry.corners.destroy(); entry.corners = null;
        }
    }

    destroy() {
        this._backdropRedraw.destroy();
        global.window_manager.disconnect(this._map);
        global.display.disconnect(this._focus);
        for (const entry of this._sources.values()) {
            entry.monitor?.cancel();
            if (entry.timer) GLib.source_remove(entry.timer);
        }
        this._sources.clear();
        for (const [actor, entry] of this._actors) {
            actor.disconnect(entry.destroy);
            actor.meta_window.disconnect(entry.title);
            entry.surface.disconnect(entry.width);
            entry.surface.disconnect(entry.height);
            entry.surface.opacity = entry.opacity;
            if (entry.blur) actor.remove_effect(entry.blur);
            if (entry.shader) entry.surface.remove_effect(entry.shader);
            entry.corners?.destroy();
            entry.borders?.destroy();
        }
        this._actors.clear();
        this._cornerToolkits.destroy();
    }
}
