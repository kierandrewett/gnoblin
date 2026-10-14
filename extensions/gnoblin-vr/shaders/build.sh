#!/usr/bin/env bash
# Regenerates the SPIR-V includes next to the sources. Needs glslc (glslc or shaderc package).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
for stage in vert frag; do
    glslc -mfmt=c -O "quad.$stage" -o "quad.$stage.inc"
done
