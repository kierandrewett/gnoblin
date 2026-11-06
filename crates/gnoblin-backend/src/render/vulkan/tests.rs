//! Tests of the Vulkan renderer's creation.
//!
//! A test that needs a Vulkan device and cannot create a renderer because there is no loader or no
//! device prints why and returns, so the workspace's tests pass on a machine without Vulkan. Any
//! other failure to create one is a test failure. The instances the lane is gated on have no GPU
//! and use the software driver `lavapipe`.

use ash::vk;
use gnoblin_types::Size;

use super::pipeline::ShaderSources;
use super::*;

/// A renderer, or [`None`] with a printed reason when this machine has no Vulkan.
fn renderer(size: Size) -> Option<Vulkan> {
    match Vulkan::new(size) {
        Ok(renderer) => {
            println!(
                "Vulkan device: {} ({:?}), validation layer {}",
                renderer.device().name,
                renderer.device().kind,
                if renderer.validation_enabled() { "on" } else { "off" }
            );
            Some(renderer)
        }
        Err(error @ (VulkanError::NoLoader { .. } | VulkanError::NoDevice { .. })) => {
            println!("skipping: {error}; there is no Vulkan device on this machine");
            None
        }
        Err(error) => panic!("the Vulkan renderer could not be created: {error}"),
    }
}

const SIZE: Size = Size::new(64, 48);

#[test]
fn a_resize_replaces_the_target() {
    let Some(mut vulkan) = renderer(Size::new(8, 8)) else {
        return;
    };
    vulkan.resize(Size::new(32, 16)).expect("a bigger target");
    assert_eq!(vulkan.size(), Size::new(32, 16));
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(image.size(), Size::new(32, 16), "the readback follows the target");
    assert_eq!(
        image.colour_at(0, 0),
        Some([0, 0, 0, 0]),
        "a replaced target holds no pixels"
    );
    assert_eq!(
        vulkan.resize(Size::new(0, 4)),
        Err(RenderError::Degenerate { width: 0, height: 4 })
    );
    let messages = vulkan.take_validation_messages();
    assert!(messages.is_empty(), "the validation layer reported: {messages:#?}");
}

// Failure modes.

#[test]
fn a_new_target_reads_back_transparent_black() {
    let Some(mut vulkan) = renderer(Size::new(8, 4)) else {
        return;
    };
    let image = vulkan.screenshot().expect("a readback");
    assert_eq!(image.size(), Size::new(8, 4));
    for (x, y) in [(0, 0), (7, 3), (3, 2)] {
        assert_eq!(
            image.colour_at(x, y),
            Some([0, 0, 0, 0]),
            "({x}, {y}) holds no pixel yet"
        );
    }
    let messages = vulkan.take_validation_messages();
    assert!(messages.is_empty(), "the validation layer reported: {messages:#?}");
}

#[test]
fn a_missing_loader_is_a_named_error() {
    let options = VulkanOptions {
        loader: Loader::Library("/nonexistent/libvulkan-gnoblin-test.so".into()),
        ..VulkanOptions::default()
    };
    let error = Vulkan::with_options(SIZE, &options).expect_err("there is no such loader");
    assert!(matches!(error, VulkanError::NoLoader { .. }), "got {error}");
    // The caller falls back on the conversion, which says only that the renderer is not there.
    assert_eq!(error.into_render_error(), RenderError::Unavailable);
}

#[test]
fn an_unmatched_device_is_a_named_error() {
    let options = VulkanOptions {
        device: DeviceChoice::Named("no device has this name".to_owned()),
        ..VulkanOptions::default()
    };
    match Vulkan::with_options(SIZE, &options) {
        Err(VulkanError::NoDevice { reason }) => {
            println!("refused as expected: {reason}");
            assert!(!reason.is_empty(), "the reason names each refusal");
        }
        Err(VulkanError::NoLoader { .. }) => println!("skipping: there is no Vulkan loader on this machine"),
        Err(other) => panic!("expected no suitable device, got {other}"),
        Ok(_) => panic!("a device name nothing matches must not produce a renderer"),
    }
}

#[test]
fn a_broken_shader_is_a_startup_error() {
    let shared = ShaderSources::shared();
    let broken = ShaderSources {
        solid: "this is not WGSL",
        ..shared
    };
    // No device is asked: the passes are compiled first, so this runs on any machine.
    let error = Vulkan::with_sources(SIZE, &VulkanOptions::default(), &broken).expect_err("a broken pass");
    match error {
        VulkanError::Shader { pass, .. } => assert_eq!(pass, "solid"),
        other => panic!("expected a shader error, got {other}"),
    }
}

#[test]
#[allow(unsafe_code)]
fn validation_reports_a_driver_misuse() {
    let Some(vulkan) = renderer(SIZE) else { return };
    if !vulkan.validation_enabled() {
        println!("skipping: the validation layer is not installed or this is not a debug build");
        return;
    }
    // A buffer of no bytes is invalid usage, which the layer reports as an error.
    let info = vk::BufferCreateInfo::default()
        .size(0)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    // SAFETY: the call is the deliberate misuse; whatever the driver returns is destroyed.
    unsafe {
        if let Ok(buffer) = vulkan.context.device.create_buffer(&info, None) {
            vulkan.context.device.destroy_buffer(buffer, None);
        }
    }
    let messages = vulkan.take_validation_messages();
    assert!(
        !messages.is_empty(),
        "the layer must report a zero-sized buffer, or no test here can fail on validation"
    );
    println!("the layer said: {}", messages[0]);
}
