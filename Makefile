# chase CLI — task runner: build/test/lint/fmt/clean/install, a cheap `smoke`
# check, and an aggregate `verify` gate. Thin wrappers over cargo so a green
# local run predicts a green CI run.

BIN := chse
CARGO := cargo

.PHONY: all build release test lint fmt fmt-check clean install deps smoke verify dev

all: verify

build: SIGN_TARGET = target/debug/$(BIN)
build:
	$(CARGO) build
	@$(SIGN)

release: SIGN_TARGET = target/release/$(BIN)
release:
	$(CARGO) build --release
	@$(SIGN)

test:
	$(CARGO) test --all

lint:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

clean:
	$(CARGO) clean

# `cargo install` ad-hoc signs, which gives the binary a *new* code identity
# every time. macOS scopes keychain "Always Allow" grants to that identity, so
# an unsigned reinstall silently revokes them and the next run prompts again.
# The one secret this CLI can store is the bearer token of `mcp --http`, which is
# why it is signed on the same terms as the rest of the family.
install: SIGN_TARGET = $${CARGO_INSTALL_ROOT:-$$HOME/.cargo}/bin/$(BIN)
install:
	$(CARGO) install --path . --force --locked
	@$(SIGN)

deps:
	$(CARGO) fetch

# Cheap sanity checks needing no config, network, or credential. The import
# run is the point: it proves the release binary actually parses a Chase
# export, not merely that it starts.
smoke: release
	./target/release/$(BIN) --version
	./target/release/$(BIN) --help > /dev/null
	./target/release/$(BIN) info > /dev/null
	./target/release/$(BIN) auth status > /dev/null
	./target/release/$(BIN) mcp --list-tools > /dev/null
	./target/release/$(BIN) --json -q import csv tests/fixtures/card-modern.csv > /dev/null
	./target/release/$(BIN) --json -q import scan tests/fixtures > /dev/null

verify: fmt-check lint test smoke

# Debug build re-signed with the same stable pk-cli-codesign identity, so the
# dev loop doesn't re-prompt either (see cli-common/scripts).
dev: SIGN_TARGET = target/debug/$(BIN)
dev:
	cargo build
	@$(SIGN)

# Shared re-signing step. No-ops with a note when the helper or identity is
# absent (CI, Linux, a fresh machine that hasn't run setup-dev-signing.sh).
define SIGN
if [ -x "$$HOME/Dev/cli-common/scripts/dev-sign.sh" ]; then \
	"$$HOME/Dev/cli-common/scripts/dev-sign.sh" "$(SIGN_TARGET)"; \
else echo "cli-common/scripts/dev-sign.sh not found — $(SIGN_TARGET) left ad-hoc signed"; fi
endef
