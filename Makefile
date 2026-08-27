.DEFAULT_GOAL := help

QUALITY_SCRIPT := ./scripts/quality.sh
INSTALL_HOOKS_SCRIPT := ./scripts/install-hooks.sh
E2E_SCRIPT := ./scripts/e2e-test.sh
CMD ?=

.PHONY: help setup hooks quality quality-commit quality-push fmt check test clippy e2e run run-command install clean release

help: ## Show available targets
	@printf '%s\n' 'Available targets:'
	@grep -E '^[a-zA-Z0-9_-]+:.*## ' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*## "}; {printf "  %-16s %s\n", $$1, $$2}'

setup: hooks ## Bootstrap the local repository setup

hooks: ## Install versioned Git hooks for this clone
	@$(INSTALL_HOOKS_SCRIPT)

quality: ## Run the full local quality suite
	@$(QUALITY_SCRIPT) all

quality-commit: ## Run the pre-commit quality checks
	@$(QUALITY_SCRIPT) pre-commit

quality-push: ## Run the pre-push quality checks
	@$(QUALITY_SCRIPT) pre-push

fmt: ## Check formatting
	@cargo fmt --check

check: ## Check compilation
	@cargo check --locked

test: ## Run tests
	@cargo test --locked

clippy: ## Run clippy with warnings denied
	@cargo clippy --locked --all-targets -- -D warnings

e2e: ## Run end-to-end tests against a real (headless) Sway; requires `sway` on PATH
	@cargo build --locked
	@$(E2E_SCRIPT) target/debug/wayinhibit

run: ## Run wayinhibit in foreground mode
	@cargo run

run-command: ## Run a command under inhibition, pass CMD="sleep 10"
	@if [ -z "$(CMD)" ]; then \
		printf '%s\n' 'Usage: make run-command CMD="sleep 10"' >&2; \
		exit 2; \
	fi
	@cargo run -- -- $(CMD)

install: ## Install the binary locally with cargo using Cargo.lock
	@cargo install --locked --path .

clean: ## Remove Cargo build artifacts
	@cargo clean

release: ## Release a new version: make release VERSION=x.y.z
	@if [ -z "$(VERSION)" ]; then \
		printf '%s\n' 'Usage: make release VERSION=x.y.z' >&2; \
		exit 2; \
	fi
	@./scripts/release.sh "$(VERSION)"
