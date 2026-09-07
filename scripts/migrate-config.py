#!/usr/bin/env python3
"""Convert supported Gnoblin INI settings to TOML without replacing either file."""
import argparse
import json
import os
from pathlib import Path
import re
import tomllib

FEATURES = {'osd', 'osd-volume', 'osd-microphone', 'osd-brightness',
            'osd-keyboard-brightness', 'osd-pad', 'screenshot', 'notifications',
            'window-switcher'}


def convert(text):
    sections = {'shell': {}, 'protocols': {}}
    ignored = set()
    section = ''
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith(('#', ';')):
            continue
        if line.startswith('['):
            section = line[1:line.index(']')].strip()
            continue
        if '=' not in line:
            continue
        key, value = (part.strip() for part in line.split('=', 1))
        if section not in sections or (section == 'shell' and key not in FEATURES | {'minimize-animation', 'minimize-duration'}):
            ignored.add(section or '(root)')
            continue
        if value.startswith(('"', "'")) and value[0] in value[1:]:
            value = value[1:value.index(value[0], 1)]
        else:
            value = re.split(r'\s+#', value, maxsplit=1)[0].strip()
        if section == 'protocols' or key in FEATURES:
            if value.lower() not in {'true', 'false', 'yes', 'no', 'on', 'off', '1', '0'}:
                raise ValueError(f'{section}.{key}: invalid boolean')
            value = value.lower() in {'true', 'yes', 'on', '1'}
        elif key == 'minimize-duration':
            value = int(value)
            if not 0 <= value <= 5000:
                raise ValueError('minimize-duration must be 0 to 5000')
        elif value not in {'zoom', 'fade', 'none', 'gnome'}:
            raise ValueError('invalid minimize-animation')
        sections[section][key] = value
    lines = ['# Migrated supported settings. The original .conf file is unchanged.']
    sections['shell'].setdefault('window-switcher', False)
    sections['shell'].setdefault('minimize-animation', 'zoom')
    sections['shell'].setdefault('minimize-duration', 200)
    for name, entries in sections.items():
        lines.extend(['', f'[{name}]'])
        lines.extend(f'{json.dumps(key)} = {json.dumps(value, ensure_ascii=False)}' for key, value in entries.items())
    result = '\n'.join(lines) + '\n'
    tomllib.loads(result)
    return result, ignored


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    default = Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config')) / 'gnoblin/gnoblin.conf'
    parser.add_argument('source', nargs='?', type=Path, default=default)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    result, ignored = convert(args.source.read_text())
    output = args.output or args.source.with_suffix('.toml')
    with output.open('x') as stream:
        stream.write(result)
    print(f'Created {output}; retained {args.source}')
    if ignored:
        print('Unsupported legacy sections were not activated: ' + ', '.join(sorted(ignored)))


if __name__ == '__main__':
    main()
