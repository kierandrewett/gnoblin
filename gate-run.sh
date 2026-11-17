#!/bin/bash
cargo test -p gnoblin-std-impl --test examples 2>&1 | grep -E "^test |test result|panicked"
