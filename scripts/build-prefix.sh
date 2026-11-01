#!/usr/bin/env bash
# Build everything Gnoblin ships into a prefix, for pkgsmith to package.
#
# usage: scripts/build-prefix.sh [--prefix DIR] [--jobs N]
#
# The prefix is laid out like an installed system: DIR/usr/lib/gnoblin holds the private runtime (Mutter, the
# compositor, the portal backend), DIR/usr/bin, DIR/usr/share and DIR/usr/lib/systemd hold the public entries, and
# DIR/etc holds the configuration files of the optional GeoClue and ananicy-cpp packages. pkgsmith.star and
# pkgsmith-ananicy.star cut the packages out of it. Nothing here writes a package.
#
# The build is ./build.sh --layout system. It needs a Fedora system with the packages of
# scripts/install-fedora-build-deps.sh, checked-out submodules, and git author settings that allow commits (the
# build applies the patch series as commits in subprojects/mutter). Run it on the oldest glibc you want the packages to
# run on. CI runs it from a Git checkout of the ref, in the fedora:45 image and in alpine:edge, on amd64 and on arm64.
#
# One prefix serves every package format, so the build avoids what differs between distributions:
#   - Lua is linked statically. Fedora, openSUSE, Arch and Debian name the Lua library differently (liblua-5.5.so,
#     liblua5.5.so.5, liblua5.4.so.5.4, liblua5.4.so.0), so a program linked to one of them runs on no other. The
#     runtime loads no C Lua modules, so its own copy of the interpreter is safe.
#   - The libraries go to lib, not lib64, so the paths are the same on every distribution.
#   - Binaries are stripped and manual pages are compressed here, because pkgsmith packs files as they are.
#
# Environment:
#   GNOBLIN_PREFIX_JOBS   parallel jobs, the default is the number of processors
#   SOURCE_DATE_EPOCH     time stamp for the files that this script writes. Default: the commit date
set -euo pipefail

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"

prefix="$root/prefix"
jobs="${GNOBLIN_PREFIX_JOBS:-$(nproc)}"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            prefix="${2:?--prefix needs a directory}"
            shift 2
            ;;
        --jobs)
            jobs="${2:?--jobs needs a number}"
            shift 2
            ;;
        *)
            echo "usage: $0 [--prefix DIR] [--jobs N]" >&2
            exit 2
            ;;
    esac
done
prefix="$(realpath -m -- "$prefix")"
private=/usr/lib/gnoblin

lua_version=5.5.1
lua_sha256=1c4b4068d67061f2a2231ad2b5422e77acea1487ea9890f6320af614f4373dce

for tool in cmake ninja meson pkg-config python3 git strip gzip; do
    command -v "$tool" >/dev/null || {
        echo "[prefix] missing $tool. On Fedora run scripts/install-fedora-build-deps.sh first." >&2
        exit 1
    }
done
for submodule in subprojects/mutter subprojects/xdg-desktop-portal-gnome subprojects/imgui; do
    [ -e "$submodule/.git" ] || {
        echo "[prefix] $submodule is not checked out. Run scripts/checkout-submodules-with-retry.sh first." >&2
        exit 1
    }
done
: "${SOURCE_DATE_EPOCH:=$(git log -1 --format=%ct)}"
export SOURCE_DATE_EPOCH

echo "[prefix] upstream release tags of the pinned sources"
for submodule in subprojects/mutter subprojects/xdg-desktop-portal-gnome; do
    # The submodules are shallow. The source step resets each one to the upstream tag it pins, so the tags must be there.
    git -C "$submodule" fetch --quiet --force --tags origin
done

# The build directories carry the target of the compiler, so a prefix build next to a development build, or one for
# another C library in the same checkout, does not share state with it.
target="$(cc -dumpmachine)"
export GNOBLIN_BUILD_DIR="${GNOBLIN_BUILD_DIR:-build/prefix-$target}"

echo "[prefix] static Lua $lua_version"
lua_root="$root/build/lua-static-$target"
if [ ! -f "$lua_root/lib/pkgconfig/lua.pc" ]; then
    lua_source="$root/build/lua-src-$target"
    rm -rf -- "$lua_root" "$lua_source"
    mkdir -p "$lua_source" "$lua_root"
    curl -fsSL --retry 5 "https://www.lua.org/ftp/lua-$lua_version.tar.gz" -o "$lua_source.tar.gz"
    echo "$lua_sha256  $lua_source.tar.gz" | sha256sum -c -
    tar -C "$lua_source" --strip-components=1 -xzf "$lua_source.tar.gz"
    make -C "$lua_source" -j"$jobs" linux MYCFLAGS='-fPIC'
    make -C "$lua_source" install INSTALL_TOP="$lua_root"
    # The shared library must not win over the static one at link time.
    rm -f "$lua_root"/lib/liblua.so*
    mkdir -p "$lua_root/lib/pkgconfig"
    cat >"$lua_root/lib/pkgconfig/lua.pc" <<PC
prefix=$lua_root
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: Lua
Description: Lua, linked statically into Gnoblin
Version: $lua_version
Libs: -L\${libdir} -llua -lm
Cflags: -I\${includedir}
PC
fi
export PKG_CONFIG_PATH="$lua_root/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

# The compiler and linker flags that rpmbuild uses on Fedora: hardening, fortify and the rest.
if command -v rpm >/dev/null && rpm --eval '%{set_build_flags}' | grep -q CFLAGS; then
    eval "$(rpm --eval '%{set_build_flags}')"
fi
# Mutter hardcodes -Werror=array-bounds. With the LTO and fortify flags that Fedora sets, GCC reports a false positive
# on g_free (grab) in src/core/keybindings.c. Keep the warning and stop it from failing the build.
export CFLAGS="${CFLAGS:-} -Wno-error=array-bounds"

rm -rf -- "$prefix"
mkdir -p "$prefix"

echo "[prefix] ./build.sh --layout system into $prefix"
GNOBLIN_LIBDIR=lib GNOBLIN_BUILD_TYPE=release GNOBLIN_LAYOUT_GEOCLUE=1 GNOBLIN_LAYOUT_ANANICY=1 \
    ./build.sh --layout system --prefix "$private" --destdir "$prefix" --jobs "$jobs"

echo "[prefix] licences"
install -Dm644 COPYING "$prefix$private/share/licenses/gnoblin/COPYING"
install -Dm644 subprojects/xdg-desktop-portal-gnome/COPYING \
    "$prefix$private/share/licenses/xdg-desktop-portal-gnoblin/COPYING"

echo "[prefix] strip binaries and compress manual pages, as package builds do"
while IFS= read -r -d '' file; do
    if [ "$(head -c 4 -- "$file" | od -An -tx1 | tr -d ' ')" = 7f454c46 ]; then
        strip --strip-unneeded -- "$file"
    fi
done < <(find "$prefix" -type f -print0)
while IFS= read -r -d '' page; do
    gzip -9n -- "$page"
done < <(find "$prefix" -type f -path '*/share/man/man*/*' ! -name '*.gz' -print0)
# The compressed pages replace the uncompressed ones, so the links that name them must follow.
while IFS= read -r -d '' link; do
    case "$(readlink "$link")" in
        *.gz) ;;
        *) ln -sfn "$(readlink "$link").gz" "$link.gz" && rm -- "$link" ;;
    esac
done < <(find "$prefix" -type l -path '*/share/man/man*/*' ! -name '*.gz' -print0)

echo "[prefix] done: $(find "$prefix" -type f | wc -l) files, $(find "$prefix" -type l | wc -l) links in $prefix"
