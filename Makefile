# Detect host target for the native Rust binary
UNAME_S := $(shell uname -s)
UNAME_M := $(shell uname -m)
ifeq ($(UNAME_S),Darwin)
  ifeq ($(UNAME_M),arm64)
    NATIVE_TARGET := aarch64-apple-darwin
  else
    NATIVE_TARGET := x86_64-apple-darwin
  endif
else
  ifeq ($(UNAME_M),aarch64)
    NATIVE_TARGET := aarch64-unknown-linux-gnu
  else
    NATIVE_TARGET := x86_64-unknown-linux-gnu
  endif
endif

BUN        := bun
CARGO      := cargo
DOCKER     := $(shell if command -v podman >/dev/null 2>&1; then echo podman; elif command -v docker >/dev/null 2>&1; then echo docker; else echo docker; fi)
COMPOSE    := $(shell if command -v podman-compose >/dev/null 2>&1; then echo podman-compose; elif command -v podman >/dev/null 2>&1; then echo "podman compose"; elif command -v docker >/dev/null 2>&1; then echo "docker compose"; else echo "docker compose"; fi)
FRONTEND   := packages/frontend
CLI_CRATE  := tokscale-cli

.DEFAULT_GOAL := help

# ── Help ──────────────────────────────────────────────────────────────────────

.PHONY: help
help:
	@printf "\nUsage: make <target>\n"
	@grep -E '^(## .+|[a-zA-Z0-9_/-]+:.*##.*)$$' $(MAKEFILE_LIST) | \
	  awk -F':.*##' \
	    '/^## /{printf "\n\033[1m%s\033[0m\n", substr($$0,4); next} \
	    {printf "  \033[36m%-26s\033[0m %s\n", $$1, $$2}'
	@echo ""

# ── Install ────────────────────────────────────────────────────────────────────

## Install

.PHONY: install
install: install/js install/rs  ## Install all dependencies (JS + Rust)

.PHONY: install/js
install/js:  ## Install JS workspace dependencies
	$(BUN) install --frozen-lockfile

.PHONY: install/rs
install/rs:  ## Fetch and cache Rust crate dependencies
	$(CARGO) fetch

# ── Build ─────────────────────────────────────────────────────────────────────

## Build

.PHONY: build
build: build/cli build/frontend  ## Build CLI binary and Next.js frontend

.PHONY: build/cli
build/cli:  ## Build the tokscale CLI binary (release, native target)
	$(CARGO) build --release -p $(CLI_CRATE) --target $(NATIVE_TARGET)
	@echo "Binary: target/$(NATIVE_TARGET)/release/tokscale"

.PHONY: build/cli/debug
build/cli/debug:  ## Build the tokscale CLI binary (debug, fast)
	$(CARGO) build -p $(CLI_CRATE)

.PHONY: build/frontend
build/frontend: install/js  ## Build the Next.js frontend (production)
	$(BUN) run --cwd $(FRONTEND) build

.PHONY: build/cli/js
build/cli/js: install/js  ## Build the JS wrapper CLI package
	$(BUN) run build:cli

# ── Dev ───────────────────────────────────────────────────────────────────────

## Dev

.PHONY: dev
dev: dev/frontend  ## Start the frontend dev server

.PHONY: dev/frontend
dev/frontend: install/js  ## Start Next.js dev server (localhost:3000)
	$(BUN) run --cwd $(FRONTEND) dev

.PHONY: dev/bench
dev/bench: install/js  ## Run benchmarks against live data
	$(BUN) run --cwd packages/benchmarks run

.PHONY: dev/bench/synthetic
dev/bench/synthetic: install/js  ## Run synthetic benchmarks (no real data needed)
	$(BUN) run --cwd packages/benchmarks run:synthetic

.PHONY: run
run:  ## Run the tokscale TUI (native binary must be built first)
	./target/$(NATIVE_TARGET)/release/tokscale

.PHONY: run/dev
run/dev:  ## Run the tokscale CLI via bun (no build needed)
	bash scripts/cli.sh

# ── Test ──────────────────────────────────────────────────────────────────────

## Test

.PHONY: test
test: test/rs test/frontend  ## Run all tests (Rust + frontend)

.PHONY: test/rs
test/rs:  ## Run Rust workspace tests
	$(CARGO) test --workspace --all-features

.PHONY: test/frontend
test/frontend: install/js  ## Run frontend Vitest tests
	$(BUN) run --cwd $(FRONTEND) test

.PHONY: test/launchers
test/launchers: install/js  ## Smoke-test npm package launchers
	bash scripts/test-package-launchers.sh

.PHONY: test/scripts
test/scripts:  ## Run release script unit tests
	bash scripts/test-calculate-release-version.sh
	bash scripts/test-check-version-coherence.sh
	bash scripts/test-npm-release-state.sh
	bash scripts/test-prepare-release-provenance.sh
	bash scripts/test-release-workflow-safety.sh

# ── Lint / Format ─────────────────────────────────────────────────────────────

## Lint & Format

.PHONY: lint
lint: lint/rs lint/frontend  ## Lint all code

.PHONY: lint/rs
lint/rs:  ## Run cargo clippy (strict: -D warnings)
	$(CARGO) clippy --locked --workspace --all-features -- -D warnings

.PHONY: lint/frontend
lint/frontend: install/js  ## Run ESLint on the frontend
	$(BUN) run --cwd $(FRONTEND) lint

.PHONY: fmt
fmt: fmt/rs  ## Format all code

.PHONY: fmt/rs
fmt/rs:  ## Run cargo fmt across the workspace
	$(CARGO) fmt --all

.PHONY: fmt/rs/check
fmt/rs/check:  ## Check Rust formatting without modifying files
	$(CARGO) fmt --all -- --check

.PHONY: typecheck
typecheck: install/js  ## Run TypeScript type-check on the frontend
	$(BUN) run --cwd $(FRONTEND) typecheck

.PHONY: check/versions
check/versions:  ## Verify version coherence across manifests
	bash scripts/check-version-coherence.sh

# ── Database ──────────────────────────────────────────────────────────────────

## Database

.PHONY: db/migrate
db/migrate: install/js  ## Apply pending Drizzle migrations (requires DATABASE_URL)
	$(BUN) run --cwd $(FRONTEND) db:migrate

.PHONY: db/generate
db/generate: install/js  ## Generate a new Drizzle migration from schema changes
	$(BUN) run --cwd $(FRONTEND) db:generate

.PHONY: db/push
db/push: install/js  ## Push schema directly to DB without a migration file (dev only)
	$(BUN) run --cwd $(FRONTEND) db:push

.PHONY: db/studio
db/studio: install/js  ## Open Drizzle Studio (DB browser UI)
	$(BUN) run --cwd $(FRONTEND) db:studio

.PHONY: db/seed
db/seed: install/js  ## Seed the dev database with sample data
	$(BUN) run --cwd $(FRONTEND) scripts/seed-dev.ts

# ── Coverage ──────────────────────────────────────────────────────────────────

## Coverage

.PHONY: coverage
coverage:  ## Run Rust tests and generate coverage report (requires cargo-tarpaulin)
	$(CARGO) tarpaulin --workspace --all-features --out Html Xml --output-dir target/coverage --timeout 300
	@echo "Report: target/coverage/tarpaulin-report.html"

# ── Docker ────────────────────────────────────────────────────────────────────

## Docker

.PHONY: docker/build
docker/build:  ## Build the Docker image for the frontend
	$(DOCKER) build \
	  --build-arg DATABASE_URL=$${DATABASE_URL:-postgresql://tokscale:tokscale@localhost:5432/tokscale} \
	  -t tokscale:latest .

.PHONY: tui
tui:  ## Run the tokscale TUI in a container (builds image if needed)
	$(COMPOSE) --profile tui run --rm tui

.PHONY: tui/build
tui/build:  ## Build only the TUI container image
	$(DOCKER) build -f Dockerfile.tui -t tokscale-tui:latest .

.PHONY: up
up:  ## Start all services (db + app) via docker compose
	$(COMPOSE) up --build -d
	@echo "App: http://localhost:3333"

.PHONY: up/db
up/db:  ## Start only the database service
	$(COMPOSE) up db -d
	@echo "Postgres: localhost:5432 (db=tokscale user=tokscale)"

.PHONY: down
down:  ## Stop all docker compose services
	$(COMPOSE) down

.PHONY: down/volumes
down/volumes:  ## Stop all services and delete volumes (destroys DB data)
	$(COMPOSE) down -v

.PHONY: logs
logs:  ## Tail logs from all compose services
	$(COMPOSE) logs -f

.PHONY: logs/app
logs/app:  ## Tail logs from the app container only
	$(COMPOSE) logs -f app

.PHONY: ps
ps:  ## Show status of compose services
	$(COMPOSE) ps

# ── Clean ─────────────────────────────────────────────────────────────────────

## Clean

.PHONY: clean
clean: clean/rs clean/frontend  ## Remove all build artifacts

.PHONY: clean/rs
clean/rs:  ## Remove Rust build artifacts
	$(CARGO) clean

.PHONY: clean/frontend
clean/frontend:  ## Remove Next.js build output
	rm -rf $(FRONTEND)/.next

.PHONY: clean/docker
clean/docker:  ## Remove tokscale Docker images
	$(DOCKER) rmi tokscale:latest 2>/dev/null || true
