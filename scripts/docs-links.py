"""Keep repository-relative source links useful in the published documentation."""

import re
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit


def on_page_markdown(markdown, page, config, files):
    docs = Path(config["docs_dir"]).resolve()
    root = docs.parent
    source = Path(page.file.abs_src_path).parent

    def replace(match):
        target = match.group(1)
        parsed = urlsplit(target)
        if parsed.scheme or target.startswith(("#", "/")):
            return match.group(0)
        resolved = (source / unquote(parsed.path)).resolve()
        if resolved.is_relative_to(docs) or not resolved.is_relative_to(root):
            return match.group(0)
        if not resolved.exists():
            raise ValueError(f"{page.file.src_uri}: missing repository link {target}")
        kind = "tree" if resolved.is_dir() else "blob"
        url = f"{config['repo_url']}/{kind}/main/{quote(resolved.relative_to(root).as_posix())}"
        if parsed.fragment:
            url += "#" + parsed.fragment
        return match.group(0).replace(target, url)

    return re.sub(r"\]\(([^\s)]+)\)", replace, markdown)
