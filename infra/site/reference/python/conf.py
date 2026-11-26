"""Sphinx configuration of the Python SDK reference.

The reference is written from the stub package an install gives an editor (`share/gnoblin/types/python`, written by
`cargo xtask docs-reference`). sphinx-autoapi reads the stubs statically, so nothing is imported and the native extension
is not needed. The descriptions are the schema files' own comments, which the generator writes as docstrings.
"""

import os
import re
from pathlib import Path

here = Path(__file__).resolve().parent
stubs = Path(os.environ.get("GNOBLIN_PYTHON_STUBS", here / "../../.generated/sdk/share/gnoblin/types/python")).resolve()

project = "Gnoblin Python SDK"
author = "Gnoblin contributors"
html_title = "Gnoblin Python SDK"

extensions = ["autoapi.extension"]

# A description is written with Markdown's backticks, so a backtick makes a code span.
default_role = "code"

autoapi_type = "python"
autoapi_dirs = [str(stubs)]
autoapi_root = "autoapi"
autoapi_keep_files = False
autoapi_add_toctree_entry = True
autoapi_file_patterns = ["*.pyi", "*.py"]
# `_gnoblin_core` is the native module's own stub: a shell never imports it.
autoapi_ignore = ["*_gnoblin_core*"]
autoapi_options = ["members", "undoc-members", "show-inheritance", "show-module-summary", "private-members"]
autoapi_python_class_content = "class"
autoapi_member_order = "alphabetical"
autoapi_own_page_level = "class"

exclude_patterns = ["_build", "requirements.*"]

# A type in an annotation is a short name (`Client`), and the SDK has a `Client` in more than one module (the object model and the
# first-generation tree). Sphinx picks the one of the module being documented, which is the one meant.
#
# - `ref.python`: that ambiguity;
# - `toc.duplicate_entry`: `gnoblin.std` names a class and its constructor function alike (`Layout`), which autoapi lists twice;
# - `autoapi.python_import_resolution`: a stub imports a name from a module that is a stub with no source beside it, which autoapi
#   cannot follow; the name is still documented where it is defined.
suppress_warnings = ["ref.python", "toc.duplicate_entry", "autoapi.python_import_resolution"]


# A role (:class:`Event`), an inline literal (``x``) or a Markdown code span (`x`), in that order of preference.
_SPAN = re.compile(r"(:[A-Za-z:]+:`[^`]*`)|(``.*?``)|`([^`\n]+)`", re.S)


def _markdown_code(text: str) -> str:
    """Make each Markdown code span of a description an inline literal.

    A description is written with backticks, which reStructuredText reads as an interpreted role, and a role ends at whitespace or
    punctuation: a letter next to the span (`Client`s) needs an escaped space.
    """

    def literal(match: "re.Match[str]") -> str:
        if match.group(3) is None:
            return match.group(0)
        before = "\\ " if match.start() and text[match.start() - 1].isalnum() else ""
        after = "\\ " if match.end() < len(text) and text[match.end()].isalnum() else ""
        return f"{before}``{match.group(3)}``{after}"

    return _SPAN.sub(literal, text)


def autoapi_prepare_jinja_env(env) -> None:
    """Every docstring reaches the templates through `indent`: it is where the descriptions are made reStructuredText."""
    indent = env.filters["indent"]
    env.filters["indent"] = lambda text, *args, **kwargs: indent(_markdown_code(text), *args, **kwargs)


html_theme = "furo"
html_static_path = []
html_theme_options = {
    "source_repository": "https://github.com/kierandrewett/gnoblin/",
    "source_branch": "main",
    # The reference is a part of the site, outside its pages: a way back to the other two languages and the docs.
    "announcement": (
        '<a href="/docs/reference">API reference</a>: '
        '<a href="/reference/typescript/">TypeScript</a> &middot; Python &middot; <a href="/reference/rust/">Rust</a>'
        ' &middot; <a href="/docs/python-shell">Python shell guide</a>'
    ),
}
html_show_sphinx = False
# The pages carry the reference text; a copy of the source of each is only weight.
html_copy_source = False
html_show_sourcelink = False
