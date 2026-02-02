.PHONY: check coverage fmt install lint license coverage outdated

# Run all CI checks
check:
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

# Add license headers
license:
	quench check --fix --license

# Generate coverage report
coverage:
	@scripts/coverage

# Check for outdated dependencies
outdated:
	cargo outdated
