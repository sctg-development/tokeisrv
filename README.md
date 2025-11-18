# tokeisrv — Tokei HTTP Badge Service

A small HTTP service exposing Tokei statistics (lines of code, comments, blanks, etc.) as SVG badges. The service uses `tokei` to compute language statistics, `actix-web` to expose endpoints, and `rsbadges` to generate SVG badge images.

This project is an adaptation of XAMPPRocky's tokei web badge server; modifications and maintenance are by Ronan Le Meillat (SCTG Development). Code is licensed under the MIT license — see source headers.

---

## Features ✅

- Serves SVG badges for repository language statistics (lines, code, files, comments, blanks)
- Badge customization: color, style, label, logo, and language ranking
- Cache remote repository stats for faster responses (`cached` crate)
- CLI args and environment variables for server configuration
- Verbose logs by default, quiet mode via `-q`/`--quiet`

---

## Getting started — build & run 🚀

Prerequisites:
- Rust toolchain (stable)
- `git` available on PATH

Build:

```bash
cargo build --release
```

Run with default settings (bind 0.0.0.0:8000):

```bash
cargo run --release --
```

Run with custom bind/port and quiet flag (CLI has precedence over env vars):

```bash
cargo run --release -- --bind 127.0.0.1 --port 8080 -q
```

You can also use environment variables instead of CLI options:

```bash
export TOKEI_BIND=127.0.0.1
export TOKEI_PORT=8080
cargo run --release --
```

Notes:
- CLI options take precedence over environment variables.
- Default behavior: verbose logs (RUST_LOG defaults to `debug` when unset). Use `-q` or `--quiet` to silence logs.

---

## API endpoints and usage 🛠️

Main endpoint (badge generator):

- GET /b1/{domain}/{user}/{repo}

Examples:

```bash
# Default: show badge for lines
curl "http://127.0.0.1:8000/b1/github.com/XAMPPRocky/tokei"

# Show code lines as a badge
curl "http://127.0.0.1:8000/b1/github.com/XAMPPRocky/tokei?category=code"

# Show top language ranking
curl "http://127.0.0.1:8000/b1/github.com/XAMPPRocky/tokei?show_language=true"

# Use branch override
curl "http://127.0.0.1:8000/b1/github.com/XAMPPRocky/tokei?branch=main"

# Generate JSON instead of SVG
curl -H "Accept: application/json" "http://127.0.0.1:8000/b1/github.com/XAMPPRocky/tokei"
```

Query parameters details:
- `category`: `lines` (default), `code`, `blanks`, `comments`, `files`
- `label`: custom left-side label
- `style`: `plastic`, `flat`, `flat-square`, `for-the-badge`, `social`
- `color`: custom hex color for the message side of the badge
- `logo`: badge logo name (if supported by `rsbadges`)
- `type`: filter which language types are considered (comma-separated)
- `show_language`: Boolean (`true`/`false`) to display top language name on the badge
- `language_rank`: choose index for ranking language
- `branch`: choose repository branch to analyze

---

## Caching behavior 🧠

This service uses the `cached` crate to cache computation results. Default configuration:

- Cache store: `TimedSizedCache` (size = 1000 entries)
- Lifespan: 1 day (24 hours)

Etag headers and `If-None-Match` are supported by the service; cached responses will return 304 Not Modified when appropriate.

Note: updating `cached` from 0.55 to 0.56 requires a Duration type for TTL — the repo uses `std::time::Duration::from_secs(DAY_IN_SECONDS)`.

---

## Logging

- Default: logs are verbose (RUST_LOG defaults to `debug` when not set)
- To reduce log output: pass `-q` / `--quiet` to the binary
- You can still use `RUST_LOG` to control specific logging levels if `-q` is not used

Example:

```bash
# Use an environment variable for detailed filtering
RUST_LOG=actix_web=info,target=debug cargo run --release --
```

---

## Security & limitations ⚠️

- The service clones remote repositories to a temporary directory — ensure you trust the sources you allow or limit access.
- The `git` command must be available in the environment where the service runs.

---

## Contributing 👩‍💻👨‍💻

Please open issues or pull requests. If you're planning larger changes, create an issue first so we can coordinate design.

---

## License

This software is provided under the MIT license. See comments in source files for details.

---

If you want, I can add a short `Makefile` or an example `systemd` service file for deployment. Which would you like next?