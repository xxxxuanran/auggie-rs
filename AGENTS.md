# Repository Guidelines

## Project Structure & Module Organization

- `Cargo.toml` / `Cargo.lock` define the Rust package (`auggie`, edition 2021).
- `src/main.rs` is the CLI entrypoint (clap) and switches between CLI and MCP server mode (`--mcp`).
- `src/api/` contains the Augment HTTP client, endpoint wrappers, and request/response types.
- `src/mcp/` contains the MCP server implementation and tool routing (rmcp).
- `src/workspace/` implements workspace scanning, caching, and upload coordination.
- `src/startup/` performs startup validation/“ensure” checks used in MCP mode.
- Tests are primarily unit tests colocated in `src/**` (there is no top-level `tests/` directory).

## Build, Test, and Development Commands

- `cargo build` / `cargo build --release`: build debug/release binaries.
- `cargo test`: run all tests; `cargo test <name>` filters (example: `cargo test oauth`).
- `cargo run -- [args]`: run the CLI locally.
- `cargo run -- --mcp -w <path>`: run the MCP server over stdio with an explicit workspace root.
- `cargo fmt`: format (required before PRs).
- `cargo clippy --all-targets -- -D warnings`: lint for common mistakes.

## Coding Style & Naming Conventions

- Follow standard Rust conventions; rely on `cargo fmt` instead of manual formatting.
- Prefer `tracing` for logs and structured diagnostics (avoid `println!` for normal flow).
- Naming: `snake_case` modules/functions/files, `PascalCase` types/traits, `SCREAMING_SNAKE_CASE` constants.
- Keep changes scoped: add new flags/env vars only with help text and minimal surface area.

## Testing Guidelines

- Use Rust’s built-in test framework (`#[test]`) and keep tests deterministic (avoid real network calls).
- For filesystem-heavy code, prefer temp directories/files via `tempfile`.

## Commit & Pull Request Guidelines

- Commit subjects follow the repo’s existing prefix style (examples from history: `Feat: …`, `Refactor: …`).
- PRs should include: what/why, how to test (exact commands), and notes on user-visible behavior changes.
- If a change touches auth/session storage or telemetry, call it out explicitly in the PR description.

## Security & Configuration Tips

- Never commit credentials or local state under `~/.augment/` (session and blob caches).
- Prefer environment variables for local testing (examples: `AUGMENT_SESSION_AUTH`, `AUGMENT_API_TOKEN`, `AUGMENT_API_URL`).
