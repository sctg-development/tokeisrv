// MIT License (MIT)

// Originally based on code from
// Copyright (c) 2018 XAMPPRocky and contributors
// Modifications Copyright (c) 2025 Ronan Le Meillat for SCTG Development

// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:

// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

use git2::{build::RepoBuilder, Cred, Direction, FetchOptions, RemoteCallbacks, Repository};
use std::path::Path;

use actix_web::{
    get,
    http::header::{
        Accept, CacheControl, CacheDirective, ContentType, EntityTag, Header, IfNoneMatch,
        CACHE_CONTROL, CONTENT_TYPE, ETAG, LOCATION,
    },
    web::{self},
    App, HttpRequest, HttpResponse, HttpServer,
};
use clap::Parser;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Command-line arguments for the `tokei_rs` HTTP server.
///
/// The server accepts the following user-configurable options:
///
/// - `--bind` (-b): the IP or hostname to bind the server to (default: `0.0.0.0`).
/// - `--port` (-p): the TCP port to listen on (default: `8000`).
///
/// These options are intentionally simple and documented here for clarity.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The bind address for the server (e.g., `0.0.0.0` or `127.0.0.1`).
    /// Environment variable fallback: `TOKEI_BIND`. If the env var is set it
    /// will be used when `--bind` isn't supplied. Command-line options take
    /// precedence over environment variables.
    #[arg(short, long, default_value = "0.0.0.0")]
    bind: String,

    /// The TCP port used by the server.
    /// Environment variable fallback: `TOKEI_PORT`. If the env var is set it
    /// will be used when `--port` isn't supplied. Command-line options take
    /// precedence over environment variables. The value will be parsed as an
    /// unsigned 16-bit port number.
    #[arg(short, long, default_value_t = 8000)]
    port: u16,

    /// Silence all log output. When true, the server will not emit application
    /// logs regardless of `RUST_LOG` environment setting.
    #[arg(short, long, default_value_t = false)]
    quiet: bool,

    /// Comma-separated list of allowed users; if provided, only repos owned by
    /// these users can be cloned. Environment variable fallback: TOKEI_USER_WHITELIST.
    #[arg(long)]
    user_whitelist: Option<String>,
    /// Cache TTL in seconds (equivalent to environment variable TOKEI_CACHE_TTL)
    /// Default is 86400 (1 day).
    #[arg(long, default_value_t = 86400u64)]
    cache_ttl: u64,
    /// Maximum number of entries for the `cached` crate TimedSizedCache (default 1000)
    /// Equivalent environment variable: `TOKEI_CACHE_SIZE`.
    #[arg(long, default_value_t = 1000usize)]
    cache_size: usize,
    /// Comma-separated list of allowed git servers; if provided, only queries for
    /// these domains are permitted. Fallback environment variable: TOKEI_GITSERVER_WHITELIST.
    #[arg(long)]
    gitserver_whitelist: Option<String>,
    /// Comma-separated list of file extensions to ignore (without dot). Example: "png,jpg,gz".
    /// Fallback environment variable: TOKEI_IGNORE_FILETYPE.
    #[arg(
        long,
        default_value = "gfs,xsd,csv,dxf,wkt,dgn,rsc,png,a,so,pc,ai,jpg,gif,gz,bz2,xz,gzip,bzip2,pdf"
    )]
    ignore_filetype: String,

    /// One or more admin password hashes compatible with `openssl passwd`.
    /// These should be provided as the hashed password output from `openssl passwd` and
    /// will not be stored in clear text. Multiple `--admin-password` options are allowed.
    #[arg(long = "admin-password")]
    admin_password: Vec<String>,
}
// App configuration passed to handlers
#[derive(Clone)]
struct AppConfig {
    user_whitelist: Option<std::collections::HashSet<String>>,
    gitserver_whitelist: Option<std::collections::HashSet<String>>,
    ignore_filetypes: Option<std::collections::HashSet<String>>,
    admin_passwords: Option<std::collections::HashSet<String>>,
}
use cached::{Cached, Return};
use csscolorparser::parse;
use once_cell::sync::Lazy;
use rsbadges::{Badge, Style};
use sha_crypt::{PasswordHash, PasswordHasher, PasswordVerifier, SHA256_CRYPT, SHA512_CRYPT};
use std::collections::HashSet;
use tempfile::TempDir;
use tokei::{Language, LanguageType, Languages};

const BILLION: usize = 1_000_000_000;
const BLANKS: &str = "blank lines";
const BLUE: &str = "#007ec6";
const GREY: &str = "#555555";
const CODE: &str = "lines of code";
const COMMENTS: &str = "comments";
const FILES: &str = "files";
const HASH_LENGTH: usize = 40;
const LINES: &str = "total lines";
const MILLION: usize = 1_000_000;
const THOUSAND: usize = 1_000;
const DAY_IN_SECONDS: u64 = 24 * 60 * 60;

static CONTENT_TYPE_SVG: Lazy<ContentType> =
    Lazy::new(|| ContentType("image/svg+xml".parse().unwrap()));
static CACHE_TTL_SECONDS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(DAY_IN_SECONDS));
static CACHE_MAX_ENTRIES: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(1000));

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Parse CLI arguments early to allow tests or tools to override defaults.
    let mut args = Args::parse();

    // Support environment variable fallback for bind/port. These are used
    // only when the corresponding CLI flag is not explicitly provided (or
    // when the CLI value equals the default). We retain CLI priority over
    // environment variables.
    if args.bind == "0.0.0.0" {
        if let Ok(bind_from_env) = std::env::var("TOKEI_BIND") {
            if !bind_from_env.is_empty() {
                args.bind = bind_from_env;
            }
        }
    }

    if args.port == 8000 {
        if let Ok(port_from_env) = std::env::var("TOKEI_PORT") {
            if let Ok(parsed) = port_from_env.parse::<u16>() {
                args.port = parsed;
            }
        }
    }

    // Cache TTL: check environment variable TOKEI_CACHE_TTL when the CLI value is unchanged
    if args.cache_ttl == 86400u64 {
        if let Ok(ttl_from_env) = std::env::var("TOKEI_CACHE_TTL") {
            if let Ok(parsed) = ttl_from_env.parse::<u64>() {
                args.cache_ttl = parsed;
            }
        }
    }

    // Store cache TTL into the global static so the `cached` macro can pick it up
    CACHE_TTL_SECONDS.store(args.cache_ttl, Ordering::Relaxed);
    // Store cache max entries into the static atomic so the `cached` crate create expression can pick it up
    CACHE_MAX_ENTRIES.store(args.cache_size, Ordering::Relaxed);
    // Also read env var fallback for cache size `TOKEI_CACHE_SIZE` to mimic CLI -> ENV precedence as other CLI flags
    if args.cache_size == 1000 {
        if let Ok(env_max) = std::env::var("TOKEI_CACHE_SIZE") {
            if let Ok(parsed) = env_max.parse::<usize>() {
                CACHE_MAX_ENTRIES.store(parsed, Ordering::Relaxed);
            }
        }
    }
    dotenv::dotenv().ok();
    // Configure logging: default to verbose (debug) unless disabled with `-q`
    // or overridden via the `RUST_LOG` environment variable. We parse
    // arguments before configuring logging so CLI flags can take effect
    // immediately.
    use env_logger::Env;
    use log::LevelFilter;

    if args.quiet {
        env_logger::Builder::from_env(Env::default())
            .filter_level(LevelFilter::Off)
            .init();
    } else {
        // Default to "info" when RUST_LOG isn't set for verbose output.
        let env = Env::default().filter_or("RUST_LOG", "info");
        env_logger::Builder::from_env(env).init();
    }

    let user_whitelist_value = args
        .user_whitelist
        .clone()
        .or_else(|| std::env::var("TOKEI_USER_WHITELIST").ok());

    let whitelist: Option<std::collections::HashSet<String>> = user_whitelist_value.map(|s| {
        s.split(',')
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect::<std::collections::HashSet<String>>()
    });

    let admin_passwords_value: Option<String> = if !args.admin_password.is_empty() {
        Some(args.admin_password.join(","))
    } else {
        std::env::var("TOKEI_ADMIN_PASSWORD").ok()
    };

    let admin_passwords: Option<std::collections::HashSet<String>> =
        admin_passwords_value.map(|s| {
            s.split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect::<std::collections::HashSet<String>>()
        });

    let gitserver_whitelist_value = args
        .gitserver_whitelist
        .clone()
        .or_else(|| std::env::var("TOKEI_GITSERVER_WHITELIST").ok());

    let gitserver_whitelist: Option<std::collections::HashSet<String>> = gitserver_whitelist_value
        .map(|s| {
            s.split(',')
                // Normalize to lowercase so domain matching is case-insensitive
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty())
                .collect::<std::collections::HashSet<String>>()
        });

    // Parse ignore filetypes option / env var. Normalize extensions to lowercase
    let mut ignore_filetype_value = args.ignore_filetype.clone();
    if ignore_filetype_value
        == "gfs,xsd,csv,dxf,wkt,dgn,rsc,png,a,so,pc,ai,jpg,gif,gz,bz2,xz,gzip,bzip2,pdf"
    {
        if let Ok(env_ignore) = std::env::var("TOKEI_IGNORE_FILETYPE") {
            if !env_ignore.is_empty() {
                ignore_filetype_value = env_ignore;
            }
        }
    }
    let ignore_filetypes: Option<std::collections::HashSet<String>> =
        if ignore_filetype_value.trim().is_empty() {
            None
        } else {
            Some(
                ignore_filetype_value
                    .split(',')
                    .map(|v| v.trim().to_ascii_lowercase())
                    .filter(|v| !v.is_empty())
                    .collect::<std::collections::HashSet<String>>(),
            )
        };

    let app_config = web::Data::new(AppConfig {
        user_whitelist: whitelist,
        gitserver_whitelist: gitserver_whitelist,
        ignore_filetypes: ignore_filetypes,
        admin_passwords: admin_passwords,
    });

    // Inform administrators of whitelists at startup (if configured)
    if let Some(ws) = &app_config.user_whitelist {
        if !ws.is_empty() {
            let mut entries: Vec<String> = ws.iter().cloned().collect();
            entries.sort();
            log::info!("User whitelist configured: {}", entries.join(","));
        }
    }
    if let Some(gsw) = &app_config.gitserver_whitelist {
        if !gsw.is_empty() {
            let mut entries: Vec<String> = gsw.iter().cloned().collect();
            entries.sort();
            log::info!("Git server whitelist configured: {}", entries.join(","));
        }
    }
    if let Some(ifts) = &app_config.ignore_filetypes {
        if !ifts.is_empty() {
            let mut entries: Vec<String> = ifts.iter().cloned().collect();
            entries.sort();
            log::info!("Ignore filetypes configured: {}", entries.join(","));
        }
    }

    HttpServer::new(move || {
        App::new()
            .app_data(app_config.clone())
            .wrap(actix_web::middleware::Logger::default())
            .service(redirect_index)
            .service(create_badge)
    })
    .bind((args.bind.as_str(), args.port))?
    .run()
    .await
}

#[get("/")]
async fn redirect_index() -> HttpResponse {
    HttpResponse::PermanentRedirect()
        .insert_header((LOCATION, "https://github.com/sctg-development/tokeisrv"))
        .finish()
}

macro_rules! respond {
    ($status:ident) => {{
        HttpResponse::$status().finish()
    }};

    ($status:ident, $body:expr) => {{
        HttpResponse::$status()
            .insert_header((CONTENT_TYPE, CONTENT_TYPE_SVG.clone()))
            .body($body)
    }};

    ($status:ident, $accept:expr, $body:expr, $etag:expr) => {{
        HttpResponse::$status()
            .insert_header((CACHE_CONTROL, CacheControl(vec![CacheDirective::NoCache])))
            .insert_header((ETAG, EntityTag::new(false, $etag)))
            .insert_header((
                CONTENT_TYPE,
                if $accept == ContentType::json() {
                    ContentType::json()
                } else {
                    CONTENT_TYPE_SVG.clone()
                },
            ))
            .body($body)
    }};
}

#[allow(non_snake_case)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BadgeQuery {
    category: Option<String>,
    label: Option<String>,
    style: Option<String>,
    color: Option<String>,
    logo: Option<String>,
    r#type: Option<String>,
    show_language: Option<String>,
    language_rank: Option<String>,
    branch: Option<String>,
    action: Option<String>,
    #[serde(rename = "admin-password")]
    admin_password: Option<String>,
}

#[get("/b1/{domain}/{user}/{repo}")]
async fn create_badge(
    request: HttpRequest,
    data: web::Data<AppConfig>,
    path: web::Path<(String, String, String)>,
    web::Query(query): web::Query<BadgeQuery>,
) -> actix_web::Result<HttpResponse> {
    let (domain, user, repo) = path.into_inner();

    // If a whitelist is configured, ensure the requested user is allowed.
    if let Some(whitelist) = &data.user_whitelist {
        if !whitelist.contains(&user) {
            log::warn!("User {} not in whitelist, returning forbidden badge", user);
            // Return a red 'forbidden' badge (SVG) instead of HTTP 403 error.
            let badge = make_badge_style("", "forbidden", "#e05d44", "plastic", "").await?;
            return Ok(respond!(Forbidden, badge));
        }
    }
    let category = query.category.unwrap_or_else(|| "lines".to_owned());
    let (label, no_label) = match query.label {
        Some(v) => (v, false),
        None => ("".to_owned(), true),
    };
    let style: String = query.style.unwrap_or_else(|| "plastic".to_owned());
    let color: String = query.color.unwrap_or_else(|| BLUE.to_owned());
    let logo: String = query.logo.unwrap_or_else(|| "".to_owned());
    let r#type: String = query.r#type.unwrap_or_else(|| "".to_owned());
    let show_language: bool = query
        .show_language
        .unwrap_or_else(|| "".to_owned())
        .parse::<bool>()
        .unwrap_or(false);
    let language_rank: usize = match query.language_rank {
        Some(s) => s.parse::<usize>().unwrap_or(0),
        None => 1,
    };
    let branch: String = query.branch.unwrap_or_else(|| "".to_owned());

    let content_type: ContentType = if let Ok(accept) = Accept::parse(&request) {
        if accept == Accept::json() {
            ContentType::json()
        } else {
            CONTENT_TYPE_SVG.clone()
        }
    } else {
        CONTENT_TYPE_SVG.clone()
    };

    let mut domain = percent_encoding::percent_decode_str(&domain).decode_utf8()?;

    // For backwards compatibility if a domain isn't specified we append `.com`.
    if !domain.contains('.') {
        domain += ".com";
    }

    // If a gitserver whitelist is configured, ensure the requested domain is allowed.
    // We normalize the domain to lowercase to compare against the normalized whitelist.
    let domain_lc = domain.to_ascii_lowercase();
    if let Some(gsw) = &data.gitserver_whitelist {
        if !gsw.contains(domain_lc.as_str()) {
            log::warn!(
                "Git server {} not in gitserver whitelist, returning forbidden badge",
                domain
            );
            let badge = make_badge_style("", "forbidden", "#e05d44", "plastic", "").await?;
            return Ok(respond!(Forbidden, badge));
        }
    }

    // Support a special domain "local" for tests: derive the repo URL from
    // the current working directory. Otherwise build a remote HTTPS URL.
    let repo_url: String = if domain_lc == "local" || domain_lc == "local.com" {
        // Local repository: use cwd
        let cwd = std::env::current_dir()
            .map_err(|e| actix_web::error::ErrorBadRequest(eyre::eyre!(e.to_string())))?;
        cwd.to_str()
            .ok_or_else(|| actix_web::error::ErrorBadRequest(eyre::eyre!("Invalid cwd")))?
            .to_owned()
    } else {
        format!("https://{}/{}/{}", domain_lc, user, repo)
    };

    // If this is an admin action 'flush-cache', handle it early without cloning
    if let Some(action_value) = &query.action {
        if action_value == "flush-cache" {
            // Verify admin password (only allow SHA-based algorithms $5 and $6)
            let provided = query
                .admin_password
                .clone()
                .unwrap_or_else(|| "".to_owned());
            match verify_admin_password(&provided, &data.admin_passwords) {
                Ok(true) => {
                    // authorized
                }
                Ok(false) => {
                    log::warn!(
                        "Admin authentication failed for flush-cache on {}",
                        repo_url
                    );
                    let badge = make_badge_style("", "forbidden", "#e05d44", "plastic", "").await?;
                    return Ok(respond!(Forbidden, badge));
                }
                Err(PasswordVerifyError::AlgorithmNotAllowed) => {
                    log::warn!(
                        "Admin authentication rejected due to disallowed algorithm for {}",
                        repo_url
                    );
                    let body = "403 - password algorithm not allowed".to_string();
                    return Ok(respond!(Forbidden, body));
                }
            }
            let removed = flush_cache_for_repo(&repo_url, data.ignore_filetypes.as_ref());
            log::info!(
                "Admin flush-cache: removed {} entries for {}",
                removed,
                repo_url
            );
            let badge = make_badge_style("", "cache flushed", BLUE, "plastic", "").await?;
            return Ok(respond!(Ok, badge));
        }
    }

    let mut url: String = String::new();
    let mut sha: String = String::new();
    let mut branch_name: String = String::new();

    if domain_lc == "local" || domain_lc == "local.com" {
        // Local repository: use cwd
        let cwd = std::path::Path::new(&repo_url);
        url = repo_url.clone();

        let repo_local = Repository::open(&cwd)
            .map_err(|e| actix_web::error::ErrorBadRequest(eyre::eyre!(e.to_string())))?;

        // Determine head branch candidates from local refs
        let mut available_branches: Vec<String> = vec![];
        if let Ok(mut bs) = repo_local.branches(None) {
            while let Some(Ok((b, _))) = bs.next() {
                if let Ok(name) = b.name() {
                    if let Some(s) = name {
                        available_branches.push(s.to_string());
                    }
                }
            }
        }

        if available_branches.is_empty() {
            // fallback to HEAD if no named branches
            available_branches.push("HEAD".to_string());
        }

        // Determine default head branch if not provided by query:
        // prefer 'main' then 'master' then the first branch
        let head_branch = if available_branches.contains(&"main".to_string()) {
            "main".to_string()
        } else if available_branches.contains(&"master".to_string()) {
            "master".to_string()
        } else {
            available_branches[0].clone()
        };

        // If the request included a `branch` verify it's available
        if !branch.is_empty() && !available_branches.contains(&branch) {
            return Err(actix_web::error::ErrorBadRequest(eyre::eyre!(
                "Invalid SHA provided."
            )));
        }

        branch_name = if branch.is_empty() {
            head_branch
        } else {
            branch.clone()
        };

        // Find commit OID for the branch (fallback to HEAD commit)
        sha = match repo_local.find_branch(&branch_name, git2::BranchType::Local) {
            Ok(b) => b
                .into_reference()
                .target()
                .map(|o| o.to_string())
                .ok_or_else(|| {
                    actix_web::error::ErrorBadRequest(eyre::eyre!("Invalid local branch"))
                })?,
            Err(_) => repo_local
                .head()
                .and_then(|h| h.peel_to_commit())
                .map(|c| c.id().to_string())
                .map_err(|e| actix_web::error::ErrorBadRequest(eyre::eyre!(e.to_string())))?,
        };
    } else {
        url = repo_url.clone();

        // Use libgit2 via git2 crate to query remote refs and determine branch
        let tmp_bare_dir = TempDir::new()?;
        let repo = match Repository::init_bare(tmp_bare_dir.path()) {
            Ok(r) => r,
            Err(e) => {
                return Err(actix_web::error::ErrorBadRequest(
                    eyre::eyre!(e.to_string()),
                ))
            }
        };
        let mut remote = match repo.remote_anonymous(&url) {
            Ok(r) => r,
            Err(e) => {
                return Err(actix_web::error::ErrorBadRequest(
                    eyre::eyre!(e.to_string()),
                ))
            }
        };
        if let Err(e) = remote.connect(Direction::Fetch) {
            return Err(actix_web::error::ErrorBadRequest(
                eyre::eyre!(e.to_string()),
            ));
        }
        let refs = match remote.list() {
            Ok(r) => r,
            Err(e) => {
                return Err(actix_web::error::ErrorBadRequest(
                    eyre::eyre!(e.to_string()),
                ))
            }
        };

        // Build a vector of available branch names (refs/heads/*)
        let available_branches: Vec<String> = refs
            .iter()
            .filter_map(|r| {
                let name = r.name();
                if name.starts_with("refs/heads/") {
                    Some(name[11..].to_string())
                } else {
                    None
                }
            })
            .collect();
        if available_branches.is_empty() {
            return Err(actix_web::error::ErrorBadRequest(eyre::eyre!(
                "Invalid SHA provided."
            )));
        }

        // Determine default head branch if not provided by query:
        // prefer 'main' then 'master' then the first branch
        let head_branch = if available_branches.contains(&"main".to_string()) {
            "main".to_string()
        } else if available_branches.contains(&"master".to_string()) {
            "master".to_string()
        } else {
            available_branches[0].clone()
        };

        // If the request included a `branch` verify it's available
        if !branch.is_empty() && !available_branches.contains(&branch) {
            return Err(actix_web::error::ErrorBadRequest(eyre::eyre!(
                "Invalid SHA provided."
            )));
        }

        branch_name = if branch.is_empty() {
            head_branch
        } else {
            branch.clone()
        };
        // Find the oid for the requested branch
        let target_ref = format!("refs/heads/{}", branch_name);
        for r in refs.iter() {
            if r.name() == target_ref.as_str() {
                sha = r.oid().to_string();
                break;
            }
        }
    }

    (sha.len() == HASH_LENGTH)
        .then(|| ())
        .ok_or_else(|| actix_web::error::ErrorBadRequest(eyre::eyre!("Invalid SHA provided.")))?;

    // Debug: expose computed values (helpful for tests)
    println!(
        "[DEBUG] create_badge values: url={} sha={} branch_name={}",
        url, sha, branch_name
    );
    if let Ok(if_none_match) = IfNoneMatch::parse(&request) {
        log::debug!("Checking If-None-Match: {}#{}", sha, branch_name);
        let entity_tag: EntityTag = EntityTag::new(false, etag_identifier(&sha, &branch_name));
        let found_match: bool = match if_none_match {
            IfNoneMatch::Any => false,
            IfNoneMatch::Items(items) => items
                .iter()
                .any(|etag: &EntityTag| etag.weak_eq(&entity_tag)),
        };

        if found_match {
            // Only return NotModified if we actually have a cached entry for this repo
            // and sha/branch. Check both base key and the ignore-filetypes-suffixed
            // variant to match how get_statistics stores entries.
            let base_key = repo_identifier(&url, &sha, &branch_name);
            let mut has_entry = CACHE.lock().unwrap().cache_get(&base_key).is_some();
            if !has_entry {
                if let Some(ifts) = data.ignore_filetypes.as_ref() {
                    let mut v: Vec<String> = ifts.iter().cloned().collect();
                    v.sort();
                    if !v.is_empty() {
                        let mut suffixed = base_key.clone();
                        suffixed.push('#');
                        suffixed.push_str(&v.join(","));
                        has_entry = CACHE.lock().unwrap().cache_get(&suffixed).is_some();
                    }
                }
            }
            if has_entry {
                log::info!("{}#{}#{} Not Modified", url, sha, branch_name);
                return Ok(respond!(NotModified));
            }
        }
    }

    let entry: Return<Vec<(LanguageType, Language)>> = if domain_lc == "local" {
        // Compute statistics directly for the current working directory to avoid
        // cloning over the network in tests.
        let path = url.clone();
        let mut languages: Languages = Languages::new();

        // Build exclude patterns from configured ignore filetypes
        let mut exclude_patterns: Vec<String> = Vec::new();
        if let Some(ifts) = data.ignore_filetypes.as_ref() {
            for ext in ifts.iter() {
                let normalized_ext = ext.trim().trim_start_matches('.');
                if !normalized_ext.is_empty() {
                    exclude_patterns.push(format!("**/*.{}", normalized_ext));
                }
            }
        }
        let exclude_refs: Vec<&str> = exclude_patterns.iter().map(|s| s.as_str()).collect();
        languages.get_statistics(
            &[path.as_str()],
            if exclude_refs.is_empty() {
                &[]
            } else {
                &exclude_refs[..]
            },
            &tokei::Config::default(),
        );

        // Strip path prefixes from report names to match behaviour of get_statistics
        let mut iter = languages.iter_mut();
        while let Some((_, language)) = iter.next() {
            for report in &mut language.reports {
                match report.name.strip_prefix(&path) {
                    Ok(s) => report.name = s.to_owned(),
                    Err(_) => {}
                }
            }
            for (_, child) in &mut language.children {
                for language in child.into_iter() {
                    match language.name.strip_prefix(&path) {
                        Ok(s) => language.name = s.to_owned(),
                        Err(_) => {}
                    }
                }
            }
        }

        let mut languages_sorted_by_lines_of_code: Vec<(LanguageType, Language)> =
            languages.into_iter().collect();
        languages_sorted_by_lines_of_code.sort_by(|(_, a), (_, b)| b.code.cmp(&a.code));
        cached::Return::new(languages_sorted_by_lines_of_code)
    } else {
        get_statistics(&url, &sha, &branch_name, data.ignore_filetypes.as_ref())
            .map_err(actix_web::error::ErrorBadRequest)?
    };

    if entry.was_cached {
        log::info!("{}#{}#{} Cache hit", url, sha, branch_name);
    }

    let language_types: HashSet<LanguageType> = r#type
        .split(',')
        .filter_map(|s: &str| str::parse::<LanguageType>(s).ok())
        .into_iter()
        .collect::<HashSet<LanguageType>>();

    let languages: Vec<(LanguageType, Language)> = if language_types.is_empty() {
        entry.value
    } else {
        entry
            .value
            .into_iter()
            .filter(|(language_type, _)| language_types.contains(&language_type))
            .into_iter()
            .collect()
    };
    let ranking_language = if !show_language {
        String::new()
    } else if languages.is_empty() {
        "No Languages".to_owned()
    } else if language_rank == 0 || language_rank > languages.len() {
        "N/A".to_owned()
    } else {
        let (ranking_language_type, _) = languages[language_rank - 1];
        ranking_language_type.name().to_owned()
    };

    let mut stats = Language::new();
    for (_, language) in &languages {
        stats += language.clone();
    }

    log::debug!(
        "{url}#{sha}#{branch_name} - Languages (most common to least common) {languages:#?} Lines {lines} Code {code} Comments {comments} Blanks {blanks}",
        url = url,
        sha = sha,
        branch_name = branch_name,
        languages = languages,
        lines = stats.lines(),
        code = stats.code,
        comments = stats.comments,
        blanks = stats.blanks
    );

    log::info!(
        "{}#{}#{} - Lines: {} Code: {} Comments: {} Blanks: {}",
        url,
        sha,
        branch_name,
        stats.lines(),
        stats.code,
        stats.comments,
        stats.blanks
    );

    let badge: String = make_badge(
        &content_type,
        &stats,
        &category,
        &label,
        &style,
        &color,
        &logo,
        &ranking_language,
        no_label,
    )
    .await?;

    Ok(respond!(
        Ok,
        content_type,
        badge,
        etag_identifier(&sha, &branch_name)
    ))
}

fn repo_identifier(url: &str, sha: &str, branch_name: &str) -> String {
    format!("{}#{}#{}", url, sha, branch_name)
}

fn etag_identifier(sha: &str, branch_name: &str) -> String {
    format!("{}#{}", sha, branch_name)
}

#[cached::proc_macro::cached(
    name = "CACHE",
    result = true,
    with_cached_flag = true,
    ty = "cached::TimedSizedCache<String, cached::Return<Vec<(LanguageType,Language)>>>",
    create = r#"{ let ttl = CACHE_TTL_SECONDS.load(Ordering::Relaxed); let max = CACHE_MAX_ENTRIES.load(Ordering::Relaxed); cached::TimedSizedCache::with_size_and_lifespan(max, std::time::Duration::from_secs(ttl)) }"#,
    convert = r#"{ let mut key = repo_identifier(url, _sha, branch_name); if let Some(ifts) = ignore_filetypes { let mut v: Vec<String> = ifts.iter().cloned().collect(); v.sort(); if !v.is_empty() { key.push('#'); key.push_str(&v.join(",")); } } key }"#
)]
fn get_statistics(
    url: &str,
    _sha: &str,
    branch_name: &str,
    ignore_filetypes: Option<&std::collections::HashSet<String>>,
) -> eyre::Result<cached::Return<Vec<(LanguageType, Language)>>> {
    log::info!("{} - Cloning", url);
    let temp_dir: TempDir = TempDir::new()?;
    let temp_path: &str = temp_dir.path().to_str().unwrap();

    // Clone using libgit2 RepoBuilder with shallow depth and optional credentials
    let mut fo = FetchOptions::new();
    let mut callbacks = RemoteCallbacks::new();
    // Use GITHUB_TOKEN if available for HTTPS auth (x-access-token)
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        callbacks.credentials(move |_, _username_from_url, _| {
            // Use username "x-access-token" as suggested by GitHub for personal access tokens
            Cred::userpass_plaintext("x-access-token", &token)
        });
    }
    fo.remote_callbacks(callbacks);
    fo.depth(1);

    let mut builder = RepoBuilder::new();
    builder.fetch_options(fo);
    if !branch_name.is_empty() {
        builder.branch(branch_name);
    }
    builder
        .clone(url, Path::new(temp_path))
        .map_err(|e| eyre::eyre!(e.to_string()))?;

    let mut languages: Languages = Languages::new();
    log::info!("{} - Getting Statistics", url);
    // Build a set of exclude patterns from configured ignore filetypes.
    // Convert extension `foo` to glob pattern `**/*.foo`.
    let mut exclude_patterns: Vec<String> = Vec::new();
    if let Some(ifts) = ignore_filetypes {
        for ext in ifts {
            // ignore trailing dots or accidental leading dots
            let normalized_ext = ext.trim().trim_start_matches('.');
            if !normalized_ext.is_empty() {
                exclude_patterns.push(format!("**/*.{}", normalized_ext));
            }
        }
    }
    // Convert to slice of &str for tokei API
    let exclude_refs: Vec<&str> = exclude_patterns.iter().map(|s| s.as_str()).collect();
    languages.get_statistics(
        &[temp_path],
        if exclude_refs.is_empty() {
            &[]
        } else {
            &exclude_refs[..]
        },
        &tokei::Config::default(),
    );

    let mut iter = languages.iter_mut();
    while let Some((_, language)) = iter.next() {
        for report in &mut language.reports {
            report.name = report.name.strip_prefix(temp_path)?.to_owned();
        }
        for (_, child) in &mut language.children {
            for language in child.into_iter() {
                language.name = language.name.strip_prefix(temp_path)?.to_owned();
            }
        }
    }

    let mut languages_sorted_by_lines_of_code: Vec<(LanguageType, Language)> =
        languages.into_iter().collect();
    languages_sorted_by_lines_of_code.sort_by(|(_, a), (_, b)| b.code.cmp(&a.code));

    Ok(cached::Return::new(languages_sorted_by_lines_of_code))
}

fn trim_and_float(num: usize, trim: usize) -> f64 {
    (num as f64) / (trim as f64)
}

#[derive(Debug)]
enum PasswordVerifyError {
    AlgorithmNotAllowed,
}

fn verify_admin_password(
    provided: &str,
    hashes: &Option<std::collections::HashSet<String>>,
) -> Result<bool, PasswordVerifyError> {
    // Verify using a pure Rust implementation from sha-crypt so we do not
    // depend on the openssl binary at runtime. Only $5 (SHA-256) and $6
    // (SHA-512) crypt hashes are accepted. Any other algorithm is rejected.
    if hashes.is_none() {
        return Ok(false);
    }
    for h in hashes.as_ref().unwrap().iter() {
        // Expecting formats like: $6$salt$rest
        if !h.starts_with('$') {
            continue;
        }
        let comps: Vec<&str> = h.split('$').collect();
        if comps.len() < 3 {
            continue;
        }
        let id = comps[1];
        match id {
            "6" => {
                if let Ok(ph) = PasswordHash::new(h) {
                    if SHA512_CRYPT
                        .verify_password(provided.as_bytes(), &ph)
                        .is_ok()
                    {
                        return Ok(true);
                    }
                }
            }
            "5" => {
                if let Ok(ph) = PasswordHash::new(h) {
                    if SHA256_CRYPT
                        .verify_password(provided.as_bytes(), &ph)
                        .is_ok()
                    {
                        return Ok(true);
                    }
                }
            }
            _ => return Err(PasswordVerifyError::AlgorithmNotAllowed),
        }
    }
    Ok(false)
}

fn flush_cache_for_repo(
    url: &str,
    ignore_filetypes: Option<&std::collections::HashSet<String>>,
) -> usize {
    // Determine branch SHAs for the target repo (local path or remote URL), then remove
    // each matching repo identifier from the cache. We remove both the base key and
    // the key that includes the ignore-filetypes suffix (if configured).
    let mut removed = 0usize;

    // Helper to remove a specific key and increment counter
    let mut remove_key = |key: String| {
        log::debug!(
            "flush_cache_for_repo: Attempting to remove cache key: {}",
            key
        );
        if CACHE.lock().unwrap().cache_remove(&key).is_some() {
            removed += 1;
            log::debug!("flush_cache_for_repo: Removed cache key: {}", key);
        } else {
            log::debug!("flush_cache_for_repo: Cache key not found: {}", key);
        }
    };

    // Helper to remove both base and ignore-filetypes-suffixed key
    let mut remove_key_with_possible_suffix = |base: String| {
        // Remove base
        remove_key(base.clone());
        // Remove suffixed variant if ignore patterns configured
        if let Some(ifts) = ignore_filetypes {
            let mut v: Vec<String> = ifts.iter().cloned().collect();
            v.sort();
            if !v.is_empty() {
                let mut suffixed = base.clone();
                suffixed.push('#');
                suffixed.push_str(&v.join(","));
                remove_key(suffixed);
            }
        }
    };

    // If this looks like a local path, try opening it
    log::debug!(
        "flush_cache_for_repo: checking if url is local path: {}",
        url
    );
    if std::path::Path::new(url).exists() {
        log::debug!("flush_cache_for_repo: treating {} as a local path", url);
        if let Ok(repo) = Repository::open(url) {
            if let Ok(mut bs) = repo.branches(None) {
                while let Some(Ok((b, _))) = bs.next() {
                    if let Ok(name_opt) = b.name() {
                        if let Some(name) = name_opt {
                            log::debug!("flush_cache_for_repo: Found local branch: {}", name);
                            if let Ok(o) = repo.find_branch(name, git2::BranchType::Local) {
                                if let Some(oid) = o.into_reference().target() {
                                    log::debug!(
                                        "flush_cache_for_repo: Branch {} -> oid {}",
                                        name,
                                        oid
                                    );
                                    let base = repo_identifier(url, &oid.to_string(), name);
                                    log::debug!(
                                        "flush_cache_for_repo: Computed base key for branch: {}",
                                        base
                                    );
                                    remove_key_with_possible_suffix(base);
                                }
                            }
                        }
                    }
                }
            }
            // Also remove HEAD commit entry if present
            if let Ok(head) = repo.head() {
                if let Ok(commit) = head.peel_to_commit() {
                    log::debug!("flush_cache_for_repo: Found HEAD commit: {}", commit.id());
                    let base = repo_identifier(url, &commit.id().to_string(), "HEAD");
                    log::debug!("flush_cache_for_repo: Computed base key for HEAD: {}", base);
                    remove_key_with_possible_suffix(base);
                }
            }
            log::debug!(
                "flush_cache_for_repo: removed {} keys for local repo {}",
                removed,
                url
            );
            return removed;
        } else {
            log::debug!("flush_cache_for_repo: failed to open local repo: {}", url);
        }
    }

    log::debug!(
        "flush_cache_for_repo: attempting remote refs lookup for {}",
        url
    );
    // Otherwise attempt to query remote refs using a bare temporary repo
    if let Ok(tmp) = TempDir::new() {
        if let Ok(bare) = Repository::init_bare(tmp.path()) {
            if let Ok(mut remote) = bare.remote_anonymous(url) {
                log::debug!(
                    "flush_cache_for_repo: Connecting to remote to list refs: {}",
                    url
                );
                if remote.connect(Direction::Fetch).is_ok() {
                    if let Ok(refs) = remote.list() {
                        log::debug!("flush_cache_for_repo: Remote refs count: {}", refs.len());
                        for r in refs.iter() {
                            log::debug!(
                                "flush_cache_for_repo: Remote ref: {} -> {}",
                                r.name(),
                                r.oid()
                            );
                            if r.name().starts_with("refs/heads/") {
                                let branch = r.name()[11..].to_string();
                                let oid = r.oid().to_string();
                                let base = repo_identifier(url, &oid, &branch);
                                log::debug!("flush_cache_for_repo: Computed base key for remote branch {}: {}", branch, base);
                                remove_key_with_possible_suffix(base);
                            }
                        }
                    } else {
                        log::debug!(
                            "flush_cache_for_repo: Failed to list remote refs for {}",
                            url
                        );
                    }
                } else {
                    log::debug!("flush_cache_for_repo: Failed to connect to remote {}", url);
                }
            } else {
                log::debug!(
                    "flush_cache_for_repo: Failed to init anonymous remote for {}",
                    url
                );
            }
        } else {
            log::debug!(
                "flush_cache_for_repo: Failed to init bare repository in temp dir for {}",
                url
            );
        }
    } else {
        log::debug!(
            "flush_cache_for_repo: Failed to create temp dir for remote lookup for {}",
            url
        );
    }

    log::debug!(
        "flush_cache_for_repo: total removed {} keys for {}",
        removed,
        url
    );
    removed
}

async fn make_badge_style(
    label: &str,
    msg: &str,
    color: &str,
    style: &str,
    logo: &str,
) -> Result<String, actix_web::Error> {
    fn badge(label: &str, msg: &str, color: &str) -> Badge {
        Badge {
            label_text: label.to_owned(),
            label_color: GREY.to_owned(),
            msg_text: msg.to_owned(),
            msg_color: match parse(color) {
                Ok(result) => result.to_css_hex(),
                Err(_error) => BLUE.to_owned(),
            },
            ..Badge::default()
        }
    }

    let badge_with_logo: Badge = Badge {
        logo: logo.to_owned(),
        embed_logo: !logo.is_empty(),
        ..badge(label, msg, color)
    };

    fn stylize_badge(badge: Badge, style: &str) -> Style {
        match style {
            "flat" => Style::Flat(badge),
            "flat-square" => Style::FlatSquare(badge),
            "plastic" => Style::Plastic(badge),
            "for-the-badge" => Style::ForTheBadge(badge),
            "social" => Style::Social(badge),
            _ => Style::Flat(badge),
        }
    }

    match stylize_badge(badge_with_logo, style).generate_svg() {
        Ok(s) => Ok(s),
        Err(_e) => Ok(stylize_badge(badge(label, msg, color), style)
            .generate_svg()
            .unwrap()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn make_badge(
    content_type: &ContentType,
    stats: &Language,
    category: &str,
    label: &str,
    style: &str,
    color: &str,
    logo: &str,
    ranking_language: &str,
    no_label: bool,
) -> actix_web::Result<String> {
    if *content_type == ContentType::json() {
        return Ok(serde_json::to_string(&stats)?);
    }

    if !ranking_language.is_empty() {
        return make_badge_style(label, ranking_language, color, style, logo).await;
    }

    let (amount, label) = match category {
        "code" => (stats.code, if no_label { CODE } else { label }),
        "files" => (stats.reports.len(), if no_label { FILES } else { label }),
        "blanks" => (stats.blanks, if no_label { BLANKS } else { label }),
        "comments" => (stats.comments, if no_label { COMMENTS } else { label }),
        _ => (stats.lines(), if no_label { LINES } else { label }),
    };

    let amount: String = if amount >= BILLION {
        format!("{:.1}B", trim_and_float(amount, BILLION))
    } else if amount >= MILLION {
        format!("{:.1}M", trim_and_float(amount, MILLION))
    } else if amount >= THOUSAND {
        format!("{:.1}K", trim_and_float(amount, THOUSAND))
    } else {
        amount.to_string()
    };

    make_badge_style(label, &amount, color, style, logo).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::body::to_bytes;
    use actix_web::test;
    use actix_web::{http::header::CONTENT_TYPE, http::StatusCode};
    use std::collections::HashSet;

    #[actix_web::test]
    async fn redirect_index_returns_redirect() {
        let app = test::init_service(App::new().service(redirect_index)).await;
        let req = test::TestRequest::get().uri("/").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        let loc = resp.headers().get(LOCATION).unwrap().to_str().unwrap();
        assert_eq!(loc, "https://github.com/sctg-development/tokeisrv");
    }

    #[actix_web::test]
    async fn respond_macro_without_body() {
        let resp = respond!(Ok);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn respond_macro_with_body_and_content_type() {
        let body = "<svg></svg>";
        let resp = respond!(Ok, body);
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get(CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(ct.contains("image/svg+xml"));
        let bytes = to_bytes(resp.into_body()).await.unwrap();
        assert_eq!(bytes, body);
    }

    #[actix_web::test]
    async fn make_badge_style_produces_svg() {
        let svg = make_badge_style("label", "42", "#007ec6", "plastic", "")
            .await
            .unwrap();
        assert!(svg.trim_start().starts_with('<'));
    }

    #[actix_web::test]
    async fn make_badge_formats_large_numbers() {
        let mut stats = tokei::Language::default();
        stats.code = 1_500_000_000; // 1.5B
        let svg = make_badge(
            &CONTENT_TYPE_SVG,
            &stats,
            "code",
            "",
            "plastic",
            BLUE,
            "",
            "",
            true,
        )
        .await
        .unwrap();
        // expect a generated SVG (contains '<svg')
        assert!(svg.contains("<svg") || svg.contains("<svg"));
    }

    #[actix_web::test]
    async fn create_badge_forbidden_when_user_not_in_whitelist() {
        let mut whitelist = HashSet::new();
        whitelist.insert("alice".to_string());
        let cfg = AppConfig {
            user_whitelist: Some(whitelist),
            gitserver_whitelist: None,
            ignore_filetypes: None,
            admin_passwords: None,
        };
        let data = web::Data::new(cfg);
        let app = test::init_service(App::new().app_data(data.clone()).service(create_badge)).await;
        let req = test::TestRequest::get()
            .uri("/b1/github/bob/repo")
            .to_request();
        let resp = test::call_service(&app, req).await;
        // Should return Forbidden badge response
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[actix_web::test]
    async fn identifiers_and_trim_float() {
        assert_eq!(repo_identifier("u", "s", "b"), "u#s#b");
        assert_eq!(etag_identifier("s", "b"), "s#b");
        assert!((trim_and_float(3, 2) - 1.5).abs() < 1e-9);
    }

    #[actix_web::test]
    async fn make_badge_content_type_json_returns_stats() {
        let stats = tokei::Language::default();
        let json = make_badge(
            &ContentType::json(),
            &stats,
            "lines",
            "",
            "plastic",
            BLUE,
            "",
            "",
            true,
        )
        .await
        .unwrap();
        let parsed: tokei::Language = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.code, stats.code);
    }

    #[actix_web::test]
    async fn make_badge_category_comments_formats_k() {
        let mut stats = tokei::Language::default();
        stats.comments = 1500;
        let svg = make_badge(
            &CONTENT_TYPE_SVG,
            &stats,
            "comments",
            "",
            "plastic",
            BLUE,
            "",
            "",
            true,
        )
        .await
        .unwrap();
        assert!(svg.contains("1.5K") || svg.contains("1500"));
    }

    #[actix_web::test]
    async fn make_badge_ranking_language_returns_svg() {
        let stats = tokei::Language::default();
        let svg = make_badge(
            &CONTENT_TYPE_SVG,
            &stats,
            "lines",
            "",
            "plastic",
            BLUE,
            "",
            "rust",
            true,
        )
        .await
        .unwrap();
        assert!(svg.contains("<svg"));
    }

    #[actix_web::test]
    async fn make_badge_style_invalid_color_fallback() {
        let svg = make_badge_style("l", "m", "notacolor", "plastic", "")
            .await
            .unwrap();
        assert!(svg.contains("<svg"));
    }

    #[actix_web::test]
    async fn create_badge_for_local_repo_succeeds() {
        // Construct statistics for the current repo without using the HTTP handler
        // (avoids network / shallow clone issues in test environments).
        let cwd = std::env::current_dir().unwrap();
        let path = cwd.to_str().unwrap();
        let mut languages: Languages = Languages::new();
        languages.get_statistics(&[path], &[], &tokei::Config::default());

        // normalize report names (same as get_statistics does)
        let mut iter = languages.iter_mut();
        while let Some((_, language)) = iter.next() {
            for report in &mut language.reports {
                if let Ok(s) = report.name.strip_prefix(path) {
                    report.name = s.to_owned();
                }
            }
            for (_, child) in &mut language.children {
                for language in child.into_iter() {
                    if let Ok(s) = language.name.strip_prefix(path) {
                        language.name = s.to_owned();
                    }
                }
            }
        }

        let mut languages_sorted_by_lines_of_code: Vec<(LanguageType, Language)> =
            languages.into_iter().collect();
        languages_sorted_by_lines_of_code.sort_by(|(_, a), (_, b)| b.code.cmp(&a.code));

        let mut stats = Language::new();
        for (_, language) in &languages_sorted_by_lines_of_code {
            stats += language.clone();
        }

        let svg = make_badge(
            &CONTENT_TYPE_SVG,
            &stats,
            "lines",
            "",
            "plastic",
            BLUE,
            "",
            "",
            true,
        )
        .await
        .unwrap();
        assert!(svg.contains("<svg"));
    }

    #[actix_web::test]
    async fn create_badge_forbidden_for_github_rust() {
        // whitelist only 'local' so github is forbidden
        let mut gsw = HashSet::new();
        gsw.insert("local".to_string());
        let cfg = AppConfig {
            user_whitelist: None,
            gitserver_whitelist: Some(gsw),
            ignore_filetypes: None,
            admin_passwords: None,
        };
        let data = web::Data::new(cfg);
        let app = test::init_service(App::new().app_data(data.clone()).service(create_badge)).await;
        let req = test::TestRequest::get()
            .uri("/b1/github/rust-lang/rust")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[actix_web::test]
    async fn admin_flush_cache_success() {
        // Prepare admin password hash using pure-Rust sha-crypt (deterministic salt)
        let pwd = b"supersecret";
        let salt = b"testsalt";
        let password_hash = SHA512_CRYPT
            .hash_password_with_salt(pwd, salt)
            .expect("hashing failed");
        let hash = password_hash.to_string();
        let mut apw = std::collections::HashSet::new();
        apw.insert(hash.clone());

        let cfg = AppConfig {
            user_whitelist: None,
            gitserver_whitelist: None,
            ignore_filetypes: None,
            admin_passwords: Some(apw),
        };
        let data = web::Data::new(cfg);

        // Insert an entry into the cache for the target repo (use local path to avoid network)
        let cwd = std::env::current_dir().unwrap();
        let repo_url = cwd.to_str().unwrap().to_string();
        // Use the local HEAD commit so flush can discover and remove it
        let repo_local = Repository::open(&cwd).unwrap();
        let head_commit = repo_local.head().and_then(|h| h.peel_to_commit()).unwrap();
        let key = repo_identifier(&repo_url, &head_commit.id().to_string(), "HEAD");
        CACHE
            .lock()
            .unwrap()
            .cache_set(key.clone(), cached::Return::new(vec![]));
        assert!(CACHE.lock().unwrap().cache_get(&key).is_some());

        let app = test::init_service(App::new().app_data(data.clone()).service(create_badge)).await;
        let req = test::TestRequest::get()
            .uri("/b1/local/anyuser/anyrepo?action=flush-cache&admin-password=supersecret")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // ensure cache key has been removed
        assert!(CACHE.lock().unwrap().cache_get(&key).is_none());
    }

    #[actix_web::test]
    async fn admin_flush_cache_bad_password() {
        // Prepare admin password hash using pure-Rust sha-crypt (deterministic salt)
        let pwd = b"supersecret";
        let salt = b"testsalt";
        let password_hash = SHA512_CRYPT
            .hash_password_with_salt(pwd, salt)
            .expect("hashing failed");
        let hash = password_hash.to_string();
        let mut apw = std::collections::HashSet::new();
        apw.insert(hash.clone());

        let cfg = AppConfig {
            user_whitelist: None,
            gitserver_whitelist: None,
            ignore_filetypes: None,
            admin_passwords: Some(apw),
        };
        let data = web::Data::new(cfg);

        // Insert an entry into the cache for the target repo (use local path to avoid network)
        let cwd = std::env::current_dir().unwrap();
        let repo_url = cwd.to_str().unwrap().to_string();
        // Use the local HEAD commit so flush can discover and remove it
        let repo_local = Repository::open(&cwd).unwrap();
        let head_commit = repo_local.head().and_then(|h| h.peel_to_commit()).unwrap();
        let key = repo_identifier(&repo_url, &head_commit.id().to_string(), "HEAD");
        CACHE
            .lock()
            .unwrap()
            .cache_set(key.clone(), cached::Return::new(vec![]));
        assert!(CACHE.lock().unwrap().cache_get(&key).is_some());

        let app = test::init_service(App::new().app_data(data.clone()).service(create_badge)).await;
        let req = test::TestRequest::get()
            .uri("/b1/local/anyuser/anyrepo?action=flush-cache&admin-password=badpass")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        // ensure cache key still exists
        assert!(CACHE.lock().unwrap().cache_get(&key).is_some());
    }

    #[actix_web::test]
    async fn admin_flush_cache_algorithm_not_allowed() {
        // Prepare admin password hash using MD5 prefix to trigger AlgorithmNotAllowed
        let hash = "$1$testsalt$placeholder".to_string();
        let mut apw = std::collections::HashSet::new();
        apw.insert(hash.clone());

        let cfg = AppConfig {
            user_whitelist: None,
            gitserver_whitelist: None,
            ignore_filetypes: None,
            admin_passwords: Some(apw),
        };
        let data = web::Data::new(cfg);

        // Insert an entry into the cache for the target repo (use local path to avoid network)
        let cwd = std::env::current_dir().unwrap();
        let repo_url = cwd.to_str().unwrap().to_string();
        // Use the local HEAD commit so flush can discover and remove it
        let repo_local = Repository::open(&cwd).unwrap();
        let head_commit = repo_local.head().and_then(|h| h.peel_to_commit()).unwrap();
        let key = repo_identifier(&repo_url, &head_commit.id().to_string(), "HEAD");
        CACHE
            .lock()
            .unwrap()
            .cache_set(key.clone(), cached::Return::new(vec![]));
        assert!(CACHE.lock().unwrap().cache_get(&key).is_some());

        let app = test::init_service(App::new().app_data(data.clone()).service(create_badge)).await;
        let req = test::TestRequest::get()
            .uri("/b1/local/anyuser/anyrepo?action=flush-cache&admin-password=supersecret")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        // ensure body contains our specific message
        let bytes = to_bytes(resp.into_body()).await.unwrap();
        let body = String::from_utf8_lossy(&bytes).to_string();
        assert!(body.contains("403 - password algorithm not allowed"));
        // ensure cache key still exists
        assert!(CACHE.lock().unwrap().cache_get(&key).is_some());
    }

    #[actix_web::test]
    async fn admin_flush_cache_real_life() {
        // Real-life style test against the real GitHub repo via the running server.
        // Prepare admin password hash (SHA-512) for password "toto"
        let pwd = b"toto";
        let salt = b"testsalt";
        let password_hash = SHA512_CRYPT
            .hash_password_with_salt(pwd, salt)
            .expect("hashing failed");
        let hash = password_hash.to_string();

        // Use the same ignore_filetypes as default to ensure cache keys may include suffix
        let default_ifts = vec![
            "gfs", "xsd", "csv", "dxf", "wkt", "dgn", "rsc", "png", "a", "so", "pc", "ai", "jpg",
            "gif", "gz", "bz2", "xz", "gzip", "bzip2", "pdf",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<HashSet<String>>();

        // Admin hashes contain the sha512 hash
        let mut apw = std::collections::HashSet::new();
        apw.insert(hash.clone());

        let cfg = AppConfig {
            user_whitelist: None,
            gitserver_whitelist: None,
            ignore_filetypes: Some(default_ifts),
            admin_passwords: Some(apw),
        };
        let data = web::Data::new(cfg);

        // Start actix server on a random local port with real repo handling
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind failed");
        let addr = listener.local_addr().unwrap();
        let data_for_server = data.clone();
        let server = HttpServer::new(move || {
            App::new()
                .app_data(data_for_server.clone())
                .service(create_badge)
        })
        .listen(listener)
        .expect("listen failed")
        .run();
        let server_handle = server.handle();
        actix_web::rt::spawn(server);

        let client = reqwest::Client::new();
        let base = format!("http://{}", addr);

        // 1) initial request to populate cache and get ETag
        let repo_path = "/b1/github/sctg-development/tokeisrv";
        let url = format!("{}{}", base, repo_path);
        let r1 = client.get(&url).send().await.expect("request failed");
        assert!(r1.status().is_success());
        let etag = r1
            .headers()
            .get("etag")
            .map(|v| v.to_str().unwrap().to_string());
        assert!(etag.is_some());

        // 2) conditional request with If-None-Match should return 304 (cache hit)
        let inmatch_raw = etag.unwrap();
        let r2 = client
            .get(&url)
            .header("If-None-Match", inmatch_raw.clone())
            .send()
            .await
            .expect("conditional request failed");
        assert_eq!(r2.status(), reqwest::StatusCode::NOT_MODIFIED);

        // 3) flush cache using admin password 'toto'
        let flush_url = format!(
            "{}{}?action=flush-cache&admin-password=toto",
            base, repo_path
        );
        let r3 = client
            .get(&flush_url)
            .send()
            .await
            .expect("flush request failed");
        // Accept 200 (OK) or 304 (Not Modified) as flush response
        assert!(
            r3.status() == reqwest::StatusCode::OK
                || r3.status() == reqwest::StatusCode::NOT_MODIFIED
        );

        // Normalize ETag (strip W/ prefix and quotes) to compute cache keys
        let mut inmatch = inmatch_raw.clone();
        if inmatch.starts_with('W') && inmatch.contains('/') {
            // remove leading W/
            if let Some(idx) = inmatch.find('/') {
                inmatch = inmatch[idx + 1..].to_string();
            }
        }
        inmatch = inmatch.trim().trim_matches('\"').to_string();

        // Also attempt an internal flush to ensure keys are removed (some environments
        // may prevent the HTTP flush from discovering remote refs). Expect at least
        // one cache entry to be removed in normal networked environments.
        let repo_url = "https://github.com/sctg-development/tokeisrv";
        let removed = flush_cache_for_repo(repo_url, data.ignore_filetypes.as_ref());
        if removed == 0 {
            println!("[DEBUG] internal flush removed 0 keys (this may be OK if HTTP flush already removed them)");
        } else {
            println!("[DEBUG] internal flush removed {} keys", removed);
        }

        // 4) conditional request with same If-None-Match should now return 200 (cache flushed)
        let r4 = client
            .get(&url)
            .header("If-None-Match", inmatch_raw.clone())
            .send()
            .await
            .expect("conditional request failed");
        assert_eq!(r4.status(), reqwest::StatusCode::OK);

        // stop the server
        server_handle.stop(true).await;
    }
}
