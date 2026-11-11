# xr-gpu

`xr-gpu` is the GPU side of `gnoblin-xr`: it copies a captured View (`wl_shm` pixels) into a swapchain image that the
OpenXR runtime shows.

## What is here

- `format` (pure): `choose_format` picks the swapchain format from the runtime's list, the first one it can fill of
  `R8G8B8A8_SRGB` (43) and `B8G8R8A8_SRGB` (50). `convert` writes `Argb8888` or `Xrgb8888` pixels as tightly packed
  pixels in that format: BGRA is a row copy, RGBA swaps red and blue, and Xrgb gets alpha 255.
- `Presenter`: `upload(target, pixels)` writes one picture into one acquired swapchain image and returns when the copy
  has finished. `FakePresenter` (feature `testing`) records each upload with its converted bytes.
- `VulkanDevice` and `VulkanPresenter` (`vulkan.rs`, the only `unsafe` in the crate): the instance and the device are
  created through `XR_KHR_vulkan_enable2` (`xrCreateVulkanInstanceKHR`, `xrCreateVulkanDeviceKHR`), so the runtime
  adds the extensions it needs. The Vulkan library is loaded by name with `ash`, so a machine without Vulkan fails
  with a clear error. An upload converts into a mapped staging buffer, then one submission transitions the image,
  copies the buffer into it and leaves it in `COLOR_ATTACHMENT_OPTIMAL`, as OpenXR expects at release.
  `read_back` copies an image out again, for tests.

## Test status

`format` and `FakePresenter` are unit tests and run anywhere (`cargo test -p xr-gpu`). The Vulkan path is exercised by
the ignored test `monado_vulkan_session_holds_the_uploaded_pixels_and_accepts_a_quad` in `xr-session`, against Monado
with lavapipe, and has no automatic run: see the extension README for how it was run.

```sh
cd extensions/gnoblin-xr
cargo test -p xr-gpu
```
