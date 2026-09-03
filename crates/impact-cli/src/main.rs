mod analytics;
mod install;
mod mcp;
mod ops;

use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use impact_core::{Confidence, CrossProjectMatch, ImpactReport};

/// CLI-facing mirror of `impact_core::Confidence`'s two tiers a user would realistically
/// filter on. `--min-confidence exact` keeps only unambiguous dependents; the default
/// (no flag) shows everything, `Heuristic` included.
#[derive(Clone, Copy, ValueEnum)]
enum MinConfidence {
    Exact,
    Heuristic,
}

impl From<MinConfidence> for Confidence {
    fn from(value: MinConfidence) -> Self {
        match value {
            MinConfidence::Exact => Confidence::Exact,
            MinConfidence::Heuristic => Confidence::Heuristic,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "impact",
    version,
    about = "Tell you what you're about to break, before you break it."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a project into its local cache.
    Index {
        /// Project root to index.
        path: PathBuf,
        /// Print machine-readable JSON instead of a human summary.
        #[arg(long)]
        json: bool,
        /// Where to store the index cache (defaults to `<path>/.impact`).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// Wipe the existing cache and fully re-index, ignoring content-hash skips.
        #[arg(long)]
        force: bool,
    },
    /// Report the blast radius of changing a file: who calls into it, directly or
    /// transitively. Requires the project to have been indexed already.
    Query {
        /// File to compute the blast radius for, relative to the project root or
        /// absolute — either way, resolved against `--project`.
        path: PathBuf,
        /// Project root the cache was built against (defaults to the current directory).
        #[arg(long)]
        project: Option<PathBuf>,
        /// Where the index cache lives (defaults to `<project>/.impact`).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// A `workspace.toml` registering sibling projects — when given, also reports
        /// which of them this file's API routes/events/tables touch.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Only show DIRECT/INDIRECT dependents resolved with at least this confidence —
        /// `exact` hides anything the linker could only match by ambiguous short name.
        #[arg(long)]
        min_confidence: Option<MinConfidence>,
        /// Show each INDIRECT entry's chain back to its nearest DIRECT dependent, so a
        /// `[heuristic]` entry can be checked without re-reading code.
        #[arg(long)]
        explain: bool,
        /// Print machine-readable JSON instead of the tree-text report.
        #[arg(long)]
        json: bool,
    },
    /// Report the deterministic blast radius of a specific change, e.g.
    /// `impact change "rename PaymentStatus::Failed"` or
    /// `impact change "remove variant PaymentStatus::Failed"`. Requires the project to
    /// have been indexed already.
    Change {
        /// The change description — see the module doc on `impact_core::change` for the
        /// full accepted grammar.
        description: String,
        /// Project root the cache was built against (defaults to the current directory).
        #[arg(long)]
        project: Option<PathBuf>,
        /// Where the index cache lives (defaults to `<project>/.impact`).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// A `workspace.toml` registering sibling projects — when given, also reports
        /// which of them this change's API routes/events/tables touch.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Only show DIRECT/INDIRECT dependents resolved with at least this confidence —
        /// `exact` hides anything the linker could only match by ambiguous short name.
        #[arg(long)]
        min_confidence: Option<MinConfidence>,
        /// Show each INDIRECT entry's chain back to its nearest DIRECT dependent, so a
        /// `[heuristic]` entry can be checked without re-reading code.
        #[arg(long)]
        explain: bool,
        /// Print machine-readable JSON instead of the tree-text report.
        #[arg(long)]
        json: bool,
    },
    /// Report the blast radius of an unpushed/uncommitted change, given as a unified diff
    /// (`git diff` output) — e.g. `git diff | impact diff`. Maps each touched line to the
    /// symbol it falls inside (see `impact_core::compute_diff_impact`) and reports the
    /// combined blast radius across every touched symbol in every file the diff mentions.
    /// Requires the project to have been indexed against the diff's *new* side — i.e. the
    /// working tree as it currently stands, which is what `git diff` on uncommitted
    /// changes already matches.
    Diff {
        /// Read the diff from this file instead of stdin.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Project root the cache was built against (defaults to the current directory).
        #[arg(long)]
        project: Option<PathBuf>,
        /// Where the index cache lives (defaults to `<project>/.impact`).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// A `workspace.toml` registering sibling projects — when given, also reports
        /// which of them this diff's API routes/events/tables touch.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Only show DIRECT/INDIRECT dependents resolved with at least this confidence —
        /// `exact` hides anything the linker could only match by ambiguous short name.
        #[arg(long)]
        min_confidence: Option<MinConfidence>,
        /// Show each INDIRECT entry's chain back to its nearest DIRECT dependent, so a
        /// `[heuristic]` entry can be checked without re-reading code.
        #[arg(long)]
        explain: bool,
        /// Print machine-readable JSON instead of the tree-text report.
        #[arg(long)]
        json: bool,
    },
    /// Run the MCP stdio server, exposing `impact_index`/`impact_file`/`impact_change`/
    /// `impact_diff` as tools for an MCP-speaking agent. Blocks until stdin closes.
    Mcp,
    /// Register the impact MCP server, and an agent rule telling the agent to verify
    /// blast radius with it before/after editing code, with local AI coding tools.
    Install {
        /// Client(s) to configure: cursor, codex, claude, claude-desktop, or all.
        #[arg(long, default_value = "all")]
        client: String,
        /// Config scope: user (global — works in every project) or project.
        #[arg(long, default_value = "user")]
        scope: String,
        /// Project directory for project-scoped config (defaults to the current
        /// directory). Codex and Claude Code only honor this for their rule file — their
        /// MCP registration is always user-scoped, by design (see `impact doctor`).
        #[arg(long)]
        path: Option<PathBuf>,
        /// Override the resolved home directory. Mainly for portable or non-standard
        /// user-profile setups; defaults to the OS user profile directory.
        #[arg(long)]
        home_dir: Option<PathBuf>,
        /// Preview changes without writing any files.
        #[arg(long)]
        dry_run: bool,
        /// Skip installing the agent rule file/block.
        #[arg(long)]
        no_rule: bool,
        /// Print machine-readable JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
    /// Remove the impact MCP server and rule file/block from local AI coding tools.
    Uninstall {
        #[arg(long, default_value = "all")]
        client: String,
        #[arg(long, default_value = "user")]
        scope: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        home_dir: Option<PathBuf>,
        /// Preview changes without writing any files.
        #[arg(long)]
        dry_run: bool,
        /// Skip removing the agent rule file/block.
        #[arg(long)]
        no_rule: bool,
        #[arg(long)]
        json: bool,
    },
    /// Report whether each AI coding tool has impact registered and its rule installed.
    Doctor {
        #[arg(long, default_value = "all")]
        client: String,
        #[arg(long, default_value = "user")]
        scope: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        home_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show recorded usage of `index`/`query`/`change`/`diff` (CLI and MCP combined),
    /// rolled up by day/week/month and broken down by client. Defaults to monthly.
    Gain {
        /// Roll up by day.
        #[arg(long)]
        daily: bool,
        /// Roll up by week.
        #[arg(long)]
        weekly: bool,
        /// Roll up by month (default when no period flag is given).
        #[arg(long)]
        monthly: bool,
        /// Print machine-readable JSON instead of a human summary.
        #[arg(long)]
        json: bool,
    },
}

/// Times `f`, records a CLI usage event for `command` (client from `IMPACT_CLIENT`,
/// default `"cli"`), and returns `f`'s result unchanged — recording must never change a
/// command's exit code or output.
fn with_usage_recorded<T>(
    command: &'static str,
    f: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let start = std::time::Instant::now();
    let result = f();
    let client = std::env::var("IMPACT_CLIENT").unwrap_or_else(|_| "cli".to_string());
    analytics::record_new(analytics::UsageEvent {
        command,
        source: "cli",
        client,
        client_version: None,
        duration_ms: start.elapsed().as_millis() as u64,
        success: result.is_ok(),
    });
    result
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Index {
            path,
            json,
            cache_dir,
            force,
        } => with_usage_recorded("index", || {
            run_index(&path, cache_dir.as_deref(), force, json)
        }),
        Command::Query {
            path,
            project,
            cache_dir,
            workspace,
            min_confidence,
            explain,
            json,
        } => with_usage_recorded("file", || {
            run_query(
                &path,
                project.as_deref(),
                cache_dir.as_deref(),
                workspace.as_deref(),
                min_confidence,
                explain,
                json,
            )
        }),
        Command::Change {
            description,
            project,
            cache_dir,
            workspace,
            min_confidence,
            explain,
            json,
        } => with_usage_recorded("change", || {
            run_change(
                &description,
                project.as_deref(),
                cache_dir.as_deref(),
                workspace.as_deref(),
                min_confidence,
                explain,
                json,
            )
        }),
        Command::Diff {
            file,
            project,
            cache_dir,
            workspace,
            min_confidence,
            explain,
            json,
        } => with_usage_recorded("diff", || {
            run_diff(
                file.as_deref(),
                project.as_deref(),
                cache_dir.as_deref(),
                workspace.as_deref(),
                min_confidence,
                explain,
                json,
            )
        }),
        Command::Mcp => mcp::run(),
        Command::Install {
            client,
            scope,
            path,
            home_dir,
            dry_run,
            no_rule,
            json,
        } => run_install(&client, &scope, path, home_dir, dry_run, no_rule, json),
        Command::Uninstall {
            client,
            scope,
            path,
            home_dir,
            dry_run,
            no_rule,
            json,
        } => run_uninstall(&client, &scope, path, home_dir, dry_run, no_rule, json),
        Command::Doctor {
            client,
            scope,
            path,
            home_dir,
            json,
        } => run_doctor(&client, &scope, path, home_dir, json),
        Command::Gain {
            daily,
            weekly,
            monthly,
            json,
        } => run_gain(daily, weekly, monthly, json),
    }
}

fn run_gain(daily: bool, weekly: bool, monthly: bool, json: bool) -> anyhow::Result<()> {
    if [daily, weekly, monthly].iter().filter(|f| **f).count() > 1 {
        anyhow::bail!("--daily, --weekly, and --monthly are mutually exclusive");
    }
    let period = if daily {
        analytics::Period::Daily
    } else if weekly {
        analytics::Period::Weekly
    } else {
        analytics::Period::Monthly
    };

    let conn = analytics::open()?;
    let buckets = analytics::rollup(&conn, period, 12)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&buckets)?);
        return Ok(());
    }
    if buckets.is_empty() {
        println!("no usage recorded yet");
        return Ok(());
    }
    let color = std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal();
    for (i, bucket) in buckets.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print_gain_bucket(bucket, color);
    }
    Ok(())
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_CYAN: &str = "\x1b[96m";
const ANSI_YELLOW: &str = "\x1b[33m";
const GAIN_BAR_WIDTH: usize = 24;

fn styled(text: &str, code: &str, color: bool) -> String {
    if color {
        format!("{code}{text}{ANSI_RESET}")
    } else {
        text.to_string()
    }
}

/// A thousands-grouped `u64` (`12345` -> `"12,345"`) — the only formatting `impact gain`
/// needs beyond alignment, so no separate crate for it.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",")
}

/// A `width`-cell bar, `pct`% filled with `█`, the rest `░` — dimmed when unfilled so the
/// filled portion (optionally colored) reads at a glance.
fn gain_bar(pct: f64, color: bool) -> String {
    let filled = ((pct / 100.0) * GAIN_BAR_WIDTH as f64).round() as usize;
    let filled = filled.min(GAIN_BAR_WIDTH);
    let empty = GAIN_BAR_WIDTH - filled;
    format!(
        "{}{}",
        styled(&"█".repeat(filled), ANSI_CYAN, color),
        styled(&"░".repeat(empty), ANSI_DIM, color)
    )
}

fn print_gain_bucket(bucket: &analytics::Bucket, color: bool) {
    let title = format!("{}  ·  {} calls", bucket.label, grouped(bucket.total));
    let failed = bucket.total - bucket.success;
    let title = if failed > 0 {
        format!(
            "{title}  ·  {}",
            styled(&format!("{failed} failed"), ANSI_YELLOW, color)
        )
    } else {
        title
    };
    println!("{}", styled(&title, ANSI_BOLD, color));
    println!(
        "{}",
        styled(&"─".repeat(title.chars().count().max(28)), ANSI_DIM, color)
    );
    print_gain_breakdown("BY CLIENT", &bucket.by_client, bucket.total, color);
    println!();
    print_gain_breakdown("BY COMMAND", &bucket.by_command, bucket.total, color);
}

fn print_gain_breakdown(title: &str, entries: &[(String, u64)], total: u64, color: bool) {
    println!("  {}", styled(title, ANSI_BOLD, color));
    let name_width = entries
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0);
    let count_width = entries
        .iter()
        .map(|(_, count)| grouped(*count).len())
        .max()
        .unwrap_or(1);
    for (name, count) in entries {
        let pct = if total > 0 {
            *count as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        let count_str = grouped(*count);
        println!(
            "    {name:<name_width$}  {}  {count_str:>count_width$}  {}",
            gain_bar(pct, color),
            styled(&format!("{pct:>5.1}%"), ANSI_DIM, color),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn run_install(
    client: &str,
    scope: &str,
    path: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    dry_run: bool,
    no_rule: bool,
    json: bool,
) -> anyhow::Result<()> {
    let mut options = install::build_options(client, scope, path, home_dir)?;
    options.dry_run = dry_run;
    options.install_rule = !no_rule;
    let report = install::install_clients(&options)?;
    print_install_report(
        if dry_run { "would update" } else { "updated" },
        &report,
        json,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_uninstall(
    client: &str,
    scope: &str,
    path: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    dry_run: bool,
    no_rule: bool,
    json: bool,
) -> anyhow::Result<()> {
    let mut options = install::build_options(client, scope, path, home_dir)?;
    options.dry_run = dry_run;
    options.install_rule = !no_rule;
    let report = install::uninstall_clients(&options)?;
    print_install_report(
        if dry_run { "would update" } else { "updated" },
        &report,
        json,
    )
}

fn run_doctor(
    client: &str,
    scope: &str,
    path: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    json: bool,
) -> anyhow::Result<()> {
    let options = install::build_options(client, scope, path, home_dir)?;
    let report = install::doctor(&options)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for status in &report.clients {
            println!("{}", status.client.name());
            println!(
                "  config:  {} ({})",
                status.path.display(),
                if status.configured {
                    "configured"
                } else {
                    "missing"
                }
            );
            if status.configured && !status.points_to_expected_binary {
                println!("           points at a different binary than this one");
            }
            let rule_state = if !status.rule_installed {
                "missing"
            } else if status.rule_current {
                "up to date"
            } else {
                "present, out of date"
            };
            println!("  rule:    {} ({})", status.rule_path.display(), rule_state);
        }
    }
    Ok(())
}

fn print_install_report(
    action: &str,
    report: &install::InstallReport,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    for path in &report.changed {
        println!("{action} {}", path.display());
    }
    for path in &report.rule_changed {
        println!("{action} {}", path.display());
    }
    if report.changed.is_empty() && report.rule_changed.is_empty() {
        println!("nothing to do — already up to date");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_diff(
    file: Option<&Path>,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
    workspace: Option<&Path>,
    min_confidence: Option<MinConfidence>,
    explain: bool,
    json: bool,
) -> anyhow::Result<()> {
    let diff_text = match file {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
        }
        None => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .context("reading diff from stdin")?;
            buf
        }
    };

    let local = ops::diff_impact(&diff_text, project, cache_dir)?;
    print_report(local, project, workspace, min_confidence, explain, json)
}

fn run_index(path: &Path, cache_dir: Option<&Path>, force: bool, json: bool) -> anyhow::Result<()> {
    let stats = ops::index_project(path, cache_dir, force)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!(
            "Indexed {} files ({} unchanged, skipped), {} symbols, in {}ms",
            stats.files_indexed, stats.files_skipped, stats.symbols_indexed, stats.duration_ms
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_query(
    path: &Path,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
    workspace: Option<&Path>,
    min_confidence: Option<MinConfidence>,
    explain: bool,
    json: bool,
) -> anyhow::Result<()> {
    let local = ops::query_file(path, project, cache_dir)?;
    print_report(local, project, workspace, min_confidence, explain, json)
}

#[allow(clippy::too_many_arguments)]
fn run_change(
    description: &str,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
    workspace: Option<&Path>,
    min_confidence: Option<MinConfidence>,
    explain: bool,
    json: bool,
) -> anyhow::Result<()> {
    let local = ops::apply_change(description, project, cache_dir)?;
    print_report(local, project, workspace, min_confidence, explain, json)
}

#[allow(clippy::too_many_arguments)]
fn print_report(
    local: ImpactReport,
    project: Option<&Path>,
    workspace: Option<&Path>,
    min_confidence: Option<MinConfidence>,
    explain: bool,
    json: bool,
) -> anyhow::Result<()> {
    let local = match min_confidence {
        Some(min) => impact_core::filter_min_confidence(local, min.into()),
        None => local,
    };
    let local = impact_core::apply_explain(local, explain);
    match workspace {
        None => {
            if json {
                println!("{}", serde_json::to_string_pretty(&local)?);
            } else {
                print_tree_text(&local);
            }
        }
        Some(workspace_path) => {
            let report = ops::cross_project_report(local, project, workspace_path)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_tree_text(&report.local);
                print_cross_project_text(&report.cross_project);
            }
        }
    }
    Ok(())
}

fn print_dependents(dependents: &[impact_core::Dependent]) {
    for d in dependents {
        let location = if d.file.is_empty() {
            String::new()
        } else {
            format!("  {}:{}", d.file, d.line)
        };
        match d.confidence {
            Confidence::Exact => println!("  {}{location}", d.path),
            Confidence::Probable => println!("  {}{location} [probable]", d.path),
            Confidence::Heuristic => println!("  {}{location} [heuristic]", d.path),
        }
        if !d.via.is_empty() {
            println!("    via {}", d.via.join(" -> "));
        }
    }
}

fn print_tree_text(report: &ImpactReport) {
    println!("DIRECT");
    print_dependents(&report.direct);
    println!("INDIRECT");
    print_dependents(&report.indirect);
    println!("API");
    for name in &report.api {
        println!("  {name}");
    }
    println!("EVENTS");
    for name in &report.events {
        println!("  {name}");
    }
    println!("DATABASE");
    for name in &report.database {
        println!("  {name}");
    }
    println!("TESTS");
    println!("  {} affected tests", report.tests);
    print_dependents(&report.affected_tests);
}

fn print_cross_project_text(matches: &[CrossProjectMatch]) {
    println!("CROSS-PROJECT");
    for m in matches {
        println!(
            "  [{:?}] {} ({:?}: {})",
            m.confidence, m.project_id, m.contract_kind, m.contract_id
        );
    }
}
