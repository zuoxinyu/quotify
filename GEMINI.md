# Quotify: Windows AI Quota Monitor

Quotify is a Windows system tray application designed to monitor usage quotas across multiple AI providers (Claude, Gemini, DeepSeek, etc.). It provides a glanceable tray icon and a detailed popup window showing remaining credits and usage windows.

## Project Overview

- **Technologies:** Rust (2024 edition), `tokio` (async), `eframe`/`egui` (GUI), `windows-rs` (Win32 API), `reqwest` (HTTP), `serde` (JSON/TOML).
- **Target OS:** Windows (uses Win32 specific APIs for tray and Mica backdrop effects).

### Key Components

- **`src/main.rs`**: Entry point for both CLI and Tray modes.
- **`src/app.rs`**: The `egui` application logic and Fluent-styled UI rendering.
- **`src/provider/`**: Abstractions and implementations for various AI providers.
    - `mod.rs`: Defines the `Provider` trait and `UsageData` structures.
    - `claude.rs`, `gemini.rs`, `deepseek.rs`, etc.: Provider-specific fetch logic.
- **`src/tray.rs`**: Low-level Win32 tray icon management and message loop.
- **`src/icon.rs`**: Dynamic HICON generation (e.g., drawing usage dots on the icon).
- **`src/config.rs`**: TOML configuration management.

## Building and Running

### Prerequisites
- Rust (latest stable)
- Windows 10/11 (for full functionality)

### Commands
- **Run Tray App:** `cargo run -- tray`
- **CLI Fetch (JSON output):** `cargo run -- fetch`
- **Initialize Config:** `cargo run -- init`
- **Formatting:** `cargo fmt`
- **Linting:** `cargo clippy --all-targets --all-features`
- **Release Build:** `cargo build --release`
- **Testing:** `cargo test`

## Development Conventions

- **Module Organization:** Keep provider-specific logic within `src/provider/<name>.rs`. Register new providers in `src/provider/mod.rs` and the `create_provider` function in `main.rs`.
- **Error Handling:** Use `anyhow::Result` for application-level flows. Ensure provider error messages are user-friendly as they are displayed in the UI.
- **UI Styling:** Follow the "Fluent/Mica" aesthetic in `src/app.rs`. Use semi-transparent colors and rounded corners (12px for windows, 8px for cards).
- **Configuration:** Reference `config.example.toml` for the structure. Configuration is stored in the standard platform config directory (e.g., `AppData/Roaming`).
- **Secrets:** Never commit API keys or credentials. Use the config file or environment variables (`GEMINI_API_KEY`, etc.).

## Testing Guidelines
- Add unit tests within modules using `#[cfg(test)]`.
- Focus on parsing logic and configuration loading.
- Prefer mocking or deterministic parser tests over live network calls for providers.
