/* Annotations for animation APIs implemented in src/config/lua/gnoblin-lua-animation.c,
 * src/config/lua/gnoblin-lua-contexts.c, and the native compositor animation patch
 * patches/mutter/99-typed-window-api/0024-native-animation-runtime.patch. */

/*** lua
---@class AnimationEaseBezier
---@field type "cubic-bezier" Easing defined by four control points.
---@field x1 number First control point x, from 0 to 1.
---@field y1 number First control point y.
---@field x2 number Second control point x, from 0 to 1.
---@field y2 number Second control point y.

---@class AnimationFrame
---@field x? number Horizontal translation.
---@field y? number Vertical translation.
---@field scale? number Uniform scale.
---@field scale_x? number Horizontal scale.
---@field scale_y? number Vertical scale.
---@field rotation? number Rotation in degrees.
---@field opacity? number Opacity from 0 to 1.

---@class AnimationArgs
---@field name string Unique name, 1 to 80 ASCII letters, digits, underscores, or hyphens.
---@field event string Lifecycle event supported by the animation.
---@field enable? boolean Whether the named animation is enabled. Default true; false removes it.
---@field duration? integer Duration in milliseconds, from 0 to 10000.
---@field ease? string|AnimationEaseBezier Named easing curve or cubic-bezier curve.
---@field from? AnimationFrame Starting actor properties.
---@field to? AnimationFrame Ending actor properties.
---@field keyframes? table Optional animation keyframes.
---@field origin? string|number[] Pivot name or normalized x,y coordinates.
---@field target? string Animation target hint.

---@class AnimationInfo
---@field name string Animation name.
---@field enable? boolean Whether a custom animation is enabled.
---@field event? string Lifecycle event; empty for animations without one.
---@field duration? integer Duration in milliseconds.
---@field ease? string|AnimationEaseBezier Easing curve.
---@field builtin? boolean True for a built-in animation.
---@field previewable? boolean Whether this animation supports preview controls.
---@field from? AnimationFrame Starting actor properties.
---@field to? AnimationFrame Ending actor properties.
---@field keyframes? table Optional animation keyframes.
---@field origin? string|number[] Animation pivot.
---@field target? string Animation target hint.
---@field revision? integer Configuration revision for this record.

---@class AnimationSurface
---@field id string Stable surface ID.
---@field namespace string Layer-shell namespace.
---@field title string Surface title; may be empty.

---@class AnimationInspectArgs
---@field name string Animation name.
---@field target? string Target ID; defaults to "active".
---@field event? string Lifecycle event override.
---@field target_type? "window"|"layer"|"namespace" Target identifier kind. Default "window".
---@field targetType? "window"|"layer"|"namespace" Legacy spelling of target_type.

---@class AnimationPreviewArgs
---@field name string Animation name.
---@field target? string Target ID; defaults to "active".
---@field event? string Lifecycle event override.
---@field target_type? "window"|"layer"|"namespace" Target identifier kind. Default "window".
---@field targetType? "window"|"layer"|"namespace" Legacy spelling of target_type.
---@field autoplay? boolean Start the preview immediately. Default false.

---@class AnimationInspection
---@field name string Animation name.
---@field event string Lifecycle event.
---@field target string Stable target ID.
---@field target_type "window"|"layer"|"namespace" Target identifier kind.
---@field duration integer Duration in milliseconds.
---@field ease string|AnimationEaseBezier Easing curve.
---@field from AnimationFrame Starting actor properties.
---@field to AnimationFrame Ending actor properties.
---@field keyframes? table Optional animation keyframes.
---@field properties table Properties of the target window or layer.
---@field context table Compositor context used to resolve the animation.
---@field spec table Resolved animation specification.

---@class AnimationPreview
---@field id string Preview session ID.
---@field session string Preview session ID used by control calls.
---@field name string Animation name.
---@field event string Lifecycle event.
---@field target string Stable target ID.
---@field target_type "window"|"layer"|"namespace" Target identifier kind.
---@field progress number Preview progress from 0 to 1.
---@field playing boolean Whether the timeline is playing.
---@field paused boolean Whether the timeline is paused.
---@field revision integer Configuration revision.
---@field spec table Resolved animation specification.

---@class AnimationStopResult
---@field ok boolean True when the preview was stopped.
---@field session string Stopped preview session ID.

---@class AnimationSeekArgs
---@field session string Preview session ID.
---@field progress number Progress from 0 to 1.

---@class AnimationStepArgs
---@field session string Preview session ID.
---@field milliseconds integer Step duration, from 1 to 60000.

---@class AnimationSessionArgs
---@field session string Preview session ID.

---Declare or update a named animation in configuration.
---@param animation AnimationArgs
function gnoblin.animation(animation) end

---Return the configured native animation records.
---@return AnimationInfo[]
function gnoblin.animations.list() end

---Return an animation by name, or nil if it is not registered.
---@param name string Animation name, 1 to 80 ASCII letters, digits, underscores, or hyphens.
---@return AnimationInfo|nil
function gnoblin.animations.get(name) end

---List visible layer-shell surfaces that can be targeted by animations.
---@return {surfaces: AnimationSurface[]}
function gnoblin.animations.surfaces() end

---Resolve an animation against a visible target without starting it.
---@param spec AnimationInspectArgs
---@return AnimationInspection
function gnoblin.animations.inspect(spec) end

---Start a controllable preview of an animation on a visible target.
---@param spec AnimationPreviewArgs
---@return Operation<AnimationPreview>
function gnoblin.animations.preview(spec) end

---Set a preview session to a progress value from 0 to 1.
---@param request AnimationSeekArgs
---@return Operation<AnimationPreview>
function gnoblin.animations.seek(request) end

---Advance a preview session by a duration in milliseconds.
---@param request AnimationStepArgs
---@return Operation<AnimationPreview>
function gnoblin.animations.step(request) end

---Resume a preview session.
---@param request AnimationSessionArgs
---@return Operation<AnimationPreview>
function gnoblin.animations.play(request) end

---Pause a preview session.
---@param request AnimationSessionArgs
---@return Operation<AnimationPreview>
function gnoblin.animations.pause(request) end

---Stop a preview session and restore the target's original actor state.
---@param request AnimationSessionArgs
---@return Operation<AnimationStopResult>
function gnoblin.animations.stop(request) end

---Set preview progress from an AnimationPreview record.
---@param progress number Progress from 0 to 1.
---@return Operation<AnimationPreview>
function AnimationPreview:seek(progress) end

---Advance a preview by milliseconds from an AnimationPreview record.
---@param milliseconds integer Duration from 1 to 60000.
---@return Operation<AnimationPreview>
function AnimationPreview:step(milliseconds) end

---Resume this preview.
---@return Operation<AnimationPreview>
function AnimationPreview:play() end

---Pause this preview.
---@return Operation<AnimationPreview>
function AnimationPreview:pause() end

---Stop this preview and restore the target's original actor state.
---@return Operation<AnimationStopResult>
function AnimationPreview:stop() end
*/
