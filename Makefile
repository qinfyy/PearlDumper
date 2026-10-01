# Makefile

.PHONY: build check test clean

build:
	cargo build --release --locked

check:
	cargo check --workspace --locked

test:
	cargo test --workspace --locked

clean:
	cargo clean
