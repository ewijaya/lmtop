//! CLI entry: argument parsing and the snapshot/doctor/TUI commands.

use crate::app::{App, View};
use crate::collectors::{
    Collector, ScanContext, claude::ClaudeCollector, codex::CodexCollector, custom::CustomCollector,
};
use crate::config::Config;
use crate::domain::{Provider, UsageSnapshot};
use chrono::Utc;
use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

/// lmtop — a live terminal monitor for language-model usage, quotas, and
/// subscription capacity. Local-first: reads session metadata from disk,
/// never prompt content, and requires no API keys.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Monitor a single provider (codex or claude).
    #[arg(long, global = true)]
    provider: Option<Provider>,

    /// Disable anything that would touch the network.
    #[arg(long, global = true)]
    offline: bool,

    /// Fetch live quota from the providers' own usage endpoints, using the
    /// access tokens their CLIs already store locally (equivalent to
    /// `network_quota = true` for every enabled provider).
    #[arg(long, global = true, conflicts_with = "offline")]
    live: bool,

    /// Collector refresh interval in seconds.
    #[arg(long, global = true)]
    refresh: Option<u64>,

    /// ASCII-only bars and charts (no unicode glyphs).
    #[arg(long, global = true)]
    ascii: bool,

    /// Path to an alternate config file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print a one-shot usage snapshot and exit (non-interactive).
    Snapshot {
        /// Emit machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Check provider discovery, parse health, and configuration.
    Doctor,
    /// Print a one-line usage summary for status bars (tmux, starship,
    /// waybar, Claude Code statusline) and exit.
    Line {
        /// Plain text without ANSI colors (also implied when stdout is not
        /// a terminal, unless --color is given).
        #[arg(long, conflicts_with = "color")]
        plain: bool,
        /// Force ANSI colors even when stdout is not a terminal.
        #[arg(long)]
        color: bool,
    },
}

pub fn run() -> Result<()> {
    color_eyre::install()?;
    init_tracing();
    let cli = Cli::parse();
    let (mut cfg, config_path) = Config::load(cli.config.as_deref())?;
    crate::domain::set_custom_provider_label(&cfg.providers.custom.name);

    // CLI flags override config.
    if let Some(refresh) = cli.refresh {
        cfg.ui.refresh_secs = refresh.max(1);
    }
    if cli.ascii {
        cfg.ui.ascii = true;
    }
    if cli.offline {
        cfg.ui.offline = true;
    }
    if cli.live {
        cfg.providers.codex.network_quota = true;
        cfg.providers.claude.network_quota = true;
    }
    // Offline always wins over network_quota, whatever their sources.
    if cfg.ui.offline {
        cfg.providers.codex.network_quota = false;
        cfg.providers.claude.network_quota = false;
    }
    if let Some(provider) = cli.provider {
        cfg.providers.codex.enabled &= provider == Provider::Codex;
        cfg.providers.claude.enabled &= provider == Provider::Claude;
        cfg.providers.custom.enabled &= provider == Provider::Custom;
    }

    match cli.command {
        Some(Command::Snapshot { json }) => snapshot_cmd(&cfg, json),
        Some(Command::Doctor) => doctor_cmd(&cfg, config_path),
        Some(Command::Line { plain, color }) => line_cmd(&cfg, plain, color),
        None => tui_cmd(cfg, cli.provider),
    }
}

/// Diagnostics go to a file only when the log env var is set; logging to
/// stderr would corrupt the TUI, and nothing is logged by default.
fn init_tracing() {
    if let Ok(filter) = std::env::var(crate::branding::LOG_ENV)
        && let Some(dirs) = directories::ProjectDirs::from("", "", crate::branding::APP_DIR)
    {
        let dir = dirs.cache_dir().to_path_buf();
        if std::fs::create_dir_all(&dir).is_ok()
            && let Ok(file) = std::fs::File::options()
                .create(true)
                .append(true)
                .open(dir.join(format!("{}.log", crate::branding::APP_NAME)))
        {
            use tracing_subscriber::EnvFilter;
            let _ = tracing_subscriber::fmt()
                .with_env_filter(EnvFilter::new(filter))
                .with_writer(file)
                .with_ansi(false)
                .try_init();
        }
    }
}

fn build_collectors(cfg: &Config) -> Vec<Box<dyn Collector>> {
    let mut collectors: Vec<Box<dyn Collector>> = Vec::new();
    if cfg.providers.codex.enabled {
        collectors.push(Box::new(CodexCollector::from_config(&cfg.providers.codex)));
    }
    if cfg.providers.claude.enabled {
        collectors.push(Box::new(ClaudeCollector::from_config(
            &cfg.providers.claude,
        )));
    }
    if cfg.providers.custom.enabled {
        collectors.push(Box::new(CustomCollector::from_config(
            &cfg.providers.custom,
        )));
    }
    collectors
}

fn scan_context(cfg: &Config, now: chrono::DateTime<Utc>) -> ScanContext {
    let (week_start, week_end) = crate::aggregation::week_bounds(
        now,
        cfg.time.week_start_day(),
        cfg.time.fixed_offset_hours(),
    );
    ScanContext {
        now,
        week_start,
        week_end,
        history_retention_minutes: cfg.history.retention_minutes.max(5),
    }
}

/// Run every enabled collector once, synchronously.
fn collect_once(cfg: &Config) -> UsageSnapshot {
    let now = Utc::now();
    let ctx = scan_context(cfg, now);
    let mut snapshot = UsageSnapshot::new(now);
    for mut collector in build_collectors(cfg) {
        let provider_snapshot = collector.scan(&ctx);
        snapshot
            .providers
            .insert(provider_snapshot.provider, provider_snapshot);
    }
    snapshot
}

fn snapshot_cmd(cfg: &Config, json: bool) -> Result<()> {
    let snapshot = collect_once(cfg);
    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        print!("{}", render_snapshot_text(&snapshot));
    }
    Ok(())
}

fn render_snapshot_text(snapshot: &UsageSnapshot) -> String {
    use crate::tui::theme::fmt_tokens;
    let mut out = String::new();
    out.push_str(&format!(
        "{} snapshot @ {}\n\n",
        crate::branding::APP_NAME,
        snapshot
            .generated_at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S %Z")
    ));
    for (provider, snap) in &snapshot.providers {
        out.push_str(&format!(
            "[{}]  status: {:?}\n",
            provider.display_name(),
            snap.health.status
        ));
        if let Some(msg) = &snap.health.message {
            out.push_str(&format!("  note:         {msg}\n"));
        }
        if snap.quota_windows.is_empty() {
            out.push_str("  quota:        unavailable\n");
        }
        for w in &snap.quota_windows {
            // The window rolled over since this snapshot was captured; its
            // percentage describes a finished window, not the current one.
            if w.is_expired(snapshot.generated_at) {
                let reset = w
                    .resets_at
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%m-%d %H:%M %Z")
                            .to_string()
                    })
                    .unwrap_or_else(|| "?".into());
                out.push_str(&format!(
                    "  quota {:<8} stale — window reset {} (last seen {:.1}%)\n",
                    w.label(),
                    reset,
                    w.used_percent
                ));
                continue;
            }
            let reset = w
                .resets_at
                .map(|t| {
                    format!(
                        ", resets {}",
                        t.with_timezone(&chrono::Local).format("%m-%d %H:%M %Z")
                    )
                })
                .unwrap_or_default();
            let confidence = w
                .trend_confidence
                .map(|c| format!(" (confidence: {})", c.label()))
                .unwrap_or_default();
            let outlook = match w.outlook() {
                crate::domain::QuotaOutlook::Exhausted => " — EXHAUSTED".to_string(),
                crate::domain::QuotaOutlook::AtRisk {
                    projected_exhaustion,
                } => format!(
                    " — AT RISK: projected empty {}{}",
                    projected_exhaustion
                        .with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M %Z"),
                    confidence
                ),
                crate::domain::QuotaOutlook::Lasts => {
                    format!(" — lasts to reset{confidence}")
                }
                crate::domain::QuotaOutlook::Unknown => String::new(),
            };
            let burn = w
                .burn_per_hour
                .map(|b| format!(" ({b:.1}%/h)"))
                .unwrap_or_default();
            let age_secs = snapshot
                .generated_at
                .signed_duration_since(w.captured_at)
                .num_seconds();
            let staleness = if age_secs > 600 {
                format!(
                    " [as of {}]",
                    w.captured_at
                        .with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M %Z")
                )
            } else {
                String::new()
            };
            out.push_str(&format!(
                "  quota {:<8} {:.1}% used{}{}{}{}\n",
                w.label(),
                w.used_percent,
                burn,
                reset,
                outlook,
                staleness
            ));
        }
        if let Some(c) = &snap.credits {
            out.push_str(&format!("  credits:      {:.0}\n", c.balance));
        }
        if let Some(week) = &snap.week {
            out.push_str(&format!(
                "  week:         {} total (in {} / cached {} / cachew {} / out {}), {} sessions\n",
                fmt_tokens(week.tokens.total()),
                fmt_tokens(week.tokens.input),
                fmt_tokens(week.tokens.cached_input),
                fmt_tokens(week.tokens.cache_creation),
                fmt_tokens(week.tokens.output),
                week.sessions
            ));
            for usage in week.by_model.values() {
                out.push_str(&format!(
                    "    {:<24} {}\n",
                    usage.model.display,
                    fmt_tokens(usage.tokens.total())
                ));
            }
        }
        let session_total = snap.current_session_tokens.total();
        if session_total > 0 {
            out.push_str(&format!(
                "  active now:   {} tokens across active sessions\n",
                fmt_tokens(session_total)
            ));
        }
        out.push_str(&format!(
            "  sessions:     {} in the last 48h\n\n",
            snap.sessions.len()
        ));
    }
    out
}

fn line_cmd(cfg: &Config, plain: bool, force_color: bool) -> Result<()> {
    use std::io::IsTerminal;
    let snapshot = collect_once(cfg);
    let color = !plain && (force_color || std::io::stdout().is_terminal());
    println!("{}", render_line(&snapshot, color, Utc::now()));
    Ok(())
}

/// One-line summary for status bars: per provider, every quota window plus
/// the worst outlook marker. Designed to stay useful uncolored.
fn render_line(snapshot: &UsageSnapshot, color: bool, now: chrono::DateTime<Utc>) -> String {
    use crate::domain::QuotaOutlook;
    const RESET: &str = "\x1b[0m";
    const DIM: &str = "\x1b[2m";
    let pct_color = |pct: f64| -> &'static str {
        if pct >= 90.0 {
            "\x1b[31m"
        } else if pct >= 70.0 {
            "\x1b[33m"
        } else {
            "\x1b[32m"
        }
    };
    let provider_color = |p: Provider| -> &'static str {
        match p {
            Provider::Codex => "\x1b[38;5;75m",
            Provider::Claude => "\x1b[38;5;208m",
            Provider::Custom => "\x1b[38;5;79m",
        }
    };
    let paint = |code: &str, text: &str| -> String {
        if color {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    };

    let mut parts: Vec<String> = Vec::new();
    for (provider, snap) in &snapshot.providers {
        let mut fields = vec![paint(provider_color(*provider), provider.display_name())];
        if snap.quota_windows.is_empty() {
            fields.push(paint(DIM, "n/a"));
        }
        let mut worst: Option<String> = None;
        for w in &snap.quota_windows {
            let label = match (&w.kind, &w.scope) {
                (crate::domain::QuotaWindowKind::FiveHour, _) => "5h".to_string(),
                (crate::domain::QuotaWindowKind::Weekly, None) => "wk".to_string(),
                (crate::domain::QuotaWindowKind::Weekly, Some(s)) => format!("wk·{s}"),
                _ => w.label(),
            };
            if w.is_expired(now) {
                fields.push(format!("{label} {}", paint(DIM, "stale")));
                continue;
            }
            fields.push(format!(
                "{label} {}",
                paint(
                    pct_color(w.used_percent),
                    &format!("{:.0}%", w.used_percent)
                )
            ));
            match w.outlook() {
                QuotaOutlook::Exhausted => {
                    worst = Some(paint("\x1b[31m", "✗ exhausted"));
                }
                QuotaOutlook::AtRisk {
                    projected_exhaustion,
                } if worst.is_none() => {
                    let eta = crate::tui::theme::fmt_duration_until(
                        projected_exhaustion
                            .signed_duration_since(now)
                            .num_seconds(),
                    );
                    worst = Some(paint("\x1b[33m", &format!("⚠ ~{eta}")));
                }
                _ => {}
            }
        }
        if let Some(w) = worst {
            fields.push(w);
        }
        parts.push(fields.join(" "));
    }
    if parts.is_empty() {
        return "no providers enabled".to_string();
    }
    parts.join(&paint(DIM, " · "))
}

fn doctor_cmd(cfg: &Config, config_path: Option<PathBuf>) -> Result<()> {
    let now = Utc::now();
    let ctx = scan_context(cfg, now);
    let mut providers = Vec::new();

    let mut codex = CodexCollector::from_config(&cfg.providers.codex);
    let codex_snapshot = codex.scan(&ctx);
    providers.push(crate::diagnostics::ProviderDoctor::from_snapshot(
        codex.discovery_info(),
        cfg.providers.codex.enabled,
        &codex_snapshot,
    ));

    let mut claude = ClaudeCollector::from_config(&cfg.providers.claude);
    let claude_snapshot = claude.scan(&ctx);
    providers.push(crate::diagnostics::ProviderDoctor::from_snapshot(
        claude.discovery_info(),
        cfg.providers.claude.enabled,
        &claude_snapshot,
    ));

    // The custom provider only appears in doctor when actually configured.
    if cfg.providers.custom.source.is_some() || cfg.providers.custom.command.is_some() {
        let mut custom = CustomCollector::from_config(&cfg.providers.custom);
        let custom_snapshot = custom.scan(&ctx);
        providers.push(crate::diagnostics::ProviderDoctor::from_snapshot(
            custom.discovery_info(),
            cfg.providers.custom.enabled,
            &custom_snapshot,
        ));
    }

    let report = crate::diagnostics::DoctorReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        config_path: config_path.map(|p| {
            let shown = crate::diagnostics::redact_path(&p.display().to_string());
            if p.exists() {
                shown
            } else {
                format!("{shown} (not present, using defaults)")
            }
        }),
        providers,
    };
    print!("{}", report.render_text());
    Ok(())
}

fn tui_cmd(cfg: Config, provider_filter: Option<Provider>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let control = crate::tui::CollectorControl::new();

    // Restore the terminal even on SIGTERM/SIGHUP; SIGINT arrives as a
    // Ctrl+C key event in raw mode and quits gracefully, and panics are
    // covered by ratatui's panic hook.
    #[cfg(unix)]
    runtime.spawn(async {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = hup.recv() => {}
        }
        ratatui::restore();
        std::process::exit(130);
    });

    for collector in build_collectors(&cfg) {
        let tx = tx.clone();
        let control = control.clone();
        let cfg = cfg.clone();
        runtime.spawn(async move {
            collector_loop(collector, tx, control, cfg).await;
        });
    }
    drop(tx);

    let theme = crate::tui::theme::Theme::named(cfg.ui.ascii, &cfg.ui.theme);
    let now = Utc::now();
    let mut app = App::new(now, cfg.ui.refresh_secs);
    app.view = match provider_filter {
        Some(Provider::Codex) => View::Codex,
        Some(Provider::Claude) => View::Claude,
        Some(Provider::Custom) | None => View::Combined,
    };
    app.reduced_motion = cfg.ui.reduced_motion;
    app.enabled_providers = Provider::ALL
        .iter()
        .copied()
        .filter(|p| match p {
            Provider::Codex => cfg.providers.codex.enabled,
            Provider::Claude => cfg.providers.claude.enabled,
            Provider::Custom => cfg.providers.custom.enabled,
        })
        .collect();
    if cfg.history.persist
        && let Some(path) = crate::persist::HistoryStore::default_path()
    {
        app.history = crate::persist::HistoryStore::open(path, cfg.history.retention_days, now);
    }
    let alert_engine = crate::alerts::AlertEngine::new(cfg.alerts.clone());
    let result = crate::tui::run(app, theme, rx, control, alert_engine);
    runtime.shutdown_background();
    result
}

async fn collector_loop(
    mut collector: Box<dyn Collector>,
    tx: tokio::sync::mpsc::Sender<crate::domain::ProviderSnapshot>,
    control: crate::tui::CollectorControl,
    cfg: Config,
) {
    let refresh = std::time::Duration::from_secs(cfg.ui.refresh_secs.max(1));
    loop {
        if !control.paused.load(Ordering::Relaxed) {
            let ctx = scan_context(&cfg, Utc::now());
            // File scanning is blocking work; keep it off the async threads.
            let Ok((returned, snapshot)) = tokio::task::spawn_blocking(move || {
                let snapshot = collector.scan(&ctx);
                (collector, snapshot)
            })
            .await
            else {
                return; // scan task panicked; stop this collector loop
            };
            collector = returned;
            if tx.send(snapshot).await.is_err() {
                return; // UI is gone.
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(refresh) => {}
            _ = control.refresh_now.notified() => {}
        }
    }
}
