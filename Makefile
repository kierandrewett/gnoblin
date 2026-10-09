# Gnoblin. ./build.sh is the build; these targets only name its common uses.
#
#   make           build for PREFIX into build/stage (nothing is installed yet)
#   make preview   open the built compositor in a window on this desktop
#   make install   copy the build to PREFIX and register it with the login screen (asks for sudo; run make first)
#   make check     the fast checks
#   make test      build the tests, then run the CTest suites
#   make clean     delete build/, including the staged copy; an installed copy stays
#
# Settings:  make JOBS=4 DEVKIT=0 PREFIX=/opt/gnoblin
#   PREFIX   where make install puts the runtime (default /usr/local/lib/gnoblin). The binaries have it compiled in,
#            so use the same value for make and make install.
#   DEVKIT=1 builds the nested viewer that the preview and the devkit tests need.
# A private build in ./install, or what a build would change first: use ./build.sh (--prefix DIR, --dry-run).

PREFIX ?= /usr/local/lib/gnoblin
JOBS ?= $(shell nproc)
DEVKIT ?= 1
STAGE := $(CURDIR)/build/stage
BUILD_DIR := $(or $(GNOBLIN_BUILD_DIR),build/make)
export GNOBLIN_BUILD_DIR := $(BUILD_DIR)
export GNOBLIN_VIA_MAKE := 1

.PHONY: build preview install check test clean help
.DEFAULT_GOAL := build

build:
	@GNOBLIN_DEVKIT=$(if $(filter 1,$(DEVKIT)),enabled,disabled) ./build.sh --jobs $(JOBS) --prefix $(PREFIX) --destdir $(STAGE)

preview:
	@GNOBLIN_PREFIX=$(PREFIX) ./scripts/run-staged.sh $(PREFIX) $(STAGE) ./scripts/run-gnoblin-devkit.sh

install:
	@./scripts/register-session.sh $(PREFIX) $(STAGE)

check:
	@./tests/check-fast.sh

test: build
	@cmake --build "$(BUILD_DIR)" --target gnoblin-tests --parallel $(JOBS)
	@ctest --test-dir "$(BUILD_DIR)" --output-on-failure

clean:
	rm -rf build

help:
	@sed -n '1,/^$$/{s/^# \{0,1\}//;p}' $(MAKEFILE_LIST)
