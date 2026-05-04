.DEFAULT_GOAL := help

CARGO ?= cargo
QUALITY_SCRIPT := ./scripts/quality.sh
INSTALL_HOOKS_SCRIPT := ./scripts/install-hooks.sh
CMD ?=

.PHONY: help setup hooks quality quality-commit quality-push fmt check test clippy run run-command install clean release

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
	@$(CARGO) fmt --check

check: ## Check compilation
	@$(CARGO) check --locked

test: ## Run tests
	@$(CARGO) test --locked

clippy: ## Run clippy with warnings denied
	@$(CARGO) clippy --locked --all-targets -- -D warnings

run: ## Run wayinhibit in foreground mode
	@$(CARGO) run

run-command: ## Run a command under inhibition, pass CMD="sleep 10"
	@if [ -z "$(CMD)" ]; then \
		printf '%s\n' 'Usage: make run-command CMD="sleep 10"' >&2; \
		exit 2; \
	fi
	@$(CARGO) run -- -- $(CMD)

install: ## Install the binary locally with cargo using Cargo.lock
	@$(CARGO) install --locked --path .

clean: ## Remove Cargo build artifacts
	@$(CARGO) clean

release: ## Release a new version: make release VERSION=x.y.z
	@if [ -z "$(VERSION)" ]; then \
		printf '%s\n' 'Usage: make release VERSION=x.y.z' >&2; \
		exit 2; \
	fi
	@./scripts/release.sh "$(VERSION)"
