#!/usr/bin/env python3
"""Exercise Gnoblin's generated shadow shader on an EGL GPU.

Only Cogl pipeline construction is stubbed to retrieve the actual GLSL.
Atomic counters measure executed layer work. Optional reference-source checks
compare pixels and GPU timing; this does not launch a compositor.
Requires moderngl and numpy, an EGL driver, a C compiler, and glib-2.0.
"""

import argparse
import json
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

import moderngl
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--source", type=Path, default=ROOT / "src/compositor/effects/meta-gnoblin-window-effects.c")
parser.add_argument("--reference-source", type=Path)
parser.add_argument("--report", type=Path)
parser.add_argument("--device-index", type=int, default=0, help="EGL device to use; inspect the reported renderer")
args = parser.parse_args()
STUBS = r"""#include <glib.h>
#include <stdio.h>
typedef int CoglPipeline;typedef int CoglTexture;typedef int ClutterOffscreenEffect;typedef int CoglSnippet;
static int pipeline,snippet;
static CoglPipeline* create(ClutterOffscreenEffect* e,CoglTexture*t){return &pipeline;}
static struct {CoglPipeline*(*create_pipeline)(ClutterOffscreenEffect*,CoglTexture*);} klass={create};
#define CLUTTER_OFFSCREEN_EFFECT_CLASS(x) (&klass)
#define COGL_SNIPPET_HOOK_FRAGMENT 0
#define META_GNOBLIN_WINDOW_SHADOW_MAX_LAYERS 4
static CoglSnippet* cogl_snippet_new(int hook,const char*d,const char*c){printf("%s\n---BODY---\n%s",d,c);return &snippet;}
static void cogl_pipeline_add_snippet(CoglPipeline*p,CoglSnippet*s){}
"""


def shader_source(source):
    text = Path(source).read_text()
    function = text[
        text.index("static CoglPipeline* window_shadow_create_pipeline") : text.index(
            "static void window_shadow_set_uniform_layers"
        )
    ]
    with tempfile.TemporaryDirectory(prefix="gnoblin-shadow-source-") as directory:
        path = Path(directory)
        (path / "extract.c").write_text(
            STUBS + function + "\nint main(void){" + "window_shadow_create_pipeline" + "(NULL,NULL);return 0;}\n"
        )
        flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "glib-2.0"], text=True).split()
        subprocess.run(["cc", str(path / "extract.c"), "-o", str(path / "extract"), *flags], check=True)
        return subprocess.check_output([str(path / "extract")], text=True)


ctx = moderngl.create_standalone_context(backend="egl", require=430, device_index=args.device_index)
vert = """#version 430
in vec2 position;out vec2 uv;void main(){uv=position*.5+.5;gl_Position=vec4(position,0.,1.);}
"""
vbo = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype="f4").tobytes())


def program(source, instrument=False):
    text = shader_source(source)
    declarations, body = text.split("---BODY---")
    if instrument:
        for prefix in ["a", "b"]:
            for i in range(4):
                body = body.replace(
                    f"vec4 g=gnoblin_shadow_geometry_{prefix}{i}",
                    f"atomicAdd(count,1u);vec4 g=gnoblin_shadow_geometry_{prefix}{i}",
                )
    scratch = "layout(std430,binding=0) buffer Work {uint count;};" if instrument else ""
    frag = (
        "#version 430\nin vec2 uv;out vec4 color;\n"
        + scratch
        + declarations
        + "\nvoid main(){vec4 cogl_color_in=vec4(1.0);vec4 cogl_color_out=vec4(0.2,0.3,0.4,0.5);vec2 cogl_tex_coord_in[1];cogl_tex_coord_in[0]=uv;"
        + body
        + "color=cogl_color_out;}"
    )
    p = ctx.program(vertex_shader=vert, fragment_shader=frag)
    vao = ctx.simple_vertex_array(p, vbo, "position")
    return p, vao


def uniforms(p, size, progress, exponent=2.5, radius=12):
    w, h = size
    values = {
        "gnoblin_shadow_texture_bounds": (0, 0, w, h),
        "gnoblin_shadow_bounds": (w * 0.1, h * 0.1, w * 0.9, h * 0.9),
        "gnoblin_shadow_radius": radius,
        "gnoblin_shadow_exponent": exponent,
        "gnoblin_shadow_progress": progress,
    }
    for prefix in ["a", "b"]:
        for i in range(4):
            values[f"gnoblin_shadow_geometry_{prefix}{i}"] = (i * 2, 8 + i * 5, 2 + i, 10 + i * 14)
            values[f"gnoblin_shadow_color_{prefix}{i}"] = (
                0.1 + i * 0.12,
                0.3,
                0.6 - i * 0.1,
                (0.25 + i * 0.07) if i < 3 else 0,
            )
            if prefix == "b":
                values[f"gnoblin_shadow_geometry_{prefix}{i}"] = (i * -3, 16 + i * 8, i, 6 + i * 20)
    for name, value in values.items():
        if name in p:
            p[name].value = value


def render(p, vao, size, progress, dtype="f4", exponent=2.5, radius=12):
    tex = ctx.texture(size, 4, dtype=dtype)
    fbo = ctx.framebuffer([tex])
    fbo.use()
    ctx.viewport = (0, 0, *size)
    uniforms(p, size, progress, exponent, radius)
    vao.render(moderngl.TRIANGLE_STRIP)
    ctx.finish()
    raw = fbo.read(components=4, dtype=dtype)
    fbo.release()
    tex.release()
    return raw


source = args.source
p, vao = program(source)
reference = args.reference_source
if reference:
    ref, refvao = program(reference)
    maxerror = 0
    cases = 0
    for exponent in [2.0, 2.5, 4.0, 6.0]:
        for radius in [0.0, 0.499, 0.5, 0.501, 12.0, 30.0, 60.0]:
            for progress in [-0.2, 0.0, 0.125, 0.5, 0.999, 1.0, 1.2]:
                a = np.frombuffer(
                    render(ref, refvao, (160, 120), progress, exponent=exponent, radius=radius), dtype="f4"
                )
                b = np.frombuffer(render(p, vao, (160, 120), progress, exponent=exponent, radius=radius), dtype="f4")
                err = float(np.max(np.abs(a - b)))
                assert err < 3e-6, (exponent, radius, progress, err)
                maxerror = max(maxerror, err)
                cases += 1
    print("pixel-equivalence", cases, "max_error", maxerror, flush=True)
workp, workvao = program(source, True)
counter = ctx.buffer(b"\0" * 4)
counter.bind_to_storage_buffer(0)
counts = {}
for progress in [0.0, 0.5, 1.0]:
    counter.write(b"\0" * 4)
    render(workp, workvao, (64, 64), progress)
    counts[str(progress)] = int.from_bytes(counter.read(), "little")
print("work-evaluations", counts, flush=True)
bench = {}
for label, (bp, bv) in ([("reference", (ref, refvao))] if reference else []) + [("current", (p, vao))]:
    tex = ctx.texture((3840, 2160), 4)
    fbo = ctx.framebuffer([tex])
    fbo.use()
    ctx.viewport = (0, 0, 3840, 2160)
    uniforms(bp, (3840, 2160), 1.0)
    for _ in range(3):
        bv.render(moderngl.TRIANGLE_STRIP)
    ctx.finish()
    values = []
    wall_values = []
    for _ in range(10):
        start = time.perf_counter()
        q = ctx.query(time=True)
        with q:
            bv.render(moderngl.TRIANGLE_STRIP)
        ctx.finish()
        wall_values.append((time.perf_counter() - start) * 1000)
        values.append(q.elapsed / 1e6)
    bench[label] = dict(
        median_ms=statistics.median(values), p95_ms=sorted(values)[9], wall_median_ms=statistics.median(wall_values)
    )
    fbo.release()
    tex.release()
report = dict(renderer=ctx.info["GL_RENDERER"], work=counts, benchmark=bench)
if reference:
    report.update(pixel_cases=cases, max_float_error=maxerror)
if args.report:
    args.report.write_text(json.dumps(report, indent=2))
print(json.dumps(report), flush=True)
assert all(count <= 64 * 64 * 3 for count in counts.values()), (
    "Unused shadow states or covered pixels evaluate shadow layers"
)
