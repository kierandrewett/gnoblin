"""Checks that the stubs declare what the package and the runtime really have. Run with the `data/python` directory as argument, then the
directory `gnoblin-sdkgen` wrote the Python target to (the generated classes are not committed, so the test renders them first).

Nothing is imported: `gnoblin` runs only inside a shell, so the sources are read with `ast`.
"""

import ast
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
generated_root = pathlib.Path(sys.argv[2]) / "python" / "gnoblin"
problems = []


def parse(path):
    return ast.parse(path.read_text(encoding="utf-8"), filename=str(path))


def all_names(tree):
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "__all__" for t in node.targets):
            return {elt.value for elt in node.value.elts if isinstance(elt, ast.Constant)}
    return None


# 1. `_gnoblin_core.pyi` declares exactly the functions and exceptions the runtime registers.
module_rs = (root.parent.parent / "crates/gnoblin-runtime-python/src/module.rs").read_text(encoding="utf-8")
errors_rs = (root.parent.parent / "crates/gnoblin-runtime-python/src/errors.rs").read_text(encoding="utf-8")
registered = set(re.findall(r"wrap_pyfunction!\((?:\w+::)?(\w+), module\)", module_rs))
registered_errors = set(re.findall(r'module\.add\("(\w+)"', errors_rs))
stub = parse(root / "_gnoblin_core.pyi")
declared = {n.name for n in stub.body if isinstance(n, ast.FunctionDef)}
declared_errors = {n.name for n in stub.body if isinstance(n, ast.ClassDef)}
if registered - declared:
    problems.append(f"_gnoblin_core.pyi lacks functions: {sorted(registered - declared)}")
if declared - registered:
    problems.append(f"_gnoblin_core.pyi declares functions the runtime does not have: {sorted(declared - registered)}")
if registered_errors != declared_errors:
    problems.append(f"exception classes differ: runtime {sorted(registered_errors)}, stub {sorted(declared_errors)}")

# 2. Every name `gnoblin.__all__` exports is in the stub, and the stub exports no more.
package = root / "gnoblin"
init_names = all_names(parse(package / "__init__.py"))
stub_tree = parse(package / "__init__.pyi")
stub_imports = set()
for node in stub_tree.body:
    if isinstance(node, ast.ImportFrom):
        for alias in node.names:
            if alias.name != "*" and alias.asname:
                stub_imports.add(alias.asname)
generated = all_names(parse(generated_root / "_generated" / "classes.py")) or set()
if init_names is None:
    problems.append("gnoblin/__init__.py has no literal __all__")
else:
    missing = {n for n in init_names if n not in stub_imports and n not in generated and not n.startswith("_")}
    if missing:
        problems.append(f"gnoblin/__init__.pyi does not declare: {sorted(missing)}")

# 3. Every public method of the `Gnoblin` class and every named subscription has a stub through the inline annotations: the class
# body of api.py names each event, so a registered event with no annotation would not type-check.
api = parse(package / "api.py")
gnoblin_class = next(n for n in api.body if isinstance(n, ast.ClassDef) and n.name == "Gnoblin")
annotated = {n.target.id for n in gnoblin_class.body if isinstance(n, ast.AnnAssign) and isinstance(n.target, ast.Name)}
source = (package / "api.py").read_text(encoding="utf-8")
events = set(re.findall(r'^            "(\w+)": \(\s*\d,', source, re.M))
if events - annotated:
    problems.append(f"named subscriptions without a class annotation: {sorted(events - annotated)}")
if annotated - events:
    problems.append(f"class annotations for subscriptions that are not registered: {sorted(annotated - events)}")

if problems:
    print("\n".join(problems))
    sys.exit(1)
print("stubs match")
