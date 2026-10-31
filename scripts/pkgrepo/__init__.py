"""Build, sign and publish the Gnoblin and Bingux package repositories.

One tool serves both projects. It takes a directory of built packages, merges them with the packages that are already
published, and writes a signed static repository tree for RPM, APT and pacman. design/package-repositories.md
describes the layout, the channels and the version rules.
"""

SCHEMA = 1
