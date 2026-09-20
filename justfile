set windows-shell := ["pwsh", "-NoProfile", "-Command"]

# Default recipe: display available commands
default:
    @just --list

# Format all Rust code across the workspace
format:
    cargo fmt

# Run Clippy linter with warnings treated as errors
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Run all checks (formatting and linting)
check:
    cargo fmt --check
    just lint

# Install the pre-commit hook into .git/hooks (Unix)
[unix]
setup:
    mkdir -p .git/hooks
    cp .githooks/pre-commit .git/hooks/pre-commit
    @echo "Pre-commit hook installed to .git/hooks/pre-commit"

# Install the pre-commit hook into .git/hooks (Windows)
[windows]
setup:
    New-Item -ItemType Directory -Force -Path .git/hooks | Out-Null
    Copy-Item -Force .githooks/pre-commit .git/hooks/pre-commit
    Write-Host "Pre-commit hook installed to .git/hooks/pre-commit"

# Run all test suites
test:
    cargo test
