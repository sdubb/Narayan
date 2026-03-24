.PHONY: all check build test lint fmt fmt-check clean run fix warnings

all: fmt-check lint test

check:
	cargo check

build:
	cargo build

test:
	cargo test

lint:
	cargo clippy -- -W clippy::all

fmt:
	cargo fmt

fmt-check:
	cargo fmt -- --check

clean:
	cargo clean

run:
	cargo run

fix:
	cargo fix --bin "narayan" -p narayan --allow-dirty --allow-no-vcs

warnings:
	@echo "Warning count:"
	@cargo check 2>&1 | grep "^warning" | grep -v "following packages" | wc -l
