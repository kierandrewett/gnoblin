"""Entry point for `python3 -m pkgrepo` and for `python3 scripts/pkgrepo`.

Running the directory executes this file as a top-level script. Some Python versions then give it no parent package,
so the relative import fails. The directory form puts this folder on sys.path, not its parent, so add the parent first.
"""

import sys
from pathlib import Path

if not __package__:
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    from pkgrepo.cli import main
else:
    from .cli import main

raise SystemExit(main())
