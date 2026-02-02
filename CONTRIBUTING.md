# Contributing

## License

By contributing, you acknowledge that contributions are licensed under BSL 1.1 (see LICENSE).

## Dependencies

**claudeless** - Mock Claude CLI for deterministic agent integration tests.

```bash
# Install via Homebrew
brew install alfredjean/tap/claudeless

# Or install from source
curl -sSL https://raw.githubusercontent.com/alfredjean/claudeless/main/install.sh | bash
```

## Build & Test

```bash
cargo build
make check   # Run all CI checks (fmt, clippy, test, build, audit, deny)
```

## Context

See `docs/` and `plans/` for architecture and implementation details.
