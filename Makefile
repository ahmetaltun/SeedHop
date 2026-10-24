.PHONY: build test lint render interop smoke ux-test
build:
	python3 scripts/build.py
test:
	cargo test --workspace --locked
lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
render: build
	.build/SeedHop.app/Contents/MacOS/SeedHop --render-designs --output .build/design
interop:
	python3 scripts/interop.py
smoke: build
	cargo run -p seedhop-core --bin seedhop-lab -- fixture .build/fixture
	.build/SeedHop.app/Contents/MacOS/SeedHop --verify-engine "$(CURDIR)/.build/fixture"
ux-test: build
	.build/SeedHop.app/Contents/MacOS/SeedHop --verify-ux
