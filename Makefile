.PHONY: build test lint render interop smoke ux-test interface-test faults v2-test
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
faults:
	python3 scripts/faults.py
smoke: build
	cargo run -p seedhop-core --bin seedhop-lab -- fixture .build/fixture
	.build/SeedHop.app/Contents/MacOS/SeedHop --verify-engine "$(CURDIR)/.build/fixture"
ux-test: build
	.build/SeedHop.app/Contents/MacOS/SeedHop --verify-ux
interface-test: build
	.build/SeedHop.app/Contents/MacOS/SeedHop --verify-interface --output "$(CURDIR)/.build/interface-navigation.png"

v2-test:
	python3 scripts/v2_interop.py
