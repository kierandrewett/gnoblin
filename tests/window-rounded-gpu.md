# Rounded window shader GPU probe

Run the rounded shader probe with an EGL-capable Python environment that has
`moderngl` and `numpy`, plus a C compiler and `glib-2.0` development files:

```sh
python3 tests/window-shadow-gpu.py --kind rounded \
  --reference-source /path/to/previous-window-effects.c \
  --report /var/tmp/rounded-gpu.json
```

The probe extracts `rounded_clip_create_pipeline()` from each C source and
compiles its generated GLSL. A white 1×1 texture supplies the sampler. The
63 pixel comparisons cover three exponents, three radii, and seven border
widths. They check the shader math only; CSD reconstruction and native
compositor rendering are outside this fixture.

Atomic counters report executed `pow` calls at three border widths for both
sources. The zero-border candidate must use fewer calls than the original
three calls per pixel, and candidate counts at zero and full border width must
be lower than the reference. The JSON report includes SHA-256 hashes and paths
for both source files. Timing uses ten alternating reference/current samples
at 3840×2160.
The report separates GPU query medians and p95 from wall-clock medians; use the
wall median when comparing llvmpipe, where query timing can understate CPU cost.
The probe does not measure compositor frame pacing or prove a native benefit.
