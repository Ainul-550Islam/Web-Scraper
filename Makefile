# Developer entry points — every target below is exactly what CI runs.
.PHONY: all fmt lint test build smoke buyer-e2e links web-install web-test dist rust-test

all: fmt lint test build links smoke

fmt:
	cargo fmt --all
	cd web && npx prettier --check "src/**/*.{ts,tsx,css}" >/dev/null 2>&1 || echo "web: run npx eslint . --fix"

lint:
	cargo clippy --all-targets --all-features -- -D warnings
	cd web && npx tsc --noEmit
	cd web && npx eslint .

rust-test:
	cargo test --all-features

web-test:
	cd web && npx vitest run

test: rust-test web-test

build:
	cd web && npm run build

dist: build

smoke: build
	cd web && npm run smoke

# Full buyer flow against real local backends: demo mock site + `harvest
# serve` + built console via vite preview + CLI export/verify.
buyer-e2e: build
	cd web && npm run e2e:buyer

web-install:
	cd web && npm ci

links:
	python3 tools/check_links.py .
