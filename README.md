![](https://tokeisrv.sctg.eu.org/b1/github/sctg-development/tokeisrv?type=rust&category=code)
![](https://tokeisrv.sctg.eu.org/b1/github/sctg-development/tokeisrv?type=rust&category=comments)
# tokeisrv — Tokei HTTP Badge Service

A small HTTP service exposing Tokei statistics (lines of code, comments, blanks, etc.) as SVG badges. The service uses `tokei` to compute language statistics, `actix-web` to expose endpoints, and `rsbadges` to generate SVG badge images.

This project is an adaptation of XAMPPRocky's tokei web badge server; modifications and maintenance are by Ronan Le Meillat (SCTG Development). Code is licensed under the MIT license — see source headers.

TL;DR — Quick deployment
------------------------
use MarkDown code like
```text
![](https://tokeisrv.example.com/b1/github/sctg-development/tokeisrv?type=rust&category=code)
![](https://tokeisrv.example.com/b1/github/sctg-development/tokeisrv?type=rust&category=comments)
```
Want to deploy your own instance quickly? See the short step-by-step guide: [Deploy your own service using Docker Compose](./deploy-your-own-service.md).
It covers creating a free `.pp.ua` domain, configuring Cloudflare and Cloudflare Tunnel (cloudflared), generating credentials, running `docker compose`, and example badge usage.

---

## Features ✅

- Serves SVG badges for repository language statistics (lines, code, files, comments, blanks)
- Badge customization: color, style, label, logo, and language ranking
- Cache remote repository stats for faster responses (`cached` crate) with configurable TTL and size (`--cache-ttl`, `--cache-size`)
- CLI args and environment variables for server configuration
- Verbose logs by default, quiet mode via `-q`/`--quiet`
- Optional user whitelist to limit which repository owners can be cloned (`--user-whitelist`)
- Optional git server whitelist to restrict allowed domain hosts for repo cloning (`--gitserver-whitelist`)
- No git dependencies at runtime

---

## Getting started — build & run 🚀

Prerequisites:
- Rust toolchain (stable)

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

# Adjust cache size
To control the maximum number of cached entries, provide `--cache-size`. Default is 1000 entries.

```bash
cargo run --release -- --cache-size 2048
```
```

You can also use environment variables instead of CLI options:

```bash
export TOKEI_BIND=127.0.0.1
export TOKEI_PORT=8080
cargo run --release --
```

Notes:
- CLI options take precedence over environment variables.
- Default behavior: verbose logs (RUST_LOG defaults to `info` when unset). Use `-q` or `--quiet` to silence logs.

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

This service uses an in-memory LRU cache to store repository statistics for faster responses. Each cached entry stores:

- Timestamp when the stats were added to the cache
- Full commit SHA (40 hex characters)
- SHA256 hash of the requested URL (useful for deduplication)
- JSON summary of top-level counts (Lines, Code, Comments, Blanks)
- The full tokei `Language` vector used to render badges

Default behavior:

- Number of cached entries (TimedSizedCache size): 1000 entries (default)
- TTL for cached entries: 1 day (24 hours) (default)
- You can change the maximum number of cached entries with the CLI flag `--cache-size` or the environment variable `TOKEI_CACHE_SIZE`.
- Cache TTL can be overridden with CLI flag `--cache-ttl` (seconds) or the environment variable `TOKEI_CACHE_TTL`. CLI takes precedence.
 - You can restrict which git servers can be queried using `--gitserver-whitelist` or environment variable `TOKEI_GITSERVER_WHITELIST`. If this list is empty, all servers are permitted.
- The cache follows an LRU (least recently used) policy when space is needed and evicts oldest entries first.

Notes:

- If the git SHA hasn't changed, the repository is not recloned and the badge is generated from the cached result.
- If the cache is full, the least recently used entry is evicted to make room.

Etag headers and `If-None-Match` are supported by the service; cached responses will return 304 Not Modified when appropriate.

Note: updating `cached` from 0.55 to 0.56 requires a Duration type for TTL — the repo uses `std::time::Duration::from_secs(DAY_IN_SECONDS)`.

---

## Logging

- Default: logs are verbose (RUST_LOG defaults to `info` when not set)
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

User whitelist (optional, recommended for security)
-----------------------------------------------
You can optionally restrict which repository owners the service is permitted to analyze. This prevents the server from cloning arbitrary repositories and reduces attack surface.

How it works:
- Provide a comma-separated list of allowed usernames (e.g., `alice,bob`). If the list is empty (default), all requests are permitted.
- Whitelist can be set using the CLI flag `--user-whitelist` or the environment variable `TOKEI_USER_WHITELIST`.
- When a request targets a repository whose owner is not listed, the server returns a red `forbidden` SVG badge (HTTP 403) instead of cloning the repo.

Notes on formatting and behavior:
- The whitelist expects a comma-separated list of usernames with no surrounding spaces (whitespace is trimmed). Empty entries are not allowed.
- Username matching is exact and case-sensitive. If you want case-insensitive matching, normalize usernames to lowercase before passing them in.
- The server logs a warning for requests blocked by the whitelist for monitoring/auditing purposes.

Example usage
-------------
Using the CLI:
```bash
./tokei_rs --user-whitelist alice,bob --bind 0.0.0.0 --port 8000
```

Using environment variables:
```bash
export TOKEI_USER_WHITELIST="alice,bob"
./tokei_rs
```

Using Docker (passing env to container):
```bash
docker run -e TOKEI_USER_WHITELIST="alice,bob" -p 8000:8000 sctg/tokeisrv:latest
```

Using Helm (chart value):
```bash
helm install tokeisrv helm/tokeisrv --set userWhitelist='alice,bob'
```

This feature should be used when you want to operate the service in a managed environment or expose it publicly — it helps ensure only repositories from trusted owners are processed.

Git server whitelist (optional, recommended for security)
------------------------------------------------------
You can optionally restrict which git servers (remote domains) the service is permitted to contact. This prevents the service from being used as a proxy to reach arbitrary hosts and reduces attack surface.

How it works:
- Provide a comma-separated list of allowed domain names (e.g., `github.com,gitlab.com`). If the list is empty (default), all servers are permitted.
- Whitelist can be set using the CLI flag `--gitserver-whitelist` or the environment variable `TOKEI_GITSERVER_WHITELIST`.
- When a request targets a repository whose git server domain is not listed, the server returns a red `forbidden` SVG badge (HTTP 403) instead of making the request.

Notes on formatting and behavior:
- The whitelist expects a comma-separated list of hostnames with no surrounding spaces (whitespace is trimmed). Empty entries are not allowed.
- Domain matching is exact and case-sensitive. For best results, provide fully-qualified domain names (e.g., `github.com`) and normalize to lowercase if you want to avoid mismatches.

Example usage
-------------
Using the CLI:
```bash
./tokei_rs --gitserver-whitelist github.com,gitlab.com --bind 0.0.0.0 --port 8000
```

Using environment variables:
```bash
export TOKEI_GITSERVER_WHITELIST="github.com,gitlab.com"
./tokei_rs
```

Using Docker (passing env to container):
```bash
docker run -e TOKEI_GITSERVER_WHITELIST="github.com,gitlab.com" -p 8000:8000 sctg/tokeisrv:latest
```

Using Helm (chart value):
```bash
helm install tokeisrv helm/tokeisrv --set gitServerWhitelist='github.com,gitlab.com'
```

This should be used when you want to tightly control which remote git servers are accessed by the service, such as in corporate environments.

---
## Supported Languages

Those language are supported
```
Abap
ActionScript
Ada
Agda
Alex
Alloy
APL
Asn1
Asp
AspNet
Assembly
AssemblyGAS
ATS
Autoconf
AutoHotKey
Automake
AWK
Bash
Batch
Bazel
Bean
Bicep
Bitbake
BQN
BrightScript
C
Cabal
Cassius
Ceylon
CHeader
Cil
Clojure
ClojureC
ClojureScript
CMake
Cobol
CoffeeScript
Cogent
ColdFusion
ColdFusionScript
Coq
Cpp
CppHeader
Crystal
CSharp
CShell
Css
Cuda
CUE
Cython
D
D2
DAML
Dart
DeviceTree
Dhall
Dockerfile
DotNetResource
DreamMaker
Dust
Ebuild
EdgeDB
Edn
Elisp
Elixir
Elm
Elvish
EmacsDevEnv
Emojicode
Erlang
Factor
FEN
Fish
FlatBuffers
ForgeConfig
Forth
FortranLegacy
FortranModern
FreeMarker
FSharp
Fstar
GDB
GdScript
GdShader
Gherkin
Gleam
Glsl
Go
Graphql
Groovy
Gwion
Hamlet
Handlebars
Happy
Hare
Haskell
Haxe
Hcl
Hex
Hex0
Hex1
Hex2
HiCAD
hledger
Hlsl
HolyC
Html
Hy
Idris
Ini
IntelHex
Isabelle
Jai
Janet
Java
JavaScript
Jq
Json
Jsx
Julia
Julius
Just
KakouneScript
KaemFile
Kotlin
Lean
Less
Lingua Franca
LinkerScript
Liquid
Lisp
LLVM
Logtalk
Lua
Lucius
M1Assembly
Madlang
Max
Makefile
Markdown
Mdx
Meson
Mint
Mlatu
ModuleDef
MonkeyC
MoonScript
MsBuild
Mustache
Nim
Nix
NotQuitePerl
NuGetConfig
Nushell
ObjectiveC
ObjectiveCpp
OCaml
Odin
OpenSCAD
OpenQASM
Org
Oz
Pascal
Perl
Perl6
Pest
Phix
Php
Po
Poke
Polly
Pony
PostCss
PowerShell
Processing
Prolog
Protobuf
PRQL
PSL
PureScript
Pyret
Python
Qcl
Qml
R
Racket
Rakefile
Razor
Renpy
ReStructuredText
RON
RPMSpecfile
Ruby
RubyHtml
Rust
Sass
Scala
Scheme
Scons
Sh
ShaderLab
Slang
Sml
Solidity
SpecmanE
Spice
Sql
SRecode
Stata
Stratego
Svelte
Svg
Swift
Swig
SystemVerilog
Slint
Tact
Tcl
Templ
Tex
Text
Thrift
Toml
Tsx
Twig
TypeScript
UMPL
UnrealDeveloperMarkdown
UnrealPlugin
UnrealProject
UnrealScript
UnrealShader
UnrealShaderHeader
UrWeb
UrWebProject
Vala
VB6
VBScript
Velocity
Verilog
VerilogArgsFile
Vhdl
VimScript
VisualBasic
VisualStudioProject
VisualStudioSolution
Vue
WebAssembly
Wolfram
Xaml
XcodeConfig
Xml
XSL
Xtend
Yaml
ZenCode
Zig
ZoKrates
Zsh
```
## Contributing 👩‍💻👨‍💻

Please open issues or pull requests. If you're planning larger changes, create an issue first so we can coordinate design.

---

## License

This software is provided under the MIT license. See comments in source files for details.
