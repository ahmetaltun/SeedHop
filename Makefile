.PHONY: build test lint render interop smoke ux-test interface-test faults v2-test v2-hardening layout-recovery performance workspace-test
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

v2-hardening:
	python3 scripts/v2_hardening.py

layout-recovery:
	python3 scripts/layout_recovery.py --cross-volume

performance:
	python3 scripts/build.py --identity -
	SEEDHOP_PERF_OUTPUT="$(CURDIR)/.build/performance-core-current.json" cargo test -p seedhop-core --test performance --locked -- --ignored --exact library_snapshot_baseline --nocapture
	.build/SeedHop.app/Contents/MacOS/SeedHop --verify-performance --output "$(CURDIR)/.build/performance-ui-current.json"
	python3 scripts/check_performance.py --core .build/performance-core-current.json --ui .build/performance-ui-current.json --output .build/performance-budget-current.json

workspace-test: build
	.build/SeedHop.app/Contents/MacOS/SeedHop --verify-workspace-layout
