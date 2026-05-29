# Quotify

Quotify is a small Windows tray app for checking AI provider quota usage.

It shows a compact flyout with provider status, reset times, and a tray icon that can reflect a primary provider. The app is Windows-only and uses Win32 APIs with an `egui` popup UI.

Quotify is inspired by [CodexBar](https://github.com/steipete/CodexBar), especially its practical approach to surfacing provider usage in a lightweight desktop utility.

## Providers

Supported providers:

- Codex
- OpenCode
- Claude
- Gemini
- Antigravity
- DeepSeek
- MiMo

Authentication is intentionally explicit. Browser cookie scraping is not used. Configure credentials through `quotify.toml` or environment variables.

## Usage

Create a default config:

```powershell
cargo run -- init
```

Fetch provider usage once:

```powershell
cargo run -- fetch
cargo run -- fetch --provider claude
```

Run the tray app:

```powershell
cargo run -- tray
```

Build an optimized release binary:

```powershell
cargo build --release
```

## Configuration

The default config path is:

```text
%APPDATA%\quotify\quotify.toml
```

See `config.example.toml` for available fields. Common environment variables include `OPENCODE_AUTH_COOKIE`, `OPENCODE_WORKSPACE_ID`, `CLAUDE_ACCESS_TOKEN`, `CLAUDE_SESSION_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `DEEPSEEK_API_KEY`, `MIMO_SERVICE_TOKEN`, and `MIMO_COOKIE_HEADER`.

For explicit network proxying, set `[network].proxy` to an HTTP or SOCKS5 URL, for example `http://127.0.0.1:7890` or `socks5://127.0.0.1:7890`.

## License

MIT. See `LICENSE`.
