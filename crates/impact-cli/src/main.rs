mod mcp;
mod ops;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use impact_core::{CrossProjectMatch, ImpactReport};

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
        /// Print machine-readable JSON instead of the tree-text report.
        #[arg(long)]
        json: bool,
    },
    /// Run the MCP stdio server, exposing `impact_index`/`impact_file`/`impact_change` as
    /// tools for an MCP-speaking agent. Blocks until stdin closes.
    Mcp,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Index {
            path,
            json,
            cache_dir,
            force,
        } => run_index(&path, cache_dir.as_deref(), force, json),
        Command::Query {
            path,
            project,
            cache_dir,
            workspace,
            json,
        } => run_query(
            &path,
            project.as_deref(),
            cache_dir.as_deref(),
            workspace.as_deref(),
            json,
        ),
        Command::Change {
            description,
            project,
            cache_dir,
            workspace,
            json,
        } => run_change(
            &description,
            project.as_deref(),
            cache_dir.as_deref(),
            workspace.as_deref(),
            json,
        ),
        Command::Mcp => mcp::run(),
    }
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

fn run_query(
    path: &Path,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
    workspace: Option<&Path>,
    json: bool,
) -> anyhow::Result<()> {
    let local = ops::query_file(path, project, cache_dir)?;
    print_report(local, project, workspace, json)
}

fn run_change(
    description: &str,
    project: Option<&Path>,
    cache_dir: Option<&Path>,
    workspace: Option<&Path>,
    json: bool,
) -> anyhow::Result<()> {
    let local = ops::apply_change(description, project, cache_dir)?;
    print_report(local, project, workspace, json)
}

fn print_report(
    local: ImpactReport,
    project: Option<&Path>,
    workspace: Option<&Path>,
    json: bool,
) -> anyhow::Result<()> {
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

fn print_tree_text(report: &ImpactReport) {
    println!("DIRECT");
    for name in &report.direct {
        println!("  {name}");
    }
    println!("INDIRECT");
    for name in &report.indirect {
        println!("  {name}");
    }
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
