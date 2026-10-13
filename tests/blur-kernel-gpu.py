#!/usr/bin/env python3
"""Compare real ClutterBlur GLSL and C uniform setup in standalone EGL.

Requires moderngl, numpy, glib-2.0 and a C compiler. Does not start or contact a
compositor. --source and --reference-source are clutter-blur.c files, including
their applied Gnoblin patches. Only the Cogl uniform sink is stubbed. Both blur
passes use linear sampling, clamp-to-edge and RGBA8 targets, as in ClutterBlur.
Select the software EGL device with --device-index for llvmpipe measurements
(device 1 on the investigation host), and set LP_NUM_THREADS=2. Check the
reported renderer. LIBGL_ALWAYS_SOFTWARE=1 additionally prevents a hardware run;
it requires selecting the software device, not the default hardware device.
CPU timing includes all process threads; GPU queries and wall time are separate.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import tempfile
import time

import moderngl
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
VERTEX = """#version 330
in vec2 position;
out vec2 uv;
void main() { uv = position * .5 + .5; gl_Position = vec4(position, 0., 1.); }
"""
EXTRACT_MAIN = r"""
int main (int argc, char **argv)
{
  if (argc == 1)
    {
      printf ("%s\n---BODY---\n%s", gaussian_blur_glsl_declarations,
              gaussian_blur_glsl);
      return 0;
    }
  g_assert (argc == 5);
  int width = atoi (argv[1]), height = atoi (argv[2]);
  float radius = strtof (argv[3], NULL);
  float scale = clutter_blur_calculate_downscale_factor (width, height, radius);
  CoglTexture texture = { floorf (width / scale), floorf (height / scale) };
  ClutterBlur blur = { .sigma = radius / 2.f, .downscale_factor = scale };
  BlurPass pass = { .texture = &texture, .orientation = atoi (argv[4]) };
  printf ("{\"size\":[%d,%d],\"scale\":%.9g,\"bypass\":%s}\n",
          texture.width, texture.height, scale,
          G_APPROX_VALUE (blur.sigma, 0.f, FLT_EPSILON) ? "true" : "false");
  if (!G_APPROX_VALUE (blur.sigma, 0.f, FLT_EPSILON))
    update_blur_uniforms (&blur, &pass);
  for (int i = 0; i < n_uniforms; i++)
    g_free (uniform_names[i]);
  return 0;
}
"""


def extract(source, directory):
    """Compile the actual strings, types, uniform setup and scale policy."""
    text = source.read_text()
    parts = [
        re.sub(r"^#include .*$", "", text[: text.index("static CoglPipeline*")], flags=re.MULTILINE),
        text[text.index("static void\nupdate_blur_uniforms") : text.index("static gboolean\ncreate_fbo")],
        text[text.index("float\nclutter_blur_calculate_downscale_factor") : text.index("static void\napply_blur_pass")],
    ]
    path = directory / "extract.c"
    path.write_text((ROOT / "tests/blur-kernel-extract.h").read_text() + "\n".join(parts) + EXTRACT_MAIN)
    executable = directory / "extract"
    flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "glib-2.0"], text=True).split()
    subprocess.run(
        [
            "cc",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-unused-parameter",
            str(path),
            "-o",
            str(executable),
            *flags,
            "-lm",
        ],
        check=True,
    )
    return executable


class Blur:
    def __init__(self, ctx, source, directory, vbo):
        self.ctx = ctx
        self.executable = extract(source, directory)
        declarations, body = subprocess.check_output([str(self.executable)], text=True).split("---BODY---")
        self.fragment = (
            "#version 330\nin vec2 uv; out vec4 color; uniform sampler2D cogl_sampler;\n"
            + declarations
            + "\nvoid main() { vec4 cogl_tex_coord = vec4(uv, 0., 1.); vec4 cogl_texel;\n"
            + body.replace("texture2D (", "texture (")
            + "\ncolor = cogl_texel; }"
        )
        (directory / "fragment.glsl").write_text(self.fragment)
        self.program = ctx.program(vertex_shader=VERTEX, fragment_shader=self.fragment)
        self.vao = ctx.simple_vertex_array(self.program, vbo, "position")
        self.uniform_cache = {}

    def configuration(self, size, radius, orientation):
        key = (*size, radius, orientation)
        if key not in self.uniform_cache:
            raw = subprocess.check_output([str(self.executable), *map(str, key)], text=True)
            self.uniform_cache[key] = [json.loads(line) for line in raw.splitlines()]
        return self.uniform_cache[key]

    def uniforms(self, config):
        for uniform in config[1:]:
            name = uniform["name"]
            if name not in self.program:
                continue
            values = uniform["value"]
            if uniform.get("integer"):
                self.program[name].value = values[0]
            else:
                # Cogl accepts the active prefix of an array; ModernGL requires
                # writing the whole declared array, so pad its unused tail.
                packed = np.asarray(values, dtype="f4").tobytes()
                slot = self.program[name]
                packed += bytes(slot.array_length * slot.dimension * 4 - len(packed))
                slot.write(packed)

    def prepare(self, pixels, radius, dtype="f1"):
        h, w, _ = pixels.shape
        config = [self.configuration((w, h), radius, i) for i in (0, 1)]
        texture = self.ctx.texture((w, h), 4, pixels.tobytes())
        texture.filter = (moderngl.LINEAR, moderngl.LINEAR)
        texture.repeat_x = texture.repeat_y = False
        size = tuple(config[0][0]["size"])
        targets, fbos = [], []
        for _ in range(2):
            target = self.ctx.texture(size, 4, dtype=dtype)
            target.filter = (moderngl.LINEAR, moderngl.LINEAR)
            target.repeat_x = target.repeat_y = False
            targets.append(target)
            fbos.append(self.ctx.framebuffer([target]))
        return texture, targets, fbos, config

    def draw(self, state):
        texture, targets, fbos, config = state
        if config[0][0]["bypass"]:
            return
        for i in (0, 1):
            fbos[i].use()
            self.ctx.viewport = (0, 0, *targets[i].size)
            fbos[i].clear()
            (texture if i == 0 else targets[0]).use(0)
            self.uniforms(config[i])
            self.vao.render(moderngl.TRIANGLE_STRIP)

    @staticmethod
    def release(state):
        texture, targets, fbos, _ = state
        for obj in [*fbos, *targets, texture]:
            obj.release()

    def pixels(self, pixels, radius, dtype="f1"):
        state = self.prepare(pixels, radius, dtype)
        try:
            if state[3][0][0]["bypass"]:
                return pixels.copy() if dtype == "f1" else pixels.astype("f4") / 255
            self.draw(state)
            w, h = state[1][1].size
            raw = state[2][1].read(components=4, dtype=dtype)
            return np.frombuffer(raw, dtype="u1" if dtype == "f1" else "f4").reshape(h, w, 4)
        finally:
            self.release(state)


def fixture(size, kind):
    """Premultiplied inputs: edges/corners, high frequency, alpha and noise."""
    w, h = size
    y, x = np.mgrid[:h, :w]
    rng = np.random.default_rng(501)
    out = np.zeros((h, w, 4), dtype="u1")
    if kind == "noise":
        out[:] = rng.integers(0, 256, out.shape, dtype="u1")
        out[:, :, :3] = (out[:, :, :3].astype("u2") * out[:, :, 3:4] // 255).astype("u1")
    elif kind == "edges":
        out[:, : max(1, w // 2)] = (32, 112, 255, 255)
        out[[0, -1], :] = (255, 255, 255, 255)
        out[:, [0, -1]] = (255, 0, 0, 255)
        out[h // 2, w // 2] = (0, 255, 0, 255)
    elif kind == "checker":
        out[:, :, :3] = (((x + y) % 2) * 255)[:, :, None]
        out[:, :, 3] = 255
    elif kind == "alpha":
        alpha = ((x * 255 // max(w - 1, 1)) * ((y % 7) != 0)).astype("u1")
        out[:, :, 0] = alpha
        out[:, :, 1] = alpha // 2
        out[:, :, 3] = alpha
    elif kind == "constant":
        out[:] = (13, 57, 91, 127)
    return out


def compare(current, reference, quick):
    sizes = [(1280, 82), (1280, 139), (1279, 81), (1, 17), (17, 1), (63, 37), (257, 259)]
    radii = [0, 1e-7, 0.5, 1, 1.9999999, 2, 4 / 3, 12, 24, 24.000011, 24.000017, 26.666666, 26.666668, 32, 64, 100, 128]
    if quick:
        sizes, radii = sizes[:3], [24, 24.000011, 24.000017]
    results = []
    for size in sizes:
        for radius in radii:
            for kind in ["noise", "edges", "checker", "alpha", "constant"]:
                pixels = fixture(size, kind)
                record = dict(size=size, radius=radius, kind=kind)
                config = current.configuration(size, radius, 0)
                reference_config = reference.configuration(size, radius, 0)
                assert config[0] == reference_config[0], (config[0], reference_config[0])
                record["configuration"] = config[0]
                record["kernel_scale"] = next((u["value"] for u in config[1:] if u["name"] == "kernel_scale"), None)
                for dtype in ["f1", "f4"]:
                    a = reference.pixels(pixels, radius, dtype)
                    b = current.pixels(pixels, radius, dtype)
                    assert np.isfinite(a).all() and np.isfinite(b).all(), record
                    delta = np.abs(a.astype("f8") - b.astype("f8"))
                    border = np.concatenate(
                        [delta[0].ravel(), delta[-1].ravel(), delta[:, 0].ravel(), delta[:, -1].ravel()]
                    )
                    record[dtype] = dict(
                        max=float(delta.max()),
                        mean=float(delta.mean()),
                        alpha_max=float(delta[:, :, 3].max()),
                        border_max=float(border.max()),
                        differing_channels=int(np.count_nonzero(delta)),
                    )
                    # Same full-resolution Gaussian, allowing float rounding and
                    # at most one RGBA8 code value after two quantized passes.
                    assert delta.max() <= (1 if dtype == "f1" else 3e-5), record
                    if dtype == "f1":
                        assert np.all(b[:, :, :3].astype(int) <= b[:, :, 3:4].astype(int) + 1), record
                        if kind == "constant":
                            assert np.max(np.abs(b.astype(int) - np.array([13, 57, 91, 127]))) <= 1, record
                results.append(record)
        print(f"pixel comparisons passed: {size}, {len(results)} cases", flush=True)
    return results


def benchmark(ctx, shaders, rounds, batch):
    output = []
    for size, radius in [
        ((1280, 82), 24.000011),
        ((1280, 139), 24.000017),
        ((1279, 81), 24),
        ((1920, 80), 24),
        ((640, 480), 24),
        ((1280, 82), 2),
        ((1280, 82), 32),
    ]:
        pixels = fixture(size, "noise")
        states = {name: shader.prepare(pixels, radius) for name, shader in shaders.items()}
        values = {name: dict(cpu_ms=[], wall_ms=[], gpu_ms=[]) for name in shaders}
        for name, shader in shaders.items():
            for _ in range(5):
                shader.draw(states[name])
            ctx.finish()
        for turn in range(rounds):
            order = list(shaders) if turn % 2 == 0 else list(reversed(shaders))
            for name in order:
                query = ctx.query(time=True)
                wall, cpu = time.perf_counter_ns(), time.process_time_ns()
                with query:
                    for _ in range(batch):
                        shaders[name].draw(states[name])
                ctx.finish()
                values[name]["cpu_ms"].append((time.process_time_ns() - cpu) / 1e6 / batch)
                values[name]["wall_ms"].append((time.perf_counter_ns() - wall) / 1e6 / batch)
                values[name]["gpu_ms"].append(query.elapsed / 1e6 / batch)
        result = dict(
            size=size,
            radius=radius,
            samples=values,
            median={
                name: {metric: statistics.median(samples) for metric, samples in measures.items()}
                for name, measures in values.items()
            },
        )
        output.append(result)
        print(json.dumps({k: v for k, v in result.items() if k != "samples"}), flush=True)
        for name in shaders:
            shaders[name].release(states[name])
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--reference-source", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--device-index", type=int, default=0)
    parser.add_argument("--quick", action="store_true")
    parser.add_argument("--benchmark-only", action="store_true")
    parser.add_argument("--rounds", type=int, default=11)
    parser.add_argument("--batch", type=int, default=8)
    args = parser.parse_args()
    ctx = moderngl.create_standalone_context(backend="egl", require=330, device_index=args.device_index)
    print(ctx.info["GL_RENDERER"], flush=True)
    vbo = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype="f4").tobytes())
    report = dict(
        renderer=ctx.info["GL_RENDERER"],
        version=ctx.info["GL_VERSION"],
        environment={key: os.getenv(key) for key in ["LIBGL_ALWAYS_SOFTWARE", "LP_NUM_THREADS", "GALLIUM_DRIVER"]},
        sources={},
    )
    with tempfile.TemporaryDirectory(prefix="gnoblin-blur-kernel-") as directory:
        shaders = {}
        for name, source in [("reference", args.reference_source), ("current", args.source)]:
            if source is None:
                continue
            target = Path(directory) / name
            target.mkdir()
            shaders[name] = Blur(ctx, source, target, vbo)
            report["sources"][name] = dict(
                path=str(source.resolve()),
                sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                fragment=shaders[name].fragment,
            )
        if args.reference_source and not args.benchmark_only:
            report["pixels"] = compare(shaders["current"], shaders["reference"], args.quick)
        report["benchmark"] = benchmark(ctx, shaders, args.rounds, args.batch)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(f"report: {args.report}", flush=True)


if __name__ == "__main__":
    main()
