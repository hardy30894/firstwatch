use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use firstwatch_schema::{read_ndjson, write_ndjson, ChangeEvent, Impact, Severity};

#[derive(Parser)]
#[command(name = "firstwatch", version, about = "First eyes on upstream API changes.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Diff two OpenAPI specs, emit ChangeEvents as ndjson.
    Watch {
        #[arg(long)]
        old: PathBuf,
        #[arg(long)]
        new: PathBuf,
        #[arg(long, default_value = "stripe")]
        provider: String,
    },
    /// Scan a codebase for provider SDK call sites, emit UsageSites as ndjson.
    Scan {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Join a changefeed against usage sites, emit Impacts as ndjson.
    Match {
        /// ChangeEvent ndjson file.
        #[arg(long)]
        events: PathBuf,
        /// UsageSite ndjson file. Reads stdin when omitted.
        #[arg(long)]
        sites: Option<PathBuf>,
    },
    /// Scan the codebase, match against a changefeed, and print a report.
    Check {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        events: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Watch { old, new, provider } => {
            let old: serde_json::Value = serde_json::from_reader(BufReader::new(File::open(old)?))?;
            let new: serde_json::Value = serde_json::from_reader(BufReader::new(File::open(new)?))?;
            let events = firstwatch_watch::diff_openapi(&old, &new, &provider)?;
            write_ndjson(io::stdout().lock(), &events)?;
        }
        Command::Scan { path } => {
            let sites = firstwatch_scan::scan_dir(&path)?;
            write_ndjson(io::stdout().lock(), &sites)?;
        }
        Command::Match { events, sites } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(events)?))?;
            let sites = match sites {
                Some(path) => read_ndjson(BufReader::new(File::open(path)?))?,
                None => read_ndjson(io::stdin().lock())?,
            };
            let impacts = firstwatch_match::match_impacts(&events, &sites);
            write_ndjson(io::stdout().lock(), &impacts)?;
        }
        Command::Check { path, events } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(events)?))?;
            let sites = firstwatch_scan::scan_dir(&path)?;
            let impacts = firstwatch_match::match_impacts(&events, &sites);
            report(&impacts, sites.len());
        }
    }
    Ok(())
}

fn report(impacts: &[Impact], scanned_sites: usize) {
    if impacts.is_empty() {
        println!("firstwatch: {scanned_sites} call sites scanned, nothing affected.");
        return;
    }
    for impact in impacts {
        let tag = match impact.severity {
            Severity::BreaksNow => "BREAKS NOW",
            Severity::BreaksLater => "BREAKS LATER",
            Severity::Deprecated => "DEPRECATED",
            Severity::Opportunity => "NEW",
        };
        println!(
            "[{tag}] {}:{} {} ({:?} {})",
            impact.site.file,
            impact.site.line,
            impact.site.surface,
            impact.change.kind,
            impact.change.field.as_deref().unwrap_or("")
        );
        if let Some(snippet) = &impact.site.snippet {
            println!("    {snippet}");
        }
    }
    let breaking = impacts
        .iter()
        .filter(|i| matches!(i.severity, Severity::BreaksNow | Severity::BreaksLater))
        .count();
    println!(
        "firstwatch: {} impacts across {} call sites, {} breaking.",
        impacts.len(),
        scanned_sites,
        breaking
    );
}
