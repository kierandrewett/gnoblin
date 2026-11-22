"""Where the windows go: pure arithmetic, no compositor in it, so it can be tested and reloaded on its own."""

#: The space between windows and around them.
GAP = 10.0


def tiles(area, count):
    """The rectangles of `count` windows in `area` (x, y, width, height): the first is the master."""
    x, y, width, height = area
    inner = (x + GAP, y + GAP, width - 2 * GAP, height - 2 * GAP)
    if count == 0:
        return []
    if count == 1:
        return [inner]
    half = (inner[2] - GAP) / 2
    others = count - 1
    each = (inner[3] - GAP * (others - 1)) / others
    master = (inner[0], inner[1], half, inner[3])
    stack = [(inner[0] + half + GAP, inner[1] + index * (each + GAP), half, each) for index in range(others)]
    return [master, *stack]
