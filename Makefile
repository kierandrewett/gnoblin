# Gnoblin. ./build.sh is the build; these targets only name its common uses.
#
#   make           build into ./install
#   make install   build, then add the login entry, the commands and the man pages (asks for sudo)
#   make check     the fast checks
#   make test      the CTest suites (needs a build)
#   make clean     delete build/ and keep ./install
#
# Settings:  make JOBS=4 DEVKIT=0     DEVKIT=1 builds the nested viewer that the preview and the devkit tests need.
# See what a build would change first with: ./build.sh --dry-run

JOBS ?= $(shell nproc)
DEVKIT ?= 1
PREFIX ?= $(CURDIR)/install

.PHONY: build install check test clean help
.DEFAULT_GOAL := build

build:
	GNOBLIN_DEVKIT=$(if $(filter 1,$(DEVKIT)),enabled,disabled) ./build.sh --jobs $(JOBS) --prefix $(PREFIX)

install: build
	./build.sh --prefix $(PREFIX) --register-session

check:
	./tests/check-fast.sh

test:
	ctest --test-dir build/ninja --output-on-failure

clean:
	rm -rf build

help:
	@sed -n '1,/^$$/{s/^# \{0,1\}//;p}' $(MAKEFILE_LIST)
