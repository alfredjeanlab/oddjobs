.PHONY: check ci coverage fmt install lint license coverage outdated

# Quick checks
#
# Flags:
#   quench --no-cloc         # Don't force agents to fix LOC errors
#   cargo test --workspace   # Unlike --all skips 'tests/specs'
#
# Excluded:
#   SKIP `cargo audit`
#   SKIP `cargo deny`
#
check:
	cargo fmt --all
	cargo clippy --all -- -D warnings
	quench check --fix --no-cloc
	cargo build --all
	cargo test --workspace

# Full pre-release checks
ci:
	cargo fmt --all
	cargo clippy --all -- -D warnings
	quench check --fix
	cargo build --all
	cargo test --all
	cargo audit
	cargo deny check licenses bans sources

# Format code
fmt:
	cargo fmt --all

# Build and install oj to ~/.local/bin
install:
	@scripts/install

# Add license headers (--ci required for --license)
license:
	quench check --ci --fix --license

# Generate coverage report
coverage:
	@scripts/coverage

# Check for outdated dependencies
outdated:
	cargo outdated
