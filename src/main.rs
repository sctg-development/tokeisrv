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
}
// App configuration passed to handlers
#[derive(Clone)]
struct AppConfig {
    user_whitelist: Option<std::collections::HashSet<String>>,
}
use cached::{Cached, Return};
use csscolorparser::parse;
use once_cell::sync::Lazy;
use rsbadges::{Badge, Style};
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

    let app_config = web::Data::new(AppConfig {
        user_whitelist: whitelist,
    });

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

    let url: &str = &format!("https://{}/{}/{}", domain, user, repo);

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

    let branch_name = if branch.is_empty() {
        head_branch.as_str()
    } else {
        &branch
    };
    // Find the oid for the requested branch
    let mut sha: String = String::new();
    let target_ref = format!("refs/heads/{}", branch_name);
    for r in refs.iter() {
        if r.name() == target_ref.as_str() {
            sha = r.oid().to_string();
            break;
        }
    }
    (sha.len() == HASH_LENGTH)
        .then(|| ())
        .ok_or_else(|| actix_web::error::ErrorBadRequest(eyre::eyre!("Invalid SHA provided.")))?;

    if let Ok(if_none_match) = IfNoneMatch::parse(&request) {
        log::debug!("Checking If-None-Match: {}#{}", sha, branch_name);
        let entity_tag: EntityTag = EntityTag::new(false, etag_identifier(&sha, branch_name));
        let found_match: bool = match if_none_match {
            IfNoneMatch::Any => false,
            IfNoneMatch::Items(items) => items
                .iter()
                .any(|etag: &EntityTag| etag.weak_eq(&entity_tag)),
        };

        if found_match {
            CACHE
                .lock()
                .unwrap()
                .cache_get(&repo_identifier(&url, &sha, branch_name));
            log::info!("{}#{}#{} Not Modified", url, sha, branch_name);
            return Ok(respond!(NotModified));
        }
    }

    let entry: Return<Vec<(LanguageType, Language)>> =
        get_statistics(&url, &sha, &branch_name).map_err(actix_web::error::ErrorBadRequest)?;

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
        etag_identifier(&sha, branch_name)
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
    create = "{ cached::TimedSizedCache::with_size_and_lifespan(1000, std::time::Duration::from_secs(DAY_IN_SECONDS)) }",
    convert = r#"{ repo_identifier(url, _sha, branch_name) }"#
)]
fn get_statistics(
    url: &str,
    _sha: &str,
    branch_name: &str,
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
    languages.get_statistics(&[temp_path], &[], &tokei::Config::default());

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
