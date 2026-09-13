#include <clutter/clutter.h>
/**
 * frame_probe_save:
 * @view: the view just painted
 * @path: output PNG
 *
 * Returns: whether the displayed framebuffer was read successfully
 */
gboolean frame_probe_save(ClutterStageView* view, const char* path);
/**
 * frame_probe_arm:
 * @stage: the nested compositor stage
 * @path: output PNG for the next paint before buffer swap
 */
void frame_probe_arm(ClutterStage* stage, const char* path);
void frame_probe_stop(void);
