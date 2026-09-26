import Shell from "gi://Shell";
import Clutter from "gi://Clutter";
import Gio from "gi://Gio";
import GLib from "gi://GLib";
import GObject from "gi://GObject";
import Meta from "gi://Meta";
import { WindowFrame } from "./gnoblinFrames.js";
import * as Config from "./gnoblinConfig.js";
import * as Workspaces from "./gnoblinWorkspaces.js";
import { WindowCorners, WindowBorders, ToolkitCache } from "./gnoblinCorners.js";
import { BackdropRedraw } from "./gnoblinBackdropRedraw.js";
import { BackgroundEffects } from "./gnoblinBackgroundEffects.js";

const supportsShadowMask = Shell.BlurEffect.list_properties().some((p) => p.name === "ignore-shadow-pixels");

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

function isWindowSurface(child) {
    return !child._gnoblinDecoration && child.get_name() !== "gnoblin-native-frame";
}

const WindowShader = GObject.registerClass(
    class GnoblinWindowShader extends Clutter.ShaderEffect {
        _init(source, uniforms) {
            super._init();
            this.set_shader_source(source);
            const sampler = new GObject.Value();
            sampler.init(GObject.TYPE_INT);
            sampler.set_int(0);
            this.set_uniform_value("gnoblin_texture", sampler);
            for (const [name, value] of Object.entries(uniforms)) this.setFloat(name, value);
        }

        setFloat(name, number) {
            const value = new GObject.Value();
            value.init(GObject.TYPE_FLOAT);
            value.set_float(number);
            this.set_uniform_value(name, value);
        }
    },
);

export class WindowRules {
    constructor() {
        this._destroyed = false;
        this._config = Config.settings;
        this._actors = new Map();
        this._initialWorkspaceHandled = new WeakSet();
        this._backdropRedraw = new BackdropRedraw();
        this._cornerToolkits = new ToolkitCache();
        this._sources = new Map();
        this._pending = new Set();
        this._pendingSpare = new Set();
        this._pendingId = 0;
        this._backgroundEffects = new BackgroundEffects((actor) => this._apply(actor));
        this._updateRuleDependencies(this._config);
        this._map = global.window_manager.connect("map", (_wm, actor) => {
            this._assignInitialWorkspace(actor.meta_window);
            this._apply(actor);
        });
        this._workspaceRemoved = global.workspace_manager.connect("workspace-removed", () =>
            this._workspaceNumbersChanged(),
        );
        this._workspacesReordered = global.workspace_manager.connect("workspaces-reordered", () =>
            this._workspaceNumbersChanged(),
        );
        this._focusedActor = global.display.focus_window?.get_compositor_private() ?? null;
        this._focus = global.display.connect("notify::focus-window", () => {
            const previous = this._focusedActor;
            this._focusedActor = global.display.focus_window?.get_compositor_private() ?? null;
            if (!this._hasFocusedRules) return;
            if (previous) this._schedule(previous);
            if (this._focusedActor) this._schedule(this._focusedActor);
        });
        this._backgroundEffects.refresh();
    }

    _schedule(actor) {
        if (this._destroyed) return;
        if (!this._actors.has(actor)) return;
        this._pending.add(actor);
        if (this._pendingId) return;
        this._pendingId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._pendingId = 0;
            const pending = this._pending;
            this._pending = this._pendingSpare;
            this._pendingSpare = pending;
            for (const value of pending) if (this._actors.has(value)) this._apply(value);
            pending.clear();
            return GLib.SOURCE_REMOVE;
        });
    }

    _assignInitialWorkspace(window) {
        if (this._destroyed || this._initialWorkspaceHandled.has(window)) return;
        // Mutter keeps attached dialogs on their parent's workspace. Applying
        // an app placement rule to one would break that relationship.
        if (window.get_transient_for?.()) {
            this._initialWorkspaceHandled.add(window);
            return;
        }
        if (Meta.gnoblin_layer_namespace(window) !== null) {
            this._initialWorkspaceHandled.add(window);
            return;
        }
        // Evaluate placement once the actor maps, after the client has sent
        // its initial title and application ID. Override-redirect windows
        // have no workspace and cannot receive a workspace placement rule.
        if (!window.get_workspace()) return;
        this._initialWorkspaceHandled.add(window);
        try {
            const target = Config.initialWorkspaceTarget(Config.windowProperties(window), this._config);
            if (!target) return;
            const workspace = Workspaces.resolve(target);
            if (!workspace) throw new Error("target workspace is unavailable");
            window.change_workspace(workspace);
            if (window.get_workspace() !== workspace)
                throw new Error("Mutter did not move the window to the target workspace");
        } catch (error) {
            console.warn(`gnoblin-window-rule: initial workspace placement failed: ${error.message}`);
        }
    }

    _workspaceNumbersChanged() {
        if (this._destroyed || !this._hasWorkspaceNumberRules) return;
        for (const [actor, entry] of this._actors) {
            const workspace = actor.meta_window?.get_workspace();
            const number = workspace ? workspace.index() + 1 : null;
            if (number === entry.workspaceNumber) continue;
            entry.workspaceNumber = number;
            this._schedule(actor);
        }
    }

    _updateRuleDependencies(config) {
        this._hasFocusedRules = Boolean(Meta.gnoblin_window_frame_get);
        this._hasTitleRules = Boolean(Meta.gnoblin_window_frame_get);
        this._hasWorkspaceRules = false;
        this._hasWorkspaceNumberRules = false;
        this._shaderPaths = new Map();
        this._shaderSourcePaths = new Set();
        for (const rule of config["window-rules"]) {
            this._hasFocusedRules ||= Object.hasOwn(rule.match, "focused");
            this._hasTitleRules ||= Object.hasOwn(rule.match, "title");
            this._hasWorkspaceRules ||=
                Object.hasOwn(rule.match, "workspace-id") || Object.hasOwn(rule.match, "workspace-number");
            this._hasWorkspaceNumberRules ||= Object.hasOwn(rule.match, "workspace-number");
            if (rule.shader) {
                const path = this._shaderPath(rule.shader);
                this._shaderPaths.set(rule.shader, path);
                this._shaderSourcePaths.add(path);
            }
        }
    }

    refresh(config = this._config) {
        if (this._destroyed) return;
        this._config = config;
        this._updateRuleDependencies(config);
        for (const actor of global.get_window_actors()) this._apply(actor);
        for (const [path, entry] of this._sources) {
            if (this._shaderSourcePaths.has(path)) continue;
            entry.monitor?.cancel();
            if (entry.timer) GLib.source_remove(entry.timer);
            this._sources.delete(path);
        }
    }

    _shaderPath(path) {
        if (path.startsWith("~/")) return GLib.build_filenamev([GLib.get_home_dir(), path.slice(2)]);
        if (GLib.path_is_absolute(path)) return path;
        const override = GLib.getenv("GNOBLIN_CONFIG");
        const directory = override
            ? GLib.path_get_dirname(override)
            : GLib.build_filenamev([GLib.get_user_config_dir(), "gnoblin"]);
        return GLib.build_filenamev([directory, path]);
    }

    _shader(path) {
        if (this._sources.has(path)) return this._sources.get(path).source;
        const file = Gio.File.new_for_path(path);
        const entry = { source: null, monitor: null, timer: 0 };
        this._sources.set(path, entry);
        const load = () => {
            try {
                if (file.query_info("standard::size", Gio.FileQueryInfoFlags.NONE, null).get_size() > 65536)
                    throw new Error("shader exceeds 64 KiB");
                const [, bytes] = file.load_contents(null);
                const source = shaderSource(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
                Shell.gnoblin_validate_shader(source);
                entry.source = source;
            } catch (error) {
                console.warn(`gnoblin-shader: keeping previous effect for ${path}: ${error.message}`);
            }
        };
        try {
            entry.monitor = file.get_parent().monitor_directory(Gio.FileMonitorFlags.WATCH_MOVES, null);
            entry.monitor.connect("changed", (_monitor, changed, other) => {
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

    _forgetSurface(entry, surface, surfaceAlive, restoreOpacity = false) {
        if (entry.surface !== surface) return;
        if (surfaceAlive) {
            if (entry.surfaceDestroy) surface.disconnect(entry.surfaceDestroy);
            if (entry.width) surface.disconnect(entry.width);
            if (entry.height) surface.disconnect(entry.height);
            if (restoreOpacity) surface.opacity = entry.opacity;
            if (entry.shader) surface.remove_effect(entry.shader);
        }
        entry.surface = null;
        entry.surfaceDestroy = 0;
        entry.width = 0;
        entry.height = 0;
        entry.opacity = null;
        // Surface actors can be replaced while the MetaWindowActor remains
        // mapped. Effects and size observers belong to that individual actor.
        entry.shader = null;
        entry.shaderKey = null;
        entry.corners?.destroy(!surfaceAlive);
        entry.corners = null;
        entry.borders?.destroy();
        entry.borders = null;
    }

    _trackSurface(actor, entry, surface) {
        entry.surface = surface;
        entry.opacity = surface.opacity;
        entry.width = surface.connect("notify::width", () => this._schedule(actor));
        entry.height = surface.connect("notify::height", () => this._schedule(actor));
        entry.surfaceDestroy = surface.connect("destroy", () => {
            if (entry.surface !== surface) return;
            this._forgetSurface(entry, surface, false);
            this._schedule(actor);
        });
    }

    _apply(actor) {
        if (this._destroyed) return;
        let entry = this._actors.get(actor);
        // Shadows are inserted before the client surface. A new rules owner
        // must never attach client effects or size listeners to that decoration.
        const surface = actor.get_children().find(isWindowSurface) ?? null;
        if (!actor.meta_window) return;
        const currentWorkspace = actor.meta_window.get_workspace();
        if (!entry) {
            const title = actor.meta_window.connect("notify::title", () => {
                if (this._hasTitleRules) this._schedule(actor);
            });
            const workspaceChanged = actor.meta_window.connect("workspace-changed", () => {
                if (this._hasWorkspaceRules) this._schedule(actor);
            });
            const destroy = actor.connect("destroy", () => {
                this._actors.delete(actor);
                this._pending.delete(actor);
                if (entry.surface)
                    this._forgetSurface(entry, entry.surface, actor.get_children().includes(entry.surface));
                actor.meta_window?.disconnect(title);
                actor.meta_window?.disconnect(workspaceChanged);
                entry.corners?.destroy();
                entry.borders?.destroy();
                entry.frame?.destroy();
            });
            const childAdded = actor.connect("child-added", (_actor, child) => {
                if (isWindowSurface(child)) this._schedule(actor);
            });
            const childRemoved = actor.connect("child-removed", (_actor, child) => {
                if (entry.surface === child) {
                    this._forgetSurface(entry, child, true);
                    this._schedule(actor);
                }
            });
            entry = {
                surface: null,
                title,
                workspaceChanged,
                destroy,
                childAdded,
                childRemoved,
                surfaceDestroy: 0,
                width: 0,
                height: 0,
                opacity: null,
                workspaceNumber: currentWorkspace ? currentWorkspace.index() + 1 : null,
                blur: null,
                shader: null,
                shaderKey: null,
                corners: null,
                borders: null,
            };
            this._actors.set(actor, entry);
        }
        if (!surface) {
            if (entry.surface) this._forgetSurface(entry, entry.surface, false);
            return;
        }
        if (entry.surface !== surface) {
            if (entry.surface) this._forgetSurface(entry, entry.surface, actor.get_children().includes(entry.surface));
            this._trackSurface(actor, entry, surface);
        }
        entry.workspaceNumber = currentWorkspace ? currentWorkspace.index() + 1 : null;
        const effects = Config.windowEffects(Config.windowProperties(actor.meta_window), this._config);
        if (Meta.gnoblin_window_frame_get?.(actor.meta_window).recursiveUnpack().supported) {
            if (!entry.frame) entry.frame = new WindowFrame(actor, () => this._schedule(actor));
            entry.frame.update(effects.frame, effects.corners);
        }
        surface.opacity = Math.round(entry.opacity * effects.opacity);
        const standardBlur = this._backgroundEffects.owns(actor);
        if (standardBlur) {
            const policy = Config.windowEffects(Config.windowProperties(actor.meta_window), this._config, 24);
            this._backgroundEffects.setRadius(actor, policy.blur);
        }
        if (effects.blur > 0 && !standardBlur) {
            if (!entry.blur) {
                entry.blur = new Shell.BlurEffect({ mode: Shell.BlurMode.BACKGROUND_MASKED, brightness: 1 });
                // Keep backdrop capture outside the surface shader. Blur paints
                // the client twice (alpha mask and colour); each child paint
                // must start with a fresh shader effect chain.
                actor.add_effect_with_name("gnoblin-window-blur", entry.blur);
            }
            entry.blur.radius = effects.blur;
            if (actor._gnoblinBlurRegion && entry.blur.set_region) entry.blur.set_region(...actor._gnoblinBlurRegion);
            // Client alpha describes glass tint, not blur strength. Using it as
            // coverage mixes sharp pixels back into translucent panel interiors.
            entry.blur.mask_opacity = 0;
            if (supportsShadowMask) entry.blur.ignore_shadow_pixels = effects["blur-ignore-shadows"];
        } else if (entry.blur) {
            actor.remove_effect(entry.blur);
            entry.blur = null;
        }
        this._backdropRedraw.set(actor, !!entry.blur);
        const source = effects.shader
            ? this._shader(this._shaderPaths.get(effects.shader) ?? this._shaderPath(effects.shader))
            : null;
        const key = source ? JSON.stringify([source, effects["shader-uniforms"]]) : null;
        if ((!effects.shader || source) && entry.shaderKey !== key) {
            const shader = source ? new WindowShader(source, effects["shader-uniforms"]) : null;
            if (entry.shader) surface.remove_effect(entry.shader);
            entry.shader = shader;
            entry.shaderKey = key;
            if (shader) surface.add_effect_with_name("gnoblin-window-shader", shader);
        }
        if (entry.shader) {
            entry.shader.setFloat("gnoblin_width", surface.width);
            entry.shader.setFloat("gnoblin_height", surface.height);
        }
        if (effects.borders["inner-width"] > 0 || effects.borders["outer-width"] > 0) {
            if (!entry.borders) entry.borders = new WindowBorders(actor, surface, () => this._schedule(actor));
            entry.borders.update(effects.borders);
        } else if (entry.borders) {
            entry.borders.destroy();
            entry.borders = null;
        }
        if (effects.corners.radius > 0 && effects.corners.mode !== "off") {
            if (!entry.corners)
                entry.corners = new WindowCorners(actor, surface, this._cornerToolkits, () => this._schedule(actor));
            entry.corners.update(effects.corners);
        } else if (entry.corners) {
            entry.corners.destroy();
            entry.corners = null;
        }
    }

    destroy() {
        if (this._destroyed) return;
        this._destroyed = true;
        if (this._pendingId) GLib.source_remove(this._pendingId);
        this._pendingId = 0;
        this._pending.clear();
        this._pendingSpare.clear();
        this._backgroundEffects.destroy();
        this._backdropRedraw.destroy();
        global.window_manager.disconnect(this._map);
        global.workspace_manager.disconnect(this._workspaceRemoved);
        global.workspace_manager.disconnect(this._workspacesReordered);
        global.display.disconnect(this._focus);
        for (const entry of this._sources.values()) {
            entry.monitor?.cancel();
            if (entry.timer) GLib.source_remove(entry.timer);
        }
        this._sources.clear();
        for (const [actor, entry] of this._actors) {
            actor.disconnect(entry.destroy);
            actor.disconnect(entry.childAdded);
            actor.disconnect(entry.childRemoved);
            actor.meta_window.disconnect(entry.title);
            actor.meta_window.disconnect(entry.workspaceChanged);
            if (entry.surface)
                this._forgetSurface(entry, entry.surface, actor.get_children().includes(entry.surface), true);
            if (entry.blur) actor.remove_effect(entry.blur);
            entry.corners?.destroy();
            entry.borders?.destroy();
            entry.frame?.destroy();
        }
        this._actors.clear();
        this._cornerToolkits.destroy();
    }
}
