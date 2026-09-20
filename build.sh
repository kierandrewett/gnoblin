#!/usr/bin/env bash
# Build private dependencies and the required Gnoblin runtime.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "$0")")"

usage() {
    cat <<'HELP'
Usage: ./build.sh [--deps-only | --no-deps] [--dry-run]

Build private dependencies in ./install/deps, then Gnoblin in ./install.
No host package manager is invoked. Run as your normal user.

  --deps-only  Build private dependencies without building Gnoblin
  --no-deps    Reuse the existing private dependencies
  --dry-run    Print the build steps without changing files
  --help       Show this help
HELP
}

skip_deps=false
deps_only=false
dry_run=false
for argument in "$@"; do
    case "$argument" in
        --yes | --install-deps) ;; # Compatibility: private builds need no prompts.
        --no-deps) skip_deps=true ;;
        --deps-only) deps_only=true ;;
        --dry-run) dry_run=true ;;
        --help | -h)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done
if "$skip_deps" && "$deps_only"; then
    echo '--no-deps cannot be combined with --deps-only.' >&2
    exit 2
fi
if "$dry_run"; then
    if ! "$skip_deps"; then
        python3 scripts/build-private-deps.py --dry-run
    fi
    if ! "$deps_only"; then
        printf 'Build Gnoblin into %s/install using ./install/deps.\n' "$PWD"
        echo '  just reset <previously generated subprojects>'
        echo '  just setup'
        echo '  python3 scripts/check-build-deps.py'
        echo '  just build-source'
    fi
    exit 0
fi
for tool in python3 git; do
    if ! command -v "$tool" >/dev/null; then
        echo "Missing $tool. See docs/install-source.md for build prerequisites." >&2
        exit 1
    fi
done
if ! "$deps_only" && ! command -v just >/dev/null; then
    echo 'Missing just. See docs/install-source.md for build prerequisites.' >&2
    exit 1
fi
if "$skip_deps" && [ ! -x install/deps/bin/patchelf ]; then
    echo 'Private dependencies are missing. Run ./build.sh without --no-deps first.' >&2
    exit 1
fi
if ! "$skip_deps"; then
    python3 scripts/build-private-deps.py
fi
"$deps_only" && exit 0

# A running system session can export GNOBLIN_PREFIX=/usr. Source builds always
# stay in this checkout; use the individual Just recipes for custom prefixes.
export GNOBLIN_PREFIX="$PWD/install"
export GNOBLIN_LIBDIR=lib64
# Applying our patches creates local commits; no global Git setup is needed.
export GIT_AUTHOR_NAME="${GIT_AUTHOR_NAME:-$(git config user.name || echo 'Gnoblin build')}"
export GIT_AUTHOR_EMAIL="${GIT_AUTHOR_EMAIL:-$(git config user.email || echo 'build@gnoblin.local')}"
export GIT_COMMITTER_NAME="${GIT_COMMITTER_NAME:-$GIT_AUTHOR_NAME}"
export GIT_COMMITTER_EMAIL="${GIT_COMMITTER_EMAIL:-$GIT_AUTHOR_EMAIL}"
# Validate and clear only previously generated patch state before submodule init.
for project in mutter gnome-shell gnome-control-center xdg-desktop-portal-gnome; do
    if [ -f "build/subproject-state/$project.sha256" ]; then
        just reset "$project"
    fi
done
just setup
python3 scripts/build-private-deps.py --run python3 scripts/check-build-deps.py
python3 scripts/build-private-deps.py --run just build-source
python3 scripts/build-private-deps.py --fix-runtime
printf '\nComplete Gnoblin build installed in %s\n' "$GNOBLIN_PREFIX"
printf 'Optional Settings and portal builds: docs/source-development.md\n'
