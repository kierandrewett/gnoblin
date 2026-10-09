# Gnoblin. ./build.sh is the build; these targets only name its common uses.
#
#   make           build into ./install
#   make install   build, then add the login entry and link gnoblinctl and the man pages (asks for sudo)
#   make check     the fast checks
#   make test      build the tests, then run the CTest suites
#   make clean     delete build/ and keep ./install
#
# Settings:  make JOBS=4 DEVKIT=0     DEVKIT=1 builds the nested viewer that the preview and the devkit tests need.
# Another prefix, or what a build would change first: use ./build.sh (--prefix DIR, and GNOBLIN_DEVKIT=enabled ./build.sh --dry-run).

JOBS ?= $(shell nproc)
DEVKIT ?= 1
BUILD_DIR := $(or $(GNOBLIN_BUILD_DIR),build/ninja)

.PHONY: build install check test clean help
.DEFAULT_GOAL := build

build:
	@GNOBLIN_DEVKIT=$(if $(filter 1,$(DEVKIT)),enabled,disabled) ./build.sh --jobs $(JOBS)

install: build
	@./build.sh --register-session

check:
	@./tests/check-fast.sh

test: build
	@cmake --build "$(BUILD_DIR)" --target gnoblin-tests --parallel $(JOBS)
	@ctest --test-dir "$(BUILD_DIR)" --output-on-failure

clean:
	rm -rf build

help:
	@sed -n '1,/^$$/{s/^# \{0,1\}//;p}' $(MAKEFILE_LIST)
