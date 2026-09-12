#!/usr/bin/env python3
"""Black-box pixels for ext-background-effect state, regions and subsurfaces."""
import os
from pathlib import Path
import select
import subprocess
import sys
import time
from PIL import Image, ImageChops, ImageStat

assert os.environ.get('WAYLAND_DISPLAY', '').startswith('gnoblin-gs-')
output = Path('/tmp/gnoblin-standard-blur')
output.mkdir(exist_ok=True)
process = subprocess.Popen([sys.argv[1], 'pixels'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def reply(expected):
    assert select.select([process.stdout], [], [], 5)[0], 'Client response timed out'
    result = process.stdout.readline().strip()
    assert result == expected, (expected, result, process.poll())
def command(value):
    process.stdin.write(value + '\n'); process.stdin.flush(); reply('OK')
def capture(name):
    time.sleep(.4)
    path = output / (name + '.png')
    subprocess.run(['grim', str(path)], check=True)
    return Image.open(path).convert('RGB')
def same(a, b):
    assert max(ImageStat.Stat(ImageChops.difference(a, b)).mean) < .05, 'Uncommitted state changed pixels'
def blurred(image, box):
    deviation = max(ImageStat.Stat(image.crop(box)).stddev)
    assert deviation < 2, ('Expected blur', box, deviation)
def sharp(image, box):
    deviation = max(ImageStat.Stat(image.crop(box)).stddev)
    assert deviation > 80, ('Expected sharp backdrop', box, deviation)
try:
    reply('READY')
    initial = capture('initial')
    sharp(initial, (144, 144, 224, 224))
    command('set'); same(initial, capture('pending'))
    command('commit'); applied = capture('applied')
    blurred(applied, (192, 192, 212, 212))
    blurred(applied, (288, 160, 336, 208))
    sharp(applied, (164, 164, 180, 180))  # subtracted hole
    sharp(applied, (236, 160, 260, 208))  # gap between disjoint rectangles
    print('PASS: copied region, commit boundary, transparent pixels, holes and disjoint rectangles', flush=True)
    command('scale'); same(applied, capture('scaled'))
    print('PASS: buffer scale leaves surface-local coordinates unchanged', flush=True)
    command('empty'); same(applied, capture('empty-pending'))
    command('commit'); same(initial, capture('empty'))
    command('full'); command('commit'); full = capture('full')
    blurred(full, (144, 144, 432, 352)); sharp(full, (456, 144, 488, 208))
    command('clear'); same(full, capture('clear-pending'))
    command('commit'); same(initial, capture('cleared'))
    print('PASS: surface-size clipping, empty and NULL regions', flush=True)
    command('set'); command('commit'); capture('restored')
    command('destroy'); same(applied, capture('destroy-pending'))
    command('commit'); same(initial, capture('destroyed'))
    command('recreate'); command('full'); command('commit'); capture('recreated')
    command('clear'); command('commit'); capture('parent-clear')
    command('child'); same(initial, capture('child-pending'))
    command('commit'); child = capture('child-applied')
    blurred(child, (176, 272, 240, 320))
    sharp(child, (280, 272, 312, 320))
    print('PASS: destruction commits, recreation and synchronised subsurface state', flush=True)
    command('manager'); command('set'); command('commit')
    final = capture('manager-destroyed')
    blurred(final, (288, 160, 336, 208))
    print('PASS: manager destruction preserves existing surface objects', flush=True)
finally:
    process.terminate(); process.wait(timeout=5)
