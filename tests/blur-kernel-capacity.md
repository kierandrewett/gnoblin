# Gaussian blur GLES uniform capacity

Run the real-Cogl probe against two `clutter-blur.c` files and an existing
configured, built Mutter tree:

```sh
python3 tests/blur-kernel-capacity.py \
  --mutter-build /path/to/gnoblin/build/make/mutter \
  --reference-source /path/to/original/clutter-blur.c \
  --source /path/to/candidate/clutter-blur.c \
  --artifacts /var/tmp/blur-capacity-check
```

The artifact directory must not exist. The runner copies Cogl and MTK libraries
there and uses the build's headers and compiler flags read-only. It needs a C
compiler, Python 3, and development files for GLib, EGL and GLESv2. Mesa's
surfaceless EGL software renderer runs with GLES 2.0 and two llvmpipe threads.
No compositor, VM or desktop session is started or contacted.

The C probe includes each implementation verbatim and calls its constructor,
uniform setters and both blur passes through real Cogl. Eleven persistent blur
instances alternate across three rounds, reversing order in the middle round.
Cases include both traced thin panels, odd dimensions, one-pixel axes, short
kernel uploads, both fast-path boundaries, original-recurrence fallbacks, and
downscaling. Inputs contain premultiplied alpha, edges and checker patterns.

The runner requires at most one RGBA8 code value of difference from the
original, byte-identical repeats for each instance, no GL errors, and one shared
GL program across instances. See `blur-kernel-gpu.py` for the larger pixel and
timing matrix; this check specifically exercises Cogl's real array uploads.

## Supported minimum and generated shader

The source audit uses the build pin in `gnome-versions.json`, Mutter
`138a14fbeef09d49ebf5be8a0cb83b042dd5c841`:

- `cogl/cogl/driver/gl/gles2/cogl-driver-gles2.c`,
  `check_gl_version()`, requires OpenGL ES 2.0 or better.
  Its initialization selects GLSL ES 1.00.
- `cogl/cogl/cogl-display-egl.c`, `try_create_context()`, requests EGL client
  version 2 for the GLES2 driver.
- `clutter/clutter/clutter-blur.c`, `create_blur_pipeline()`, starts a plain
  pipeline with one texture layer and the Gaussian lookup snippet.
- `cogl/cogl/cogl-pipeline-layer.c`, `_cogl_pipeline_init_default_layers()`,
  selects MODULATE with PREVIOUS and TEXTURE, without a constant operand.
  `cogl/cogl/cogl-pipeline.c` initializes the alpha test to ALWAYS.
- `cogl/cogl/driver/gl/cogl-pipeline-fragend-glsl.c` emits the layer sampler,
  a layer constant only for a CONSTANT operand, and an alpha-test reference
  only for a nontrivial alpha test. The latter two are absent for blur.

The generated fragment shader's active declarations have this conservative
budget, allowing an entire vec4 slot for every scalar, vector and sampler:

| Declaration               | Slots |
| ------------------------- | ----: |
| `vec4 kernel[10]`         |    10 |
| `vec3 kernel_scale`       |     1 |
| `float sigma`             |     1 |
| `float pixel_step`        |     1 |
| `vec2 direction`          |     1 |
| `sampler2D cogl_sampler0` |     1 |
| Total                     |    15 |

This includes the original recurrence's uniforms because its fallback remains
in the same shader. It assumes no sharing of scalar/vector slots. Cogl's common
boilerplate also declares three matrices in the fragment source, but the blur
fragment never accesses them. Matrices used by the vertex shader consume
vertex resources. The fragment receives color and texture coordinates through
varyings; later compositor saturation, noise and masking use other pipelines.

[OpenGL ES 2.0.25](https://registry.khronos.org/OpenGL/specs/es/2.0/es_full_spec_2.0.pdf)
defines active uniforms in section 2.10 (page 35), per-fragment uniform storage
in section 3.8.1 (page 86), and a minimum of 16 fragment uniform vectors in
table 6.20 (page 154).
[GLSL ES 1.00 revision 17](https://registry.khronos.org/OpenGL/specs/es/2.0/GLSL_ES_Specification_1.00.pdf),
Appendix A section 7, defines the required packing rules. Ten full rows plus
five individually reserved rows fit the minimum without relying on tighter
packing. No capacity fallback or candidate shader change is needed.

## What the probe proves

The probe saves Cogl's actual generated vertex and fragment sources. It links
the exact compiled fragment shader with a uniform-free vertex shader and
reflects that inspection program to count only fragment uniforms. The
inspection program is never bound for rendering. Unknown uniform types fail
the check rather than silently undercounting matrices or other new resources.

The verified candidate has six active fragment declarations using 15 slots;
the original has four declarations using four slots. All 33 RGBA8 comparisons
pass with maximum difference one, and all repeated-instance renders match
exactly. The tested llvmpipe driver advertises 4096 fragment uniform vectors.
Execution therefore proves GLES shader generation, linkage and uploads; the
16-vector compatibility conclusion comes from the source and specification
budget, not execution on a physical driver limited to 16 vectors.

The build pin's original blur file is byte-identical to the initially measured
gitlink `15761641006132ab027e86b997fa22be06e7e8e9`. Patch 0126 applies cleanly to
the build pin and produces the measured candidate bytes. The audited Cogl
generation files also match the build pin. Recheck this budget if the blur
pipeline gains layers, snippets, alpha tests or other fragment uniforms.
