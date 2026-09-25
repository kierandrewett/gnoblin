#!/usr/bin/env python3
"""Build pinned GNOME libraries in a private prefix without a package manager."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def order_recipes(recipes):
    """Return manifest recipes in dependency order and reject an ambiguous closure."""
    by_name = {}
    for recipe in recipes:
        name = recipe.get("name")
        if not isinstance(name, str) or not name:
            raise RuntimeError("Every private dependency recipe needs a non-empty name")
        if name in by_name:
            raise RuntimeError(f"Private dependency manifest contains {name} more than once")
        if not isinstance(recipe.get("requires", []), list) or not all(
            isinstance(dependency, str) for dependency in recipe.get("requires", [])
        ):
            raise RuntimeError(f"Private dependency {name} has invalid requires")
        by_name[name] = recipe

    ordered = []
    visiting = set()
    complete = set()

    def visit(name):
        if name in complete:
            return
        if name in visiting:
            raise RuntimeError(f"Private dependency cycle includes {name}")
        recipe = by_name.get(name)
        if recipe is None:
            raise RuntimeError(f"Private dependency {name} is required but not declared")
        visiting.add(name)
        for dependency in recipe.get("requires", []):
            visit(dependency)
        visiting.remove(name)
        complete.add(name)
        ordered.append(recipe)

    for recipe in recipes:
        visit(recipe["name"])
    return ordered


def select_recipes(recipes, names):
    """Select named recipes and every declared private prerequisite."""
    if not names:
        return recipes
    by_name = {recipe["name"]: recipe for recipe in recipes}
    selected = set()

    def include(name):
        recipe = by_name.get(name)
        if recipe is None:
            raise RuntimeError(f"Private dependency {name} is not declared")
        if name in selected:
            return
        selected.add(name)
        for dependency in recipe.get("requires", []):
            include(dependency)

    for name in names:
        include(name)
    return [recipe for recipe in recipes if recipe["name"] in selected]


def verify_private_interfaces(prefix, recipes, env):
    """Reject a compatibility build that silently falls back to host GI data."""
    expected = {typelib for recipe in recipes for typelib in recipe.get("private_typelibs", [])}
    if any(recipe["name"] == "glib" for recipe in recipes):
        pkgconfig = prefix / "lib64/pkgconfig/girepository-2.0.pc"
        if not pkgconfig.is_file():
            raise RuntimeError(f"Private GLib did not install GIRepository 2: {pkgconfig}")
    typelib_dir = prefix / "lib64/girepository-1.0"
    for typelib in sorted(expected):
        path = typelib_dir / typelib
        if not path.is_file():
            raise RuntimeError(f"Private compatibility typelib is missing: {path}")
    if not any(recipe["name"] == "gjs" for recipe in recipes):
        return
    imports = ["imports.gi.GLib"]
    if any(recipe["name"] == "gnome-desktop" for recipe in recipes):
        imports.extend(["imports.gi.versions.GnomeDesktop = '4.0'", "imports.gi.GnomeDesktop", "imports.gi.GnomeQR"])
    namespaces = {
        "GdkPixbuf-2.0.typelib": "GdkPixbuf",
        "Gdk-4.0.typelib": "Gdk",
        "Gtk-4.0.typelib": "Gtk",
        "Gcr-4.typelib": "Gcr",
    }
    for typelib in sorted(expected):
        namespace = namespaces.get(typelib)
        if namespace:
            imports.append(f"imports.gi.{namespace}")
    runtime_env = env.copy()
    runtime_env.pop("LD_LIBRARY_PATH", None)
    system_typelibs = subprocess.check_output(
        ["pkg-config", "--variable=typelibdir", "gobject-introspection-1.0"], env=env, text=True
    ).strip()
    runtime_env["GI_TYPELIB_PATH"] += ":" + system_typelibs
    subprocess.run([str(prefix / "bin/gjs"), "-c", "; ".join(imports)], env=runtime_env, check=True)


def fix_linkage(directory, prefix):
    """Keep each installed ELF's private search path after Meson's install fixups."""
    patcher = prefix / "bin/patchelf"
    for path in directory.rglob("*"):
        if path.is_symlink() or not path.is_file():
            continue
        with path.open("rb") as source:
            header = source.read(18)
        if len(header) < 18 or header[:4] != b"\x7fELF":
            continue
        byteorder = "little" if header[5] == 1 else "big"
        if int.from_bytes(header[16:18], byteorder) not in (2, 3):
            continue
        existing = subprocess.check_output([str(patcher), "--print-rpath", str(path)], text=True).strip()
        paths = list(dict.fromkeys([str(prefix / "lib64"), *(part for part in existing.split(":") if part)]))
        with tempfile.TemporaryDirectory(dir=path.parent, prefix=".gnoblin-link-") as temporary:
            replacement = Path(temporary) / path.name
            shutil.copy2(path, replacement)
            subprocess.run(
                [str(patcher), "--force-rpath", "--set-rpath", ":".join(paths), str(replacement)], check=True
            )
            replacement.replace(path)


def install_tree(source, prefix):
    """Replace installed files atomically, including existing versioned symlinks."""
    prefix.mkdir(parents=True, exist_ok=True)
    for path in sorted(source.rglob("*")):
        destination = prefix / path.relative_to(source)
        if not destination.parent.resolve().is_relative_to(prefix):
            raise RuntimeError(f"Destination escapes private prefix: {destination}")
        if path.is_dir() and not path.is_symlink():
            destination.mkdir(exist_ok=True)
            continue
        with tempfile.TemporaryDirectory(dir=destination.parent, prefix=".gnoblin-install-") as directory:
            temporary = Path(directory) / "new"
            if path.is_symlink():
                temporary.symlink_to(os.readlink(path))
            else:
                shutil.copy2(path, temporary)
            temporary.replace(destination)


def build_environment(prefix):
    env = os.environ.copy()
    for key in ("GNOBLIN_PREFIX", "GNOBLIN_LIBDIR", "GSETTINGS_SCHEMA_DIR", "LD_LIBRARY_PATH", "GI_TYPELIB_PATH"):
        env.pop(key, None)
    for key, paths in {
        "PATH": [prefix / "bin"],
        "PKG_CONFIG_PATH": [prefix / "lib64/pkgconfig", prefix / "share/pkgconfig"],
        "GI_GIR_PATH": [prefix / "share/gir-1.0"],
        "GI_TYPELIB_PATH": [prefix / "lib64/girepository-1.0"],
        "LD_LIBRARY_PATH": [prefix / "lib64"],
        "XDG_DATA_DIRS": [prefix / "share"],
    }.items():
        previous = env.get(key, "/usr/local/share:/usr/share" if key == "XDG_DATA_DIRS" else "")
        env[key] = ":".join([*(str(path) for path in paths), *([previous] if previous else [])])
    # Install an explicit runtime search path, not a global loader configuration.
    # Preserve explicit RUNPATH entries where Meson permits; fix installed files separately.
    env["LDFLAGS"] = f"-Wl,--enable-new-dtags,-rpath,{prefix}/lib64 " + env.get("LDFLAGS", "")
    return env


def extract_archive(archive, destination):
    """Extract a source archive without permitting paths or links outside its staging tree.

    Python 3.10 and 3.11 do not have tarfile's ``filter='data'`` API.  The
    compatibility runtime is built on Ubuntu 22.04 and Debian 12, so retain
    the same boundary explicitly rather than requiring a newer interpreter.
    """
    root = destination.resolve()
    with tarfile.open(archive) as source:
        for member in source.getmembers():
            member_path = destination / member.name
            if Path(member.name).is_absolute() or not member_path.resolve().is_relative_to(root):
                raise RuntimeError(f"Archive member escapes source staging: {member.name}")
            if member.ischr() or member.isblk() or member.isfifo():
                raise RuntimeError(f"Archive member has unsupported type: {member.name}")
            if member.issym():
                link_path = member_path.parent / member.linkname
            elif member.islnk():
                link_path = destination / member.linkname
            else:
                continue
            if Path(member.linkname).is_absolute() or not link_path.resolve().is_relative_to(root):
                raise RuntimeError(f"Archive link escapes source staging: {member.name}")
        source.extractall(destination)


def checked_archive(recipe, downloads):
    archive = downloads / recipe["url"].rsplit("/", 1)[1]
    if not archive.exists():
        temporary = archive.with_suffix(archive.suffix + ".part")
        with urllib.request.urlopen(recipe["url"], timeout=60) as response, temporary.open("wb") as target:
            shutil.copyfileobj(response, target)
        temporary.replace(archive)
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    if digest != recipe["sha256"]:
        raise RuntimeError(f"Checksum mismatch: {archive}. Remove it before retrying.")
    return archive


def build(recipe, prefix, cache, env, jobs, manifest=None):
    platform_patch = ROOT / "packaging/deb/patches/mozjs-platform.patch"
    patch_bytes = platform_patch.read_bytes() if recipe.get("build_system") == "spidermonkey" else b""
    flags = {key: env.get(key, "") for key in ("CC", "CXX", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS")}
    identity = hashlib.sha256(
        json.dumps(json.loads((manifest or ROOT / "build-dependencies.json").read_text()), sort_keys=True).encode()
        + patch_bytes
        + str(prefix).encode()
        + json.dumps(flags, sort_keys=True).encode()
    ).hexdigest()
    marker = prefix / ".gnoblin-dependencies" / recipe["name"]
    if marker.exists() and marker.read_text().strip() == identity:
        print(f"[deps] {recipe['name']} {recipe['version']}: already built", flush=True)
        return
    work = cache / f"{recipe['name']}-{identity[:12]}"
    sources = work / "source"
    builddir = work / "build"
    downloads = cache / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    if not sources.exists():
        staging = work / "extract"
        shutil.rmtree(staging, ignore_errors=True)
        staging.mkdir(parents=True)
        extract_archive(checked_archive(recipe, downloads), staging)
        children = list(staging.iterdir())
        if len(children) != 1 or not children[0].is_dir():
            raise RuntimeError(f"Expected a single source directory for {recipe['name']}")
        children[0].rename(sources)
        staging.rmdir()
    print(f"[deps] Building {recipe['name']} {recipe['version']}", flush=True)
    # Stage first so absolute upstream install destinations cannot write to /usr.
    stage = work / "install"
    shutil.rmtree(stage, ignore_errors=True)
    system = recipe.get("build_system", "meson")
    if system in ("autotools", "spidermonkey"):
        configure = sources / ("js/src/configure" if system == "spidermonkey" else "configure")
        if system == "spidermonkey":
            if "#undef XP_UNIX" not in (sources / "js/src/js-config.h.in").read_text():
                subprocess.run(["patch", "-p1", "-i", str(platform_patch)], cwd=sources, check=True)
            env = {**env, "SHELL": "/bin/sh", "CC": "gcc", "CXX": "g++"}
        builddir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [str(configure), f"--prefix={prefix}", f"--libdir={prefix}/lib64", *recipe["options"]],
            cwd=builddir,
            env=env,
            check=True,
        )
        subprocess.run(["make", f"-j{jobs}"], cwd=builddir, env=env, check=True)
        subprocess.run(["make", "install", f"DESTDIR={stage}"], cwd=builddir, env=env, check=True)
    elif system == "cmake":
        subprocess.run(
            [
                "cmake",
                "-S",
                str(sources),
                "-B",
                str(builddir),
                "-G",
                "Ninja",
                f"-DCMAKE_INSTALL_PREFIX={prefix}",
                "-DCMAKE_INSTALL_LIBDIR=lib64",
                "-DCMAKE_BUILD_TYPE=Release",
                *recipe["options"],
            ],
            env=env,
            check=True,
        )
        subprocess.run(
            ["cmake", "--build", str(builddir), "--parallel", str(jobs), "--target", *recipe["targets"]],
            env=env,
            check=True,
        )
        subprocess.run(["cmake", "--install", str(builddir)], env={**env, "DESTDIR": str(stage)}, check=True)
    elif system == "lua":
        subprocess.run(["make", "clean"], cwd=sources, env=env, check=True)
        subprocess.run(
            ["make", "linux", f"-j{jobs}", "MYCFLAGS=-fPIC", f"MYLDFLAGS={env['LDFLAGS']}"],
            cwd=sources,
            env=env,
            check=True,
        )
        subprocess.run(
            ["make", "install", f"INSTALL_TOP={stage}{prefix}", f"INSTALL_LIB={stage}{prefix}/lib64"],
            cwd=sources,
            env=env,
            check=True,
        )
        pkgconfig = stage / prefix.relative_to("/") / "lib64/pkgconfig/lua.pc"
        pkgconfig.parent.mkdir(parents=True, exist_ok=True)
        pkgconfig.write_text(
            f"prefix={prefix}\nlibdir=${{prefix}}/lib64\nincludedir=${{prefix}}/include\n"
            f"Name: Lua\nDescription: Lua language runtime\nVersion: {recipe['version']}\n"
            "Libs: -L${libdir} -llua -lm -ldl\nCflags: -I${includedir}\n"
        )
    else:
        command = ["meson", "setup", str(builddir), str(sources)]
        if (builddir / "meson-private/coredata.dat").exists():
            command.append("--reconfigure")
        command += [
            f"--prefix={prefix}",
            "--libdir=lib64",
            f"--sysconfdir={prefix}/etc",
            f"--localstatedir={prefix}/var",
            "--buildtype=release",
            "--wrap-mode=nodownload",
        ]
        command += [option.format(prefix=prefix) for option in recipe["options"]]
        subprocess.run(command, env=env, check=True)
        subprocess.run(["meson", "compile", "-C", str(builddir), "-j", str(jobs)], env=env, check=True)
        subprocess.run(
            ["meson", "install", "-C", str(builddir), "--no-rebuild", "--destdir", str(stage)], env=env, check=True
        )
    staged_prefix = stage / prefix.relative_to("/")
    for path in stage.rglob("*"):
        if (path.is_file() or path.is_symlink()) and not path.is_relative_to(staged_prefix):
            raise RuntimeError(f"Install destination outside the private prefix: {path.relative_to(stage)}")
    if recipe["name"] != "patchelf":
        fix_linkage(staged_prefix, prefix)
    install_tree(staged_prefix, prefix)
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.write_text(identity + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, default=ROOT / "install/deps")
    parser.add_argument("--manifest", type=Path, default=ROOT / "build-dependencies.json")
    parser.add_argument("--cache", type=Path, default=ROOT / "build/dependencies")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--fix-runtime", action="store_true", help="Record private library paths in ./install binaries")
    parser.add_argument("--runtime-prefix", type=Path, default=ROOT / "install")
    parser.add_argument("--jobs", type=int, default=min(os.cpu_count() or 2, 8))
    parser.add_argument(
        "--only",
        nargs="+",
        metavar="RECIPE",
        help="Build only named private recipes and their declared private prerequisites",
    )
    parser.add_argument("--run", nargs=argparse.REMAINDER, help="Run a build command in the private environment")
    args = parser.parse_args()
    prefix = args.prefix.resolve()
    # A private subdirectory is required, even when called directly.
    if prefix in {
        path.resolve()
        for path in (
            Path("/"),
            Path("/usr"),
            Path("/usr/local"),
            Path("/lib"),
            Path("/lib64"),
            Path("/bin"),
            Path("/sbin"),
            Path("/etc"),
            Path.home(),
        )
    }:
        parser.error("Use a private dependency directory, not a shared prefix")
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    env = build_environment(prefix)
    if args.fix_runtime:
        runtime = args.runtime_prefix.resolve()
        if runtime in {Path("/"), Path("/usr"), Path("/usr/local"), Path("/lib"), Path("/lib64")}:
            parser.error("Use a private runtime directory")
        fix_linkage(runtime, prefix)
        return 0
    if args.run:
        return subprocess.call(args.run, env=env)
    recipes = json.loads(args.manifest.read_text())
    if not isinstance(recipes, list):
        parser.error("private dependency manifest must contain a JSON array")
    try:
        recipes = order_recipes(select_recipes(recipes, args.only))
    except RuntimeError as error:
        parser.error(str(error))
    if args.dry_run:
        print(f"Build private dependencies in {prefix}:")
        for recipe in recipes:
            print(f"  {recipe['name']} {recipe['version']} ({recipe['sha256'][:12]})")
        print("No system packages are installed or updated.")
        return 0
    if os.geteuid() == 0:
        parser.error("Run as your normal user; private dependency builds do not need root")
    for tool in ("meson", "ninja", "pkg-config", "cc", "c++", "make"):
        if not shutil.which(tool):
            parser.error(f"Missing build tool: {tool}. See docs/install-source.md")
    prefix.mkdir(parents=True, exist_ok=True)
    try:
        with (prefix / ".build.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            for recipe in recipes:
                build(recipe, prefix, args.cache.resolve(), env, args.jobs, args.manifest)
            verify_private_interfaces(prefix, recipes, env)
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"[deps] {error}")
        return 1
    print(f"[deps] Ready: {prefix}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
