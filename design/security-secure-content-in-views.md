# Secure content in a View: audit and fixes

Lane SecureGpu. Scope: commits `00835c067`, `14f904e24` and `25c0e975e` (a GLES subtree-to-texture path for `View`),
`dad838b0b` (`View:capture`, `fps`, PipeWire from a View) and `747351cc4` (the DMA-BUF close fade, stereo Views). The
question: can the pixels of a secure node reach a reader of a View that is not allowed them?

The answer is yes, twice. Both leaks were older than the GPU path, and the GPU path did not add one. Both are fixed.

## What reads a View's picture

| Reader                                                                          | Reads                                                             |
| ------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| The View on the screen                                                          | The display picture (the CPU image, or the GLES display texture). |
| `View:capture(path)`                                                            | The capture picture, after the lock's answer.                     |
| `ext_image_copy_capture` of a View, shared memory (the XR extension is one)     | The capture picture, copied through the capture table.            |
| `ext_image_copy_capture` of a View, into a GPU buffer                           | The capture picture, uploaded.                                    |
| A PipeWire stream of a View (`start_cast`)                                      | The capture picture, pumped through the capture table.            |
| `zwlr_screencopy`, `ext_image_copy_capture` of an output, the portal screenshot | The output's frame, with each secure layer replaced.              |
| A window's close or minimise fade                                               | The window's own layers, drawn from the buffer it kept.           |

The capture picture is the View drawn again with each secure node replaced by the grey an output capture draws
(`capture.secure-treatment = "mask"`) or left out (`"omit"`). On the CPU the composer draws it
(`Composer::draw_view`). On the GPU the composer describes a second set of eyes (the capture variant of `ViewScene`),
the GLES renderer draws it into a second texture, and a reader gets a readback of that texture, never of the display
texture (`Backend::read_view(scene, true, ..)` is the only call, in `capture_views.rs`). No reader reads a GPU
texture directly: the GPU-buffer copy uploads the readback.

## What was wrong

### 1. A secure ancestor above a View's source was ignored

The set of secure nodes was worked out from the View's source subtree only. A marked window or a node with `secure`
set, above the source, was not in the subtree, so nothing in the View was secure.

- A View of a popup of a secure window leaked the popup.
- A View of a node that sits under a secure shell node leaked everything in it, and moving a source under a secure
  node changed nothing.
- A View below a secure node, and a window below a secure node, were not secure layers on the output, so a capture of
  the output carried their pixels. Only canvases were checked against the whole tree.

The docs said "everything below a secure node is secure". It was true only inside the source.

Fix: one function, `Composer::find_secure`, works out which nodes are secure over the whole scene (a window the shell or
a rule marked, a node whose `secure` is true, and the subtree of each). The composer keeps the answer for the frame
(`secure_scope`) and takes it once, in `begin_frame_reading`. Every path asks that set: the layers of a window and of
its surfaces, of a canvas and of a View on the output, the CPU capture picture, and the capture variant of the GPU scene.
A change in the set damages the nodes it touches, so the frame a capture reads is drawn again with the new mark. The
per-canvas check and its damage are gone.

Tests, each written failing first (the red run is `SecureGpu-logs/sec1.log` and `red-popup.log` in the lane's log
directory):

- `view_secure_tests.rs`: `a_source_below_a_secure_ancestor_is_redacted_in_the_views_capture_and_the_view_is_a_secure_layer`,
  `a_view_below_a_secure_node_is_a_secure_layer_though_its_source_is_not`, `a_window_below_a_secure_node_is_a_secure_layer`,
  `moving_a_source_under_a_secure_node_redacts_it_at_once_and_a_sibling_is_not_secure`;
- `surface_tree_tests.rs`: `a_view_of_a_popup_of_a_secure_window_is_masked_in_its_capture`;
- `tests/view_capture_e2e.rs`: `a_node_above_the_views_source_that_is_secure_masks_every_reader_of_the_view`, on both
  renderers, for a session, a stream and a PNG.

### 2. One turn of stale picture after a `secure` mark

The pictures a capture copies from were published at the start of the frame, before the composer drew. A CPU-drawn View
that had just been redrawn with the mask was still served as its old, unmasked picture for the next turn, to
`ext_image_copy_capture` sessions of the View and to the copies into a GPU buffer. A reader that asked for a frame the
moment the shell was told the node was secure got the whole View, unmasked. A GLES View was not affected, because
`sync_view_readbacks` drops the readback of a View the composer drew again.

The existing tests did not see it: each waited for the first masked frame before it looked.

Fix: `State::begin_frame_reading` publishes the View pictures again as soon as the composer has drawn.

Test: `the_first_frame_a_reader_asks_for_after_a_mark_is_acknowledged_holds_none_of_the_pixels`. It read all 4096 pixels
of the secure window before the fix (`e2e1.log`) and none after, on both renderers.

## What was checked and is not a leak

Each is a test that runs on both paths unless it says otherwise (`view_secure_tests.rs`, `view_capture_e2e.rs`).

- **A secure window whose pixels are shared memory or a DMA-BUF**, marked by the window, by the node's `secure` value or
  by an ancestor, under `mask` and `omit`: the capture picture is the mask (or nothing) on every pixel, and the display
  still shows the window. On the GPU path the capture variant's layers never name the window's buffer: the secure window
  is replaced before the renderer is asked to import anything, so a DMA-BUF that cannot be imported cannot leak either.
  Test: `a_secure_window_is_redacted_in_a_views_capture_whatever_holds_its_pixels`.
- **Real pixels where the CPU masks grey (decision 4).** A GLES View of a window that shows a non-secure DMA-BUF samples
  the buffer, and the CPU path reads it back through the backend, or masks the window when the backend cannot read it.
  This is intended, and is in `docs/shell-api/views.md` ("Where a View is drawn"). The same buffer under a secure window
  is redacted on both: `the_same_gpu_buffer_is_redacted_with_the_mark_and_shown_without_it`,
  `without_the_mark_a_capture_reads_the_real_pixels_of_a_gpu_buffer_and_an_unreadable_one_is_masked`.
- **A capture of the output that shows a View.** The View's layer is secure when anything in its source is, so the output
  capture replaces it whole, and replaces the window too. Test:
  `a_capture_of_the_output_replaces_a_view_of_secure_content_and_the_window_itself`, and with a GLES View over a secure
  window in the running session, `a_view_drawn_on_the_gpu_over_a_secure_window_is_masked_in_a_capture_of_the_output`.
- **The close and minimise fade.** The fade draws the window's own buffer, a DMA-BUF included, through a layer that keeps
  the window's mark, so an output capture replaces it. A View of a fading window has no pixels of it, because the buffer
  is dropped for the fade. Test: `a_secure_window_that_shows_a_gpu_buffer_stays_secure_while_it_fades`.
- **A stereo View.** Both eyes of the capture variant sample only the mask, and the two paths agree. Test:
  `a_secure_node_is_redacted_in_both_eyes_of_a_stereo_view_as_on_the_cpu`.
- **A node that becomes secure after a stream started**, even when the View's `fps` leaves a second before the next draw:
  a new scene revision at once, and the next picture is the mask. Tests:
  `a_node_that_becomes_secure_is_redacted_in_the_next_picture_though_the_view_is_paced`, and the e2e above.
- **Texture and readback reuse.** A View whose source changes keeps no pixel of the old source
  (`a_view_keeps_no_pixels_of_a_source_it_had_and_each_source_is_redacted_on_its_own_status`). Two Views of one size, one
  of them secure, read by one renderer in turn and with the status swapped, never carry each other's pixels
  (`the_readback_of_one_view_never_carries_the_pixels_of_a_view_of_another_secure_status`). A texture is keyed by View
  and by display or capture variant, cleared before each draw and read into a fresh buffer, so nothing is reused.
- **The session lock.** The capture table asks the lock at every frame, for every path that reads a View. A session on a
  View is stopped, a stream ends, a still is answered `locked` and writes no file, and a cast started during the lock is
  refused. After the unlock a reader gets the window as it is then.
- **Textures and readbacks at the lock.** An output's renderer is given a cover that names no View, so it retires every
  texture, the display one and the capture one. The readbacks a reader was served from (CPU memory) and the textures
  the backend holds for them (the headless renderer; the DRM backend's probe context, which no cover frame reaches) are
  given back by `State::read_gpu_views` through `Backend::release_views` while the session is locked, and no View is
  read for a still the table is about to refuse. Tests:
  `the_lock_cuts_every_reader_of_a_view_and_none_is_served_a_picture_from_before_it` (both renderers),
  `a_locked_output_names_no_view_alive_so_the_renderer_gives_each_texture_back` (it asserts the release),
  `releasing_the_views_gives_back_every_texture_a_readback_made`,
  `a_frame_that_names_no_view_gives_back_the_display_and_the_capture_texture_alike`.
- **The XR extension.** It reads a View over `gnoblin_view_source_v1` and `ext_image_copy_capture` into shared memory, so
  it is the same reader as a `view`-role session. It has no path to a window's buffer or a texture of its own.

## What was not proved

- **A real GPU DMA-BUF imported by GLES.** llvmpipe on Agent37 cannot import a client's DMA-BUF, so the display texture of
  a non-secure DMA-BUF window was checked only structurally (the display scene names the buffer, the capture scene names
  it only when nothing is secure) and by the CPU path with a stand-in reader. The redaction does not depend on the
  import, so this does not weaken the secure case.
- **The DRM backend.** It implements `read_view` through its probe context and was not run. It reaches the same
  renderer function.
- **PipeWire buffers that are DMA-BUFs.** They are not built: a stream is one memfd plane per buffer.
- **A client that reads the output instead of the View** is the existing output capture, whose redaction is unchanged
  and which now also sees secure nodes below a secure ancestor.

## Rules this leaves

- A path that shows or reads a node asks the frame's secure scope. It does not work out secrecy from the subtree it
  happens to be drawing.
- A picture a reader copies from is published after the composer drew it, never before.
- A test that reads after a mark waits for nothing: the first frame after the acknowledgement is the one that counts.
