//! Tests of the device and output split, the frames in flight, the texture cache, the allocator and the pipeline cache file.
//!
//! The failure modes, written before the tests:
//!
//! 1. A frame that changed nothing still submits, uploads or allocates: the idle and steady tests.
//! 2. A commit's damage uploads more than it must, or less (a texture left with an old row): the damage tests, which look at
//!    the bytes uploaded and at the pixels drawn.
//! 3. A texture shows a picture older than the surface has: a commit the cache did not see in order, a frame that failed.
//! 4. A texture is destroyed under a frame the GPU has not finished, or is never destroyed: the retire and eviction tests.
//! 5. `draw` waits for the GPU, or lets more than two frames pile up: the gate tests, which hold the GPU back with a
//!    semaphore the test owns.
//! 6. A race between the frames in flight that no assertion on pixels would see: synchronisation validation, which the
//!    drawing tests run with, and a test that a deliberate race is reported.
//! 7. A pipeline cache file that is corrupt, of another device or half written takes the renderer down or is kept: the cache
//!    file tests.
//! 8. Two outputs on one device upload a window twice, or one's loss is not the other's: the shared-device tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gnoblin_render_frame::frame::ContentKey;

use super::*;
use crate::textures::{IDLE_FRAMES, disjoint};

/// A device, or [`None`] with a printed reason when this machine has no Vulkan.
fn device() -> Option<Rc<VulkanDevice>> {
    match VulkanDevice::with_options(&options()) {
        Ok(device) => Some(device),
        Err(error @ (VulkanError::NoLoader { .. } | VulkanError::NoDevice { .. })) => {
            println!("skipping: {error}; there is no Vulkan device on this machine");
            None
        }
        Err(error) => panic!("the Vulkan device could not be created: {error}"),
    }
}

/// An output of [`SIZE`] on `device`.
fn output(device: &Rc<VulkanDevice>) -> Vulkan {
    Vulkan::with_device(device, SIZE).expect("an output")
}

/// `layer` named as commit `commit` of surface `surface`, which changed `damage` of its buffer.
fn keyed(mut layer: Layer, surface: u64, commit: u64, damage: Option<Vec<Rect>>) -> Layer {
    if let Layer::Surface(held) = &mut layer {
        held.content = Some(Arc::new(ContentKey {
            surface,
            commit,
            damage: damage.map(Into::into),
        }));
    }
    layer
}

/// A flat snapshot of `colour`, `edge` pixels on a side.
fn flat(edge: u32, colour: [u8; 4]) -> SurfaceSource {
    SurfaceSource::Snapshot(Arc::new(bitmap(edge, edge, |_, _| colour)))
}

/// A layer drawing the whole of `source` (`edge` pixels on a side) at `dst`.
fn drawing(source: SurfaceSource, edge: u32, dst: Rect) -> Layer {
    Layer::Surface(Surface {
        source,
        src: RectF::new(0.0, 0.0, f64::from(edge), f64::from(edge)),
        dst,
        transform: Transform::Normal,
        opacity: 1.0,
        tint: Color::WHITE,
        corner_radius: 0.0,
        silhouette: None,
        effects: Effects::new(),
        model: None,
        secure: false,
        content: None,
    })
}

/// A frame of `layers` over black, damaged whole.
fn frame_of(layers: Vec<Layer>) -> OutputFrame {
    let mut frame = frame(Color::BLACK, SIZE);
    for layer in layers {
        frame.push_layer(layer);
    }
    frame
}

/// Ten windows, each a flat 8 by 8 snapshot of its own surface, side by side.
fn ten_windows(commit: u64) -> OutputFrame {
    let layers = (0..10)
        .map(|index| {
            let layer = drawing(
                flat(8, [20 * index as u8, 100, 200, 255]),
                8,
                Rect::new(index * 6, 0, 8, 8),
            );
            keyed(layer, u64::from(index as u32) + 1, commit, None)
        })
        .collect();
    frame_of(layers)
}

/// A timeline semaphore the test owns, which a submission waits on until it is signalled: a GPU that is slow.
struct SlowGpuGate {
    device: ash::Device,
    semaphore: vk::Semaphore,
    opened: Arc<AtomicBool>,
}

#[allow(unsafe_code)]
impl SlowGpuGate {
    /// Closes a gate in front of the next submission of `vulkan`.
    fn close(vulkan: &Vulkan) -> Self {
        let context = &vulkan.device.context;
        let mut kind = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
        // SAFETY: the device has timeline semaphores; the semaphore is destroyed by `Drop`, after it is signalled.
        let semaphore = unsafe { context.device.create_semaphore(&info, None) }.expect("a semaphore");
        context.faults.gate.set(Some((semaphore, 1)));
        Self {
            device: context.device.clone(),
            semaphore,
            opened: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Opens the gate now.
    fn open(&self) {
        if !self.opened.swap(true, Ordering::AcqRel) {
            // SAFETY: the semaphore is a timeline at value 0, and 1 is the value the submission waits for.
            unsafe {
                self.device
                    .signal_semaphore(&vk::SemaphoreSignalInfo::default().semaphore(self.semaphore).value(1))
            }
            .expect("the gate opens");
        }
    }

    /// Opens the gate from another thread after `delay`.
    fn open_after(&self, delay: Duration) -> std::thread::JoinHandle<()> {
        let device = self.device.clone();
        let semaphore = self.semaphore;
        let opened = Arc::clone(&self.opened);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if !opened.swap(true, Ordering::AcqRel) {
                // SAFETY: as in `open`.
                unsafe { device.signal_semaphore(&vk::SemaphoreSignalInfo::default().semaphore(semaphore).value(1)) }
                    .expect("the gate opens");
            }
        })
    }
}

#[allow(unsafe_code)]
impl Drop for SlowGpuGate {
    fn drop(&mut self) {
        // A test that failed with the gate shut must not leave the device waiting on it.
        self.open();
        // SAFETY: the gate is open, so the device finishes what waited on it, and nothing else uses the semaphore.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_semaphore(self.semaphore, None);
        }
    }
}

#[test]
fn an_idle_frame_submits_nothing() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    vulkan.draw(&ten_windows(1), Scale::IDENTITY).expect("a first frame");
    let before = vulkan.stats();
    // No damage, no layers: the compositor drew nothing, and the renderer is not asked to.
    vulkan
        .draw(&OutputFrame::new(), Scale::IDENTITY)
        .expect("an empty frame");
    // Damage that is all off the output has nothing to repaint either.
    let mut off = OutputFrame::new();
    off.set_background(Color::BLACK);
    off.push_damage(Rect::new(1000, 1000, 8, 8));
    vulkan
        .draw(&off, Scale::IDENTITY)
        .expect("a frame with no damage on the output");
    assert_eq!(
        vulkan.stats(),
        before,
        "an idle frame submits, uploads and allocates nothing"
    );
    clean(&vulkan);
}

#[test]
fn a_steady_frame_uploads_nothing() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let frame = ten_windows(1);
    vulkan.draw(&frame, Scale::IDENTITY).expect("the first frame");
    let first = vulkan.stats();
    assert_eq!(
        first.bytes_uploaded,
        10 * 8 * 8 * 4,
        "each window's buffer is uploaded once"
    );
    assert_eq!(first.textures, 10);
    for _ in 0..5 {
        vulkan.draw(&frame, Scale::IDENTITY).expect("a steady frame");
    }
    let steady = vulkan.stats();
    assert_eq!(
        steady.bytes_uploaded, first.bytes_uploaded,
        "a steady frame uploads nothing"
    );
    assert_eq!(steady.uploads, first.uploads);
    assert_eq!(
        steady.device_allocations, first.device_allocations,
        "and allocates nothing"
    );
    assert_eq!(steady.submits, first.submits + 5, "each of the five drew");
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(image.colour_at(2, 2), Some([0, 100, 200, 255]), "the first window");
    assert_eq!(
        image.colour_at(6 * 9 + 2, 2),
        Some([180, 100, 200, 255]),
        "the tenth window"
    );
    clean(&vulkan);
}

#[test]
fn a_steady_session_allocates_no_device_memory() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let mut frame = ten_windows(1);
    // The warm-up: the staging and uniform buffers have grown to what these frames need, a capture has made its readback.
    for _ in 0..4 {
        vulkan.draw(&frame, Scale::IDENTITY).expect("a warm-up frame");
        vulkan.capture(true).expect("a warm-up capture");
    }
    let warm = vulkan.stats();
    for commit in 2..60_u64 {
        // A client commits now and then, with damage: that uploads a rectangle and allocates nothing.
        if commit % 7 == 0 {
            frame = frame_of(
                (0..10)
                    .map(|index| {
                        let layer = drawing(
                            flat(8, [20 * index as u8, 100, 200, 255]),
                            8,
                            Rect::new(index * 6, 0, 8, 8),
                        );
                        keyed(
                            layer,
                            u64::from(index as u32) + 1,
                            commit / 7 + 1,
                            Some(vec![Rect::new(0, 0, 8, 1)]),
                        )
                    })
                    .collect(),
            );
        }
        vulkan.draw(&frame, Scale::IDENTITY).expect("a frame");
        if commit % 5 == 0 {
            vulkan.capture(commit % 2 == 0).expect("a capture");
        }
    }
    let after = vulkan.stats();
    assert_eq!(
        after.device_allocations, warm.device_allocations,
        "a steady session asks the driver for no memory: {warm:?} then {after:?}"
    );
    assert_eq!(after.device_memory, warm.device_memory, "and holds no more of it");
    clean(&vulkan);
}

#[test]
fn damage_uploads_only_the_damaged_rows() {
    // Two buffers of one surface: what the client drew first, and what it drew after damaging rows 2 and 3.
    let client = ShmClient::new(vec![
        BufferSpec::argb(8, 8, |_, _| [255, 0, 0, 255]),
        BufferSpec::argb(8, 8, |_, _| [0, 255, 0, 255]),
    ]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let dst = Rect::new(0, 0, 8, 8);
    let src = RectF::new(0.0, 0.0, 8.0, 8.0);
    vulkan
        .draw(
            &frame_of(vec![keyed(
                surface(&client, 0, src, dst, 1.0, Color::WHITE),
                7,
                1,
                None,
            )]),
            Scale::IDENTITY,
        )
        .expect("the first commit");
    assert_eq!(
        vulkan.stats().bytes_uploaded,
        8 * 8 * 4,
        "the first commit uploads the whole buffer"
    );
    let damaged = Rect::new(0, 2, 8, 2);
    vulkan
        .draw(
            &frame_of(vec![keyed(
                surface(&client, 1, src, dst, 1.0, Color::WHITE),
                7,
                2,
                Some(vec![damaged]),
            )]),
            Scale::IDENTITY,
        )
        .expect("the second commit");
    let stats = vulkan.stats();
    assert_eq!(
        stats.bytes_uploaded,
        8 * 8 * 4 + 8 * 2 * 4,
        "the second uploads the two damaged rows and nothing else"
    );
    let image = vulkan.screenshot().expect("a readback");
    // The second buffer is green everywhere, but only its damaged rows were uploaded: the rest of the texture is the first
    // commit's. That is what proves the upload was partial, and that the rest of the picture was kept.
    assert_eq!(
        image.colour_at(3, 1),
        Some([255, 0, 0, 255]),
        "above the damage: the old picture"
    );
    assert_eq!(
        image.colour_at(3, 2),
        Some([0, 255, 0, 255]),
        "the first damaged row: the new picture"
    );
    assert_eq!(
        image.colour_at(3, 3),
        Some([0, 255, 0, 255]),
        "the second damaged row: the new picture"
    );
    assert_eq!(
        image.colour_at(3, 4),
        Some([255, 0, 0, 255]),
        "below the damage: the old picture"
    );
    clean(&vulkan);
}

#[test]
fn overlapping_damage_is_uploaded_once_where_it_overlaps() {
    let client = ShmClient::new(vec![
        BufferSpec::argb(16, 16, |_, _| [255, 0, 0, 255]),
        BufferSpec::argb(16, 16, |_, _| [0, 255, 0, 255]),
    ]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let dst = Rect::new(0, 0, 16, 16);
    let src = RectF::new(0.0, 0.0, 16.0, 16.0);
    vulkan
        .draw(
            &frame_of(vec![keyed(
                surface(&client, 0, src, dst, 1.0, Color::WHITE),
                3,
                1,
                None,
            )]),
            Scale::IDENTITY,
        )
        .expect("the first commit");
    let before = vulkan.stats().bytes_uploaded;
    // Two rectangles that overlap in a 2 by 2 square, and one that reaches past the buffer.
    let damage = vec![Rect::new(0, 0, 4, 4), Rect::new(2, 2, 4, 4), Rect::new(14, 14, 10, 10)];
    vulkan
        .draw(
            &frame_of(vec![keyed(
                surface(&client, 1, src, dst, 1.0, Color::WHITE),
                3,
                2,
                Some(damage),
            )]),
            Scale::IDENTITY,
        )
        .expect("the second commit");
    assert_eq!(
        vulkan.stats().bytes_uploaded - before,
        (16 + 16 - 4 + 4) * 4,
        "the union of the damage, clipped to the buffer: 28 + 4 pixels"
    );
    let image = vulkan.screenshot().expect("a readback");
    for (point, what) in [
        ((1, 1), "the first rectangle"),
        ((5, 5), "the second"),
        ((3, 3), "where they overlap"),
        ((15, 15), "the part of the third inside the buffer"),
    ] {
        assert_eq!(image.colour_at(point.0, point.1), Some([0, 255, 0, 255]), "{what}");
    }
    assert_eq!(image.colour_at(8, 1), Some([255, 0, 0, 255]), "outside the damage");
    clean(&vulkan);
}

#[test]
fn a_commit_that_was_not_drawn_is_uploaded_whole() {
    let client = ShmClient::new(vec![
        BufferSpec::argb(8, 8, |_, _| [255, 0, 0, 255]),
        BufferSpec::argb(8, 8, |_, _| [0, 255, 0, 255]),
    ]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let dst = Rect::new(0, 0, 8, 8);
    let src = RectF::new(0.0, 0.0, 8.0, 8.0);
    vulkan
        .draw(
            &frame_of(vec![keyed(
                surface(&client, 0, src, dst, 1.0, Color::WHITE),
                9,
                1,
                None,
            )]),
            Scale::IDENTITY,
        )
        .expect("the first commit");
    let before = vulkan.stats().bytes_uploaded;
    // Commits 2 and 3 were never drawn (another output, a hidden window); the frame that draws commit 4 carries only the
    // damage of commit 4. The rows commit 2 and 3 changed are not in it.
    vulkan
        .draw(
            &frame_of(vec![keyed(
                surface(&client, 1, src, dst, 1.0, Color::WHITE),
                9,
                4,
                Some(vec![Rect::new(0, 0, 8, 1)]),
            )]),
            Scale::IDENTITY,
        )
        .expect("a later commit");
    assert_eq!(
        vulkan.stats().bytes_uploaded - before,
        8 * 8 * 4,
        "a gap in the commits is uploaded whole"
    );
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(3, 6),
        Some([0, 255, 0, 255]),
        "a row the damage did not name is the new picture too"
    );
    clean(&vulkan);
}

#[test]
fn a_new_size_makes_a_new_texture_and_the_old_one_goes() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let small = frame_of(vec![keyed(
        drawing(flat(8, [255, 0, 0, 255]), 8, Rect::new(0, 0, 8, 8)),
        4,
        1,
        None,
    )]);
    vulkan.draw(&small, Scale::IDENTITY).expect("the small buffer");
    let large = frame_of(vec![keyed(
        drawing(flat(16, [0, 255, 0, 255]), 16, Rect::new(0, 0, 16, 16)),
        4,
        2,
        Some(vec![Rect::new(0, 0, 1, 1)]),
    )]);
    vulkan.draw(&large, Scale::IDENTITY).expect("the large buffer");
    let stats = vulkan.stats();
    assert_eq!(stats.textures, 1, "one surface, one texture");
    assert_eq!(
        stats.bytes_uploaded,
        (8 * 8 + 16 * 16) * 4,
        "a new size is uploaded whole, whatever the damage says"
    );
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(
        image.colour_at(12, 12),
        Some([0, 255, 0, 255]),
        "the whole new buffer is there"
    );
    clean(&vulkan);
}

#[test]
fn a_destroyed_buffer_at_the_cached_commit_draws_from_its_texture() {
    let mut client = ShmClient::new(vec![BufferSpec::argb(8, 8, |_, _| [255, 0, 0, 255])]);
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let layer = |client: &ShmClient| {
        keyed(
            surface(
                client,
                0,
                RectF::new(0.0, 0.0, 8.0, 8.0),
                Rect::new(0, 0, 8, 8),
                1.0,
                Color::WHITE,
            ),
            2,
            1,
            None,
        )
    };
    let frame = frame_of(vec![layer(&client)]);
    vulkan.draw(&frame, Scale::IDENTITY).expect("the first frame");
    client.destroy(0);
    let before = vulkan.stats().bytes_uploaded;
    vulkan
        .draw(&frame, Scale::IDENTITY)
        .expect("the frame with the buffer destroyed");
    assert_eq!(
        vulkan.stats().bytes_uploaded,
        before,
        "the texture is current, so the buffer is not read"
    );
    assert_eq!(
        vulkan.screenshot().expect("a readback").colour_at(2, 2),
        Some([255, 0, 0, 255])
    );
    clean(&vulkan);
}

#[test]
fn a_frame_that_failed_does_not_leave_the_cache_believing_it_uploaded() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let first = frame_of(vec![keyed(
        drawing(flat(8, [255, 0, 0, 255]), 8, Rect::new(0, 0, 8, 8)),
        6,
        1,
        None,
    )]);
    vulkan.draw(&first, Scale::IDENTITY).expect("the first commit");
    // The second commit's frame is refused by the driver before it is submitted.
    let second = frame_of(vec![keyed(
        drawing(flat(8, [0, 255, 0, 255]), 8, Rect::new(0, 0, 8, 8)),
        6,
        2,
        Some(vec![Rect::new(0, 0, 8, 8)]),
    )]);
    vulkan
        .device
        .context
        .faults
        .submit
        .set(Some(vk::Result::ERROR_OUT_OF_HOST_MEMORY));
    assert_eq!(vulkan.draw(&second, Scale::IDENTITY), Err(RenderError::Failed));
    assert!(!vulkan.is_lost(), "running out of host memory is not a lost device");
    // The compositor draws the same commit again. The cache must not think it already has it.
    vulkan.draw(&second, Scale::IDENTITY).expect("the second commit again");
    assert_eq!(
        vulkan.screenshot().expect("a readback").colour_at(3, 3),
        Some([0, 255, 0, 255]),
        "the picture is the second commit's, not the first's the failed frame left behind"
    );
    clean(&vulkan);
}

#[test]
fn a_surface_not_drawn_for_a_while_gives_its_texture_back() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    vulkan.draw(&ten_windows(1), Scale::IDENTITY).expect("ten windows");
    assert_eq!(vulkan.stats().textures, 10);
    // The windows are gone: later frames draw something else.
    let other = frame_of(vec![Layer::Solid(Solid::new(
        Rect::new(0, 0, 4, 4),
        colour(9, 9, 9, 255),
    ))]);
    for _ in 0..IDLE_FRAMES + 2 {
        vulkan.draw(&other, Scale::IDENTITY).expect("a frame");
    }
    let stats = vulkan.stats();
    assert_eq!(
        stats.textures, 0,
        "a window nobody has drawn for {IDLE_FRAMES} frames has no texture"
    );
    vulkan
        .capture(false)
        .expect("a capture, which waits for the frames before it");
    assert_eq!(
        vulkan.stats().retiring,
        0,
        "and the textures are destroyed once the frames that drew them have finished"
    );
    assert!(vulkan.stats().images_destroyed >= 10);
    assert_eq!(vulkan.gpu().textures.borrow().len(), 0);
    assert_eq!(
        vulkan.gpu().pipelines.long_sets.in_use(),
        0,
        "the descriptor sets of the textures are given back"
    );
    clean(&vulkan);
}

#[test]
fn two_outputs_share_one_texture_for_one_surface() {
    let Some(device) = device() else { return };
    let mut left = output(&device);
    let mut right = output(&device);
    // One window that spans both outputs: the same surface, the same commit, drawn on each.
    let frame = frame_of(vec![keyed(
        drawing(flat(8, [10, 20, 30, 255]), 8, Rect::new(0, 0, 8, 8)),
        5,
        1,
        None,
    )]);
    left.draw(&frame, Scale::IDENTITY).expect("the left output");
    right.draw(&frame, Scale::IDENTITY).expect("the right output");
    let stats = device.stats();
    assert_eq!(
        stats.bytes_uploaded,
        8 * 8 * 4,
        "the window is uploaded once for the two outputs"
    );
    assert_eq!(stats.textures, 1);
    for (name, vulkan) in [("left", &mut left), ("right", &mut right)] {
        assert_eq!(
            vulkan.screenshot().expect("a readback").colour_at(2, 2),
            Some([10, 20, 30, 255]),
            "the {name} output shows it"
        );
    }
    clean(&left);
    // Dropping an output does not take the device's textures with it.
    drop(left);
    drop(right);
    assert_eq!(device.stats().textures, 1);
}

#[test]
fn a_lost_device_is_lost_for_every_output_on_it() {
    let Some(device) = device() else { return };
    let mut left = output(&device);
    let mut right = output(&device);
    let frame = frame_of(vec![Layer::Solid(Solid::new(
        Rect::new(0, 0, 4, 4),
        colour(255, 0, 0, 255),
    ))]);
    left.draw(&frame, Scale::IDENTITY).expect("a frame");
    device.context.faults.submit.set(Some(vk::Result::ERROR_DEVICE_LOST));
    assert_eq!(left.draw(&frame, Scale::IDENTITY), Err(RenderError::Lost));
    assert!(device.is_lost());
    assert_eq!(
        right.draw(&frame, Scale::IDENTITY),
        Err(RenderError::Lost),
        "the other output is told at once"
    );
    assert_eq!(right.capture(false).err(), Some(RenderError::Lost));
}

#[test]
fn a_frame_returns_before_the_gpu_finishes() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let frame = frame_of(vec![Layer::Solid(Solid::new(
        Rect::new(0, 0, 8, 8),
        colour(255, 0, 0, 255),
    ))]);
    vulkan.draw(&frame, Scale::IDENTITY).expect("a warm-up frame");
    vulkan.capture(false).expect("everything so far has finished");
    assert_eq!(vulkan.stats().frames_in_flight, 0);
    // The GPU is held back: the next frame is submitted and cannot start.
    let gate = SlowGpuGate::close(&vulkan);
    let started = Instant::now();
    vulkan
        .draw(&frame, Scale::IDENTITY)
        .expect("a frame the GPU has not started");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "draw returned although the GPU is held back, which it could not if it waited for it"
    );
    assert_eq!(
        vulkan.stats().frames_in_flight,
        1,
        "the frame is on the queue and unfinished"
    );
    // A second frame goes into the other slot, and two is the most an output has in flight.
    vulkan.draw(&frame, Scale::IDENTITY).expect("a second frame behind it");
    assert_eq!(vulkan.stats().frames_in_flight, 2);
    // A third has no slot until the first is done: it waits for the gate, and returns when it opens.
    let opening = gate.open_after(Duration::from_millis(300));
    let waited = Instant::now();
    vulkan.draw(&frame, Scale::IDENTITY).expect("a third frame");
    assert!(
        waited.elapsed() >= Duration::from_millis(200),
        "a third frame waits for a slot: it took {:?}",
        waited.elapsed()
    );
    opening.join().expect("the gate thread");
    // A capture waits for what it reads, and what it reads is right.
    assert_eq!(
        vulkan.capture(false).expect("a capture").colour_at(2, 2),
        Some([255, 0, 0, 255])
    );
    assert_eq!(vulkan.stats().frames_in_flight, 0);
    clean(&vulkan);
}

#[test]
fn a_capture_waits_for_the_gpu() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let frame = frame_of(vec![Layer::Solid(Solid::new(
        Rect::new(0, 0, 8, 8),
        colour(0, 0, 255, 255),
    ))]);
    vulkan.draw(&frame, Scale::IDENTITY).expect("a warm-up frame");
    vulkan.capture(false).expect("everything so far has finished");
    let gate = SlowGpuGate::close(&vulkan);
    vulkan
        .draw(&frame, Scale::IDENTITY)
        .expect("a frame the GPU has not started");
    let opening = gate.open_after(Duration::from_millis(300));
    let started = Instant::now();
    let image = vulkan.capture(false).expect("a capture");
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "the capture read the frame, which could not have finished before the gate opened"
    );
    assert_eq!(image.colour_at(2, 2), Some([0, 0, 255, 255]));
    opening.join().expect("the gate thread");
    clean(&vulkan);
}

#[test]
fn a_texture_is_destroyed_only_after_its_last_frame_retires() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    // Frame 1 draws the surface at 8 by 8, with the GPU held back.
    let small = frame_of(vec![keyed(
        drawing(flat(8, [255, 0, 0, 255]), 8, Rect::new(0, 0, 8, 8)),
        3,
        1,
        None,
    )]);
    vulkan.draw(&small, Scale::IDENTITY).expect("warm-up");
    vulkan.capture(false).expect("everything so far has finished");
    let gate = SlowGpuGate::close(&vulkan);
    vulkan
        .draw(&small, Scale::IDENTITY)
        .expect("the frame that uses the texture");
    let destroyed = vulkan.stats().images_destroyed;
    // Frame 2 draws the surface at another size: the cache replaces the texture. The old one is still read by frame 1.
    let large = frame_of(vec![keyed(
        drawing(flat(16, [0, 255, 0, 255]), 16, Rect::new(0, 0, 16, 16)),
        3,
        2,
        None,
    )]);
    vulkan
        .draw(&large, Scale::IDENTITY)
        .expect("the frame that replaces it");
    let held = vulkan.stats();
    assert_eq!(
        held.images_destroyed, destroyed,
        "the old texture is kept while the frame that reads it is unfinished"
    );
    assert!(held.retiring >= 1, "and waits in the retire queue: {held:?}");
    gate.open();
    // The capture waits for both frames and collects what they held.
    vulkan.capture(false).expect("a capture");
    let after = vulkan.stats();
    assert!(
        after.images_destroyed > destroyed,
        "the old texture is destroyed once its frame has retired: {after:?}"
    );
    assert_eq!(after.retiring, 0, "and nothing else waits: {after:?}");
    clean(&vulkan);
}

#[test]
#[allow(unsafe_code)]
fn synchronization_validation_reports_a_race() {
    use crate::sync::InFlightFrame;
    let Some(vulkan) = renderer(SIZE) else { return };
    if !vulkan.validation_enabled() {
        println!("skipping: the validation layer is not installed or this is not a debug build");
        return;
    }
    let context = &vulkan.device.context;
    let buffer =
        crate::memory::VkBuffer::new(context, 256, vk::BufferUsageFlags::TRANSFER_DST, false).expect("a buffer");
    let mut slot = InFlightFrame::new(context).expect("a slot");
    slot.begin().expect("a recording");
    // Two writes to one range with nothing between them: a write-after-write hazard.
    // (the misuse follows)
    // SAFETY: the deliberate misuse; the buffer is a transfer destination, the range is inside it, and the buffer is not
    // submitted, so nothing runs.
    unsafe {
        context.device.cmd_fill_buffer(slot.cmd, buffer.raw(), 0, 64, 1);
        context.device.cmd_fill_buffer(slot.cmd, buffer.raw(), 0, 64, 2);
    }
    let messages = vulkan.take_validation_messages();
    assert!(
        // Older layers wrote "HAZARD", newer ones "WRITE_AFTER_WRITE hazard detected".
        messages
            .iter()
            .any(|message| message.to_ascii_lowercase().contains("hazard")),
        "synchronisation validation must report the race, or no test here can fail on one: {messages:#?}"
    );
}

#[test]
fn a_corrupt_pipeline_cache_is_ignored_and_replaced() {
    let directory = tempfile::tempdir().expect("a directory");
    let options = VulkanOptions {
        pipeline_cache: PipelineCacheLocation::Directory(directory.path().to_owned()),
        ..options()
    };
    let make = || {
        VulkanDevice::with_options(&options)
            .map(Some)
            .or_else(|error| match error {
                VulkanError::NoLoader { .. } | VulkanError::NoDevice { .. } => {
                    println!("skipping: {error}; there is no Vulkan device on this machine");
                    Ok(None)
                }
                other => Err(other),
            })
    };
    let Some(first) = make().expect("a device") else { return };
    assert!(!first.stats().pipeline_cache_loaded, "no file yet");
    let file = options
        .pipeline_cache
        .file(&first.context.cache_identity)
        .expect("a file for the device");
    drop(first);
    let written = std::fs::read(&file).expect("the device wrote its cache when it was dropped");
    assert_eq!(&written[..4], b"GNVC", "the file starts with our header");
    // A second run reads it back.
    let second = make().expect("a device").expect("a device");
    assert!(second.stats().pipeline_cache_loaded, "an intact file is used");
    drop(second);

    // Each way a file can be wrong: it is ignored, deleted, and replaced by a good one when the device is dropped.
    let mut flipped = written.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0xFF;
    // A file whose header says another format version.
    let with_header = |file: &[u8], version: u32| {
        let mut file = file.to_vec();
        file[4..8].copy_from_slice(&version.to_le_bytes());
        file
    };
    for (what, corrupt) in [
        ("garbage", b"this is not a pipeline cache".to_vec()),
        ("empty", Vec::new()),
        ("truncated", written[..written.len() / 2].to_vec()),
        ("a flipped byte", flipped),
        ("a file with a header of another version", with_header(&written, 9)),
    ] {
        std::fs::write(&file, &corrupt).expect("a corrupt file");
        let device = make().expect("a device").expect("a device");
        assert!(
            !device.stats().pipeline_cache_loaded,
            "{what}: a file that fails a check is not used"
        );
        assert!(
            !file.exists(),
            "{what}: and is deleted at once, so nothing reads it again"
        );
        // The device still draws.
        let mut vulkan = Vulkan::with_device(&device, SIZE).expect("an output");
        vulkan.draw(&ten_windows(1), Scale::IDENTITY).expect("a frame");
        drop(vulkan);
        drop(device);
        let replaced = std::fs::read(&file).expect("the cache is written again");
        assert_eq!(&replaced[..4], b"GNVC", "{what}: replaced by a good file");
        let again = make().expect("a device").expect("a device");
        assert!(again.stats().pipeline_cache_loaded, "{what}: and the new file is used");
    }
}

#[test]
fn ten_shell_reloads_leave_the_footprint_and_allocations_where_they_started() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let windows = ten_windows(1);
    // A reload takes the shell's nodes away and puts them back: its solid layers, its overlay and its cursor come and go
    // while the clients' windows stay.
    let shell = |present: bool| {
        let mut frame = ten_windows(1);
        if present {
            frame.push_layer(Layer::Solid(Solid::new(
                Rect::new(0, 40, 64, 8),
                colour(30, 30, 30, 255),
            )));
            let image = bitmap(32, 32, |_, _| [200, 200, 200, 255]);
            frame.push_layer(Layer::Overlay(
                Overlay::new(Rect::new(20, 20, 32, 32), &image).expect("an overlay"),
            ));
        }
        frame
    };
    for _ in 0..3 {
        vulkan
            .draw(&shell(true), Scale::IDENTITY)
            .expect("a frame with the shell");
        vulkan.draw(&shell(false), Scale::IDENTITY).expect("a frame without it");
    }
    vulkan.draw(&windows, Scale::IDENTITY).expect("settled");
    vulkan.capture(false).expect("a capture");
    let (footprint, stats) = (vulkan.footprint(), vulkan.stats());
    for reload in 0..10 {
        vulkan.draw(&shell(false), Scale::IDENTITY).expect("the shell is gone");
        vulkan.draw(&shell(true), Scale::IDENTITY).expect("the shell is back");
        vulkan.capture(false).expect("a capture");
        let now = vulkan.stats();
        assert_eq!(
            now.device_allocations, stats.device_allocations,
            "reload {reload}: no memory is asked of the driver"
        );
        assert_eq!(
            now.textures, stats.textures,
            "reload {reload}: the clients' textures stay"
        );
    }
    vulkan.draw(&windows, Scale::IDENTITY).expect("settled again");
    vulkan.capture(false).expect("a capture");
    assert_eq!(vulkan.footprint(), footprint, "the footprint is where it started");
    assert_eq!(
        vulkan.stats().allocated_bytes,
        stats.allocated_bytes,
        "and so is what the allocator has handed out"
    );
    clean(&vulkan);
}

#[test]
fn damage_rectangles_are_disjoint() {
    // A union of rectangles, checked against the pixels they cover.
    let cases: Vec<Vec<Rect>> = vec![
        vec![],
        vec![Rect::new(0, 0, 4, 4)],
        vec![Rect::new(0, 0, 4, 4), Rect::new(2, 2, 4, 4)],
        vec![Rect::new(0, 0, 8, 8), Rect::new(2, 2, 2, 2)],
        vec![Rect::new(2, 2, 2, 2), Rect::new(0, 0, 8, 8)],
        vec![
            Rect::new(0, 0, 4, 1),
            Rect::new(0, 0, 1, 4),
            Rect::new(3, 0, 1, 4),
            Rect::new(0, 3, 4, 1),
        ],
        vec![Rect::new(5, 5, 20, 20), Rect::new(-3, -3, 5, 5)],
        vec![Rect::new(20, 20, 3, 3)],
        vec![Rect::new(1, 1, 3, 3), Rect::new(1, 1, 3, 3)],
    ];
    for case in cases {
        let (width, height) = (10_u32, 9_u32);
        let covered = |rects: &[Rect]| -> Vec<bool> {
            (0..height as i32)
                .flat_map(|y| (0..width as i32).map(move |x| (x, y)))
                .map(|(x, y)| rects.iter().any(|rect| rect.contains(gnoblin_types::Point::new(x, y))))
                .collect()
        };
        let pieces = disjoint(&case, width, height);
        assert_eq!(
            covered(&pieces),
            covered(&case),
            "{case:?}: the pieces cover the union, clipped to the buffer"
        );
        let area: u64 = pieces
            .iter()
            .map(|rect| u64::from(rect.width) * u64::from(rect.height))
            .sum();
        let union = covered(&pieces).into_iter().filter(|held| *held).count() as u64;
        assert_eq!(area, union, "{case:?}: no pixel is covered twice");
        for piece in &pieces {
            assert!(piece.width > 0 && piece.height > 0, "{case:?}: no empty piece");
        }
    }
}

#[test]
fn a_cache_file_is_checked_against_the_device_that_reads_it() {
    use crate::context::CacheIdentity;
    use crate::pipeline::{decode_cache, encode_cache};
    let identity = CacheIdentity {
        vendor: 0x10de,
        device: 7,
        uuid: [3; vk::UUID_SIZE],
    };
    // The driver's blob starts with its own header: size, version one, vendor, device, UUID.
    let blob_for = |identity: &CacheIdentity| {
        let mut blob = vec![0_u8; 48];
        blob[0..4].copy_from_slice(&32_u32.to_le_bytes());
        blob[4..8].copy_from_slice(&1_u32.to_le_bytes());
        blob[8..12].copy_from_slice(&identity.vendor.to_le_bytes());
        blob[12..16].copy_from_slice(&identity.device.to_le_bytes());
        blob[16..32].copy_from_slice(&identity.uuid);
        blob
    };
    let blob = blob_for(&identity);
    let file = encode_cache(&blob);
    assert_eq!(
        decode_cache(&file, &identity),
        Ok(&blob[..]),
        "a good file is read back whole"
    );
    for (what, other) in [
        ("vendor", CacheIdentity { vendor: 1, ..identity }),
        ("device", CacheIdentity { device: 8, ..identity }),
        (
            "driver",
            CacheIdentity {
                uuid: [4; vk::UUID_SIZE],
                ..identity
            },
        ),
    ] {
        assert!(
            decode_cache(&file, &other).is_err(),
            "a file of another {what} is refused"
        );
    }
    let mut flipped = file.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 1;
    assert_eq!(decode_cache(&flipped, &identity), Err("its checksum does not match"));
    assert!(
        decode_cache(&file[..10], &identity).is_err(),
        "a file shorter than its header"
    );
    assert!(
        decode_cache(&file[..file.len() - 1], &identity).is_err(),
        "a file that lost its last byte"
    );
}

#[test]
fn releasing_the_textures_gives_them_back_and_the_next_frame_uploads_them_again() {
    let Some(mut vulkan) = renderer(SIZE) else { return };
    let frame = ten_windows(1);
    vulkan.draw(&frame, Scale::IDENTITY).expect("ten windows");
    assert_eq!(
        vulkan.footprint().scratch,
        10 * 8 * 8 * 4,
        "the cached textures are in the footprint"
    );
    vulkan.gpu().release_textures();
    vulkan
        .capture(false)
        .expect("a capture, after which the textures are destroyed");
    assert_eq!(vulkan.stats().textures, 0);
    assert_eq!(vulkan.footprint().scratch, 0);
    assert_eq!(vulkan.gpu().pipelines.long_sets.in_use(), 0);
    let before = vulkan.stats().bytes_uploaded;
    vulkan.draw(&frame, Scale::IDENTITY).expect("the windows again");
    assert_eq!(
        vulkan.stats().bytes_uploaded - before,
        10 * 8 * 8 * 4,
        "the next frame uploads what was given back"
    );
    assert_eq!(
        vulkan.screenshot().expect("a readback").colour_at(2, 2),
        Some([0, 100, 200, 255])
    );
    clean(&vulkan);
}
