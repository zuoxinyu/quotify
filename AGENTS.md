# Repository Guidelines

## Project Structure & Module Organization
`src/main.rs` is the CLI and tray entry point. UI rendering lives in `src/app.rs`, configuration loading in `src/config.rs`, cookie helpers in `src/cookies.rs`, tray icon generation in `src/icon.rs`, and provider integrations under `src/provider/` (`claude.rs`, `codex.rs`, `gemini.rs`, etc.). Keep new provider-specific logic inside `src/provider/<name>.rs` and register it through `src/provider/mod.rs` and `create_provider()` in `src/main.rs`. Use `config.example.toml` as the reference shape for local configuration.

## Build, Test, and Development Commands
Use Cargo for all local workflows:

- `cargo check` validates the project quickly without producing a release binary.
- `cargo run -- fetch` fetches quota data and prints JSON for enabled providers.
- `cargo run -- tray` starts the Windows tray app and detail window.
- `cargo run -- init` writes the default config file under the platform config directory.
- `cargo fmt` applies standard Rust formatting.
- `cargo clippy --all-targets --all-features` catches style and correctness issues before review.
- `cargo build --release` produces the optimized binary in `target/release/`.

## Coding Style & Naming Conventions
Follow Rust defaults: 4-space indentation, `snake_case` for functions/modules, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for statics and constants. Prefer small modules with clear ownership rather than expanding `main.rs`. Use `anyhow::Result` for fallible app flows and keep provider error messages actionable because they surface in the UI. Always run `cargo fmt` before submitting changes.

## Testing Guidelines
There is no `tests/` directory yet, so add focused unit tests alongside the code with `#[cfg(test)]` when you touch parsing, config loading, or provider normalization logic. Run `cargo test` locally before opening a PR. For network-backed providers, prefer deterministic parser tests over live API calls.

## Commit & Pull Request Guidelines
This repository currently has no commit history, so use short imperative commit subjects such as `Add Gemini quota reset parsing`. Keep each commit scoped to one change. PRs should include a concise summary, manual verification steps, and screenshots for tray/UI changes. Link related issues and note any config or credential assumptions reviewers need to reproduce the change.

## Configuration & Security Tips
Do not commit real API keys, auth files, or exported usage data. Keep secrets in the generated config file or environment variables such as `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, and `OPENCODE_API_KEY`.
