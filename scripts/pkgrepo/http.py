"""One way to fetch a URL. Cloudflare rejects urllib's default User-Agent (HTTP 403, error 1010) on repo.gnoblin.org,
so every request names the tool instead."""

from __future__ import annotations

import urllib.request
from typing import IO

USER_AGENT = "gnoblin-pkgrepo (+https://github.com/kierandrewett/gnoblin)"


def open_url(url: str, timeout: int = 60) -> IO[bytes]:
    return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": USER_AGENT}), timeout=timeout)
