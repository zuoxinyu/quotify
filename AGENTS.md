# Repository Guidelines

## Project Overview
Windows system tray AI provider quota monitor. **Windows-only** — uses Win32 APIs (`windows` crate), DWM Mica backdrop, and `eframe`/`egui` for the popup UI. Building on non-Windows will fail.

## Structure
- `src/main.rs` — CLI + tray entry point; `create_provider()` wires providers; default subcommand is `Tray`
- `src/app.rs` — `eframe::App` impl with Fluent/Mica styling
- `src/config.rs` — TOML config, stored at platform config dir (`AppData/Roaming/quotify/quotify.toml`)
- `src/tray.rs` — Win32 tray icon, message loop, custom window subclass
- `src/icon.rs` — Dynamic HICON generation (usage dots)
- `src/provider/` — `Provider` trait + one file per provider (`claude.rs`, `codex.rs`, `gemini.rs`, `deepseek.rs`, `opencode.rs`, `mimo.rs`, `antigravity.rs`)

## Adding a Provider
Two registration points — miss either and the provider is silently ignored:
1. Add `src/provider/<name>.rs` implementing `Provider` trait, add `pub mod <name>;` to `src/provider/mod.rs`
2. Add a match arm in `create_provider()` in `src/main.rs` **and** add the name to `PROVIDER_ORDER` (controls tray icon dot order)

## Commands
- `cargo check` — quick validation
- `cargo run -- fetch` — fetch quota JSON for enabled providers
- `cargo run -- fetch --provider gemini` — fetch one provider
- `cargo run -- tray` — start tray app (default if no subcommand)
- `cargo run -- init` — write default config
- `cargo fmt` — format
- `cargo clippy --all-targets --all-features` — lint
- `cargo test` — tests (no `tests/` dir; unit tests inline with `#[cfg(test)]`)
- `cargo build --release` — optimized binary (`opt-level = "z"`, LTO, strip)

## Key Conventions
- Rust **edition 2024** — requires Rust ≥ 1.85
- `anyhow::Result` for all fallible flows; provider errors surface in UI, so keep messages actionable
- Use `parking_lot` for locks (not `std::sync`)
- `async_trait` for `Provider` trait; providers are `Send + Sync`
- Follow existing style in `src/app.rs` for UI: semi-transparent fills, rounded corners (12px window, 8px cards), `Segoe UI Variable` font

## Architecture Notes
- Three threads: Win32 message loop (main), background fetch (tokio runtime), eframe UI window
- Tray icon is the entry point; popup window starts offscreen and animates in via Win32 `SetWindowPos`
- Config auto-creates with defaults on first load; secrets go in config file or env vars
- `[network].proxy` is explicit-only and supports `http://`, `https://`, and `socks5://`; clients ignore ambient proxy env/system settings by default
- Provider auth varies: API keys (deepseek, gemini), auth files (claude, codex), Antigravity OAuth credentials, explicit cookie values from config/env (opencode, mimo)

## Testing
Add focused unit tests alongside code when touching parsing, config, or provider logic. Prefer deterministic parser tests over live API calls. No integration test infrastructure yet.

## Security
Never commit API keys, auth files, or usage data. Supported env vars: `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `ANTIGRAVITY_API_KEY`, `ANTIGRAVITY_OAUTH_CREDENTIALS_JSON`, `ANTIGRAVITY_OAUTH_CLIENT_ID`, `ANTIGRAVITY_OAUTH_CLIENT_SECRET`, `OPENCODE_WORKSPACE_ID`, `OPENCODE_AUTH_COOKIE`, `MIMO_SERVICE_TOKEN`, `MIMO_COOKIE_HEADER`, `CLAUDE_SESSION_KEY`, `CLAUDE_ACCESS_TOKEN`, `ANTHROPIC_ADMIN_KEY`, `ANTHROPIC_API_KEY`.
