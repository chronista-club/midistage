#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
rustup run 1.96.0 cargo build -p midistaged --example protocol_fixture
MIDISTAGE_TEST_SERVER="$PWD/target/debug/examples/protocol_fixture" swift test
