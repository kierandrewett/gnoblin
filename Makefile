# Gnoblin: the front door for building, checking and packaging.
#
# ./build.sh is the build. These targets name what you want and call it with the right flags. They hold no build
# logic of their own, and each one prints the command it runs.
#
#   make              list the targets
#   make status       show what is built, what is registered and what is on your PATH
#   make plan         show what "make build" would change, without changing anything
#
# Variables (set on the command line, for example: make build JOBS=4 DEVKIT=0)
#   PREFIX   where a development build is installed         (default: ./install)
#   JOBS     parallel build jobs                            (default: the number of CPUs)
#   DEVKIT   1 builds the nested viewer that "make preview" and the devkit tests need (default: 1)

SHELL := bash
.SHELLFLAGS := -euo pipefail -c
.DEFAULT_GOAL := help
MAKEFLAGS += --no-print-directory

PREFIX ?= $(CURDIR)/install
JOBS ?= $(shell nproc)
DEVKIT ?= 1
SYSTEM_PREFIX ?= /usr
DESTDIR ?=
OUTPUT ?= dist/release
ARGS ?=
TERMINAL ?=

DEVKIT_MODE := $(if $(filter 1,$(DEVKIT)),enabled,disabled)
export GNOBLIN_PREFIX := $(PREFIX)

# Print the command, then run it.
define run
	@echo "+ $(1)"
	@$(1)
endef

# The Mutter test suites that "make test-window-manager" runs.
MUTTER_TEST_OPTS := --prefix=$(PREFIX) --libdir=lib64 --buildtype=debugoptimized -Ddevkit=enabled -Dtests=enabled \
	-Dmutter_tests=true -Dclutter_tests=false -Dcogl_tests=false -Ddocs=false -Dprofiler=false \
	-Dudev_dir=$(PREFIX)/lib/udev
MUTTER_SUITES := --suite mutter:mutter/unit --suite mutter:mutter/wayland --suite mutter:mutter/backends/native
MUTTER_FOCUS_TESTS := mutter:focus-default-window-globally-active-input mutter:click-to-focus-and-raise \
	mutter:overview-focus mutter:sloppy-focus mutter:sloppy-focus-pointer-rest mutter:sloppy-focus-auto-raise \
	mutter:popup-focus

.PHONY: help status plan setup build build-system register preview man clean package package-srpm \
	check test-runtime test-all test-release test-window-manager test-preview test-privacy-pipewire \
	test-window-csd test-window-borders test-window-rule-lifecycle test-window-shadows lint format \
	package-manifest check-gnome-version install-fedora

##@ Start here

help: ## Show this list.
	@echo "Gnoblin make targets    (PREFIX=$(PREFIX)  JOBS=$(JOBS)  DEVKIT=$(DEVKIT))"
	@awk 'BEGIN { FS = ":.*## " } /^##@/ { printf "\n%s\n", substr($$0, 5) } /^[a-zA-Z0-9_-]+:.*## / { printf "  %-24s %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@echo
	@echo "Run 'make status' to see the current state, and 'make plan' before the first build."

status: ## Show the build, the login entry, the commands on your PATH and the man page links.
	@echo "Source"
	@printf '  %-18s %s\n' "branch" "$$(git branch --show-current) at $$(git rev-parse --short HEAD)"
	@printf '  %-18s %s\n' "build directory" "$$([ -d build ] && du -sh build | cut -f1 || echo 'none')"
	@echo
	@echo "Development build in $(PREFIX)"
	@if [ -x "$(PREFIX)/bin/gnoblin" ]; then \
	    "$(PREFIX)/bin/gnoblin" --version | sed 's/^/  /'; \
	else \
	    echo "  not built. Run: make build"; \
	fi
	@echo
	@echo "Login entry"
	@if [ -f /usr/share/wayland-sessions/gnoblin.desktop ]; then \
	    printf '  %-18s %s\n' "starts" "$$(sed -n 's/^Exec=//p' /usr/share/wayland-sessions/gnoblin.desktop)"; \
	else \
	    echo "  none. Run: make register"; \
	fi
	@echo
	@echo "Commands on your PATH"
	@for command in gnoblin gnoblinctl; do \
	    found="$$(command -v $$command || true)"; \
	    if [ -n "$$found" ]; then \
	        target="$$(readlink -f "$$found")"; \
	        printf '  %-18s %s -> %s\n' "$$command" "$$found" "$$target"; \
	        case "$$target" in "$(PREFIX)"/*) ;; *) echo "                     note: this is not the build in $(PREFIX). Run $(PREFIX)/bin/$$command for that one." ;; esac; \
	    else \
	        printf '  %-18s not found\n' "$$command"; \
	    fi; \
	done
	@echo
	@echo "Man pages"
	@for page in gnoblin.1 gnoblinctl.1; do \
	    link="$${XDG_DATA_HOME:-$$HOME/.local/share}/man/man1/$$page"; \
	    if [ -L "$$link" ]; then \
	        printf '  %-18s %s\n' "$$page" "linked to $$(readlink "$$link")"; \
	    elif [ -e "$$link" ]; then \
	        printf '  %-18s %s\n' "$$page" "a file that is not a link"; \
	    else \
	        printf '  %-18s %s\n' "$$page" "not linked (make register links it)"; \
	    fi; \
	done

plan: ## Show what "make build" would do and which paths it would change. Changes nothing.
	$(call run,./build.sh --dry-run --prefix $(PREFIX))

##@ Build

setup: ## Fetch the pinned source trees. Changes: subprojects/ (downloads and checks out pins).
	$(call run,./build.sh --target prepare-sources)

build: ## Build into PREFIX. Changes: PREFIX, build/, subprojects/mutter and the portal (patched). Not your login entry.
	$(call run,GNOBLIN_DEVKIT=$(DEVKIT_MODE) ./build.sh --jobs $(JOBS) --prefix $(PREFIX))

build-system: ## Build the package layout into DESTDIR (needs DESTDIR=...). Changes: DESTDIR, build/.
	@test -n "$(DESTDIR)" || { echo "Set DESTDIR, for example: make build-system DESTDIR=/tmp/stage" >&2; exit 2; }
	$(call run,./build.sh --jobs $(JOBS) --layout system --system-prefix $(SYSTEM_PREFIX) --destdir $(DESTDIR))

register: ## Add Gnoblin to the login screen and link its commands and man pages. Needs sudo. Changes: /usr/share, ~/.local.
	$(call run,./build.sh --prefix $(PREFIX) --register-session)

preview: ## Open the compositor in a nested viewer window on the current desktop (TERMINAL=name).
	$(call run,./scripts/run-gnoblin-devkit.sh $(TERMINAL))

man: ## Write the man pages into PREFIX/share/man/man1. Changes: PREFIX/share/man.
	$(call run,python3 scripts/build-man-pages.py $(PREFIX)/share/man/man1)

clean: ## Delete build/. Keeps PREFIX, so an installed build keeps working.
	$(call run,rm -rf build)

##@ Packages

package: ## Write the release assets: source bundle, Arch recipes, checksums (OUTPUT=dir, default dist/release).
	$(call run,./build.sh package --output $(OUTPUT))

package-srpm: ## Same as package, plus the Fedora source RPMs (needs rpm-build).
	$(call run,./build.sh package --srpm --output $(OUTPUT))

package-manifest: ## Check the generated recipes against packaging/native-packages.json (ARGS=write to regenerate).
	$(call run,./scripts/sync-package-manifest.py $(if $(ARGS),$(ARGS),check))

check-gnome-version: ## Compare the pinned GNOME version with upstream. Needs the network.
	$(call run,./scripts/gnome-versions.py check --upstream)

install-fedora: ## Install the published Fedora packages from COPR. Needs sudo.
	$(call run,./scripts/install-system.sh)

##@ Checks that need no session

check: ## Fast checks: manifests, patches, script syntax, config and package tests.
	$(call run,./scripts/gnome-versions.py check)
	$(call run,./scripts/manage-patches.py check)
	@echo "+ bash -n scripts/*.sh tests/*.sh src/tools/*.sh"
	@for file in scripts/*.sh tests/*.sh src/tools/*.sh; do bash -n "$$file"; done
	@echo "+ python3 -m py_compile scripts/*.py tests/*.py"
	@tmp="$$(mktemp -d)"; trap 'rm -rf "$$tmp"' EXIT; PYTHONPYCACHEPREFIX="$$tmp" python3 -m py_compile scripts/*.py tests/*.py
	$(call run,./tests/test-log-diagnostics.sh)
	$(call run,./tests/test-secure-state.sh)
	$(call run,./tests/test-rpm-sources.sh)
	$(call run,./tests/config-seed.test.sh)
	$(call run,python3 tests/frame-renderer-policy.test.py)
	$(call run,python3 tests/session-environment.test.py)
	$(call run,python3 tests/package-isolation.test.py)
	$(call run,python3 tests/check-build-deps.test.py)
	$(call run,python3 tests/test_generate_mutter_keybinding_catalog.py)
	$(call run,./tests/test-config.sh)

test-runtime: ## Run the CTest suites in build/ninja, then the config tests. Needs an earlier build.
	$(call run,ctest --test-dir build/ninja --output-on-failure)
	$(call run,./tests/test-config.sh)

test-all: check ## check, then build every target, then test-runtime.
	$(call run,cmake --build build/ninja --parallel)
	@$(MAKE) test-runtime

test-release: test-all test-window-manager ## test-all, then the Mutter suites. Run before a release.

test-window-manager: ## Mutter unit, Wayland, backend and focus suites. Patches subprojects/mutter, builds build/mutter-tests.
	$(call run,./scripts/apply-patches.sh mutter)
	$(call run,meson setup --reconfigure build/mutter-tests subprojects/mutter $(MUTTER_TEST_OPTS) || meson setup build/mutter-tests subprojects/mutter $(MUTTER_TEST_OPTS))
	$(call run,meson compile -C build/mutter-tests)
	$(call run,meson test -C build/mutter-tests --no-rebuild --num-processes 1 --print-errorlogs $(MUTTER_SUITES))
	$(call run,meson test -C build/mutter-tests --no-rebuild --num-processes 1 --print-errorlogs $(MUTTER_FOCUS_TESTS))

lint: ## Run the linters (ARGS="--files ..." to limit them).
	$(call run,./scripts/quality.sh lint $(ARGS))

format: ## Format the files (ARGS="--files ..." to limit it).
	$(call run,./scripts/quality.sh format $(ARGS))

##@ Checks that open a nested session on this desktop

test-preview: ## Config, native controls and the workspace animation in a nested session.
	$(call run,./tests/test-gnoblin-devkit.sh)

test-privacy-pipewire: ## Live microphone and camera activity through Lua. Needs PipeWire.
	$(call run,python3 tests/test-privacy-pipewire.py)

test-window-csd: ## Compositor reconstruction of client-drawn transparent corners.
	$(call run,GNOBLIN_DEVKIT_EXEC='python3 $(CURDIR)/tests/test-window-csd-reconstruction.py' ./scripts/run-gnoblin-devkit.sh)

test-window-borders: ## Lua window rules draw native borders.
	$(call run,GNOBLIN_DEVKIT_EXEC='python3 $(CURDIR)/tests/test-window-native-borders.py' ./scripts/run-gnoblin-devkit.sh)

test-window-rule-lifecycle: ## Repeated Lua window rule reloads keep a live window.
	$(call run,GNOBLIN_DEVKIT_CONFIG_SOURCE='$(CURDIR)/tests/configs/window-rule-lifecycle' GNOBLIN_DEVKIT_EXEC='python3 $(CURDIR)/tests/test-window-rule-lifecycle.py' ./scripts/run-gnoblin-devkit.sh)

test-window-shadows: ## Lua replacement shadows and transparent window compositing.
	$(call run,GNOBLIN_DEVKIT_EXEC='python3 $(CURDIR)/tests/test-window-native-shadows.py' ./scripts/run-gnoblin-devkit.sh)
