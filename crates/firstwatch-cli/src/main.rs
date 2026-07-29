mod pr;

use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use firstwatch_scan::ProviderSet;
use firstwatch_schema::{read_ndjson, write_ndjson, ChangeEvent, Impact, Severity};

#[derive(Parser)]
#[command(
    name = "firstwatch",
    version,
    about = "First eyes on upstream API changes."
)]
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
        /// ProviderSpec ndjson extending the built-in provider table.
        #[arg(long)]
        providers: Option<PathBuf>,
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
        /// ProviderSpec ndjson extending the built-in provider table.
        #[arg(long)]
        providers: Option<PathBuf>,
    },
    /// Propose deterministic fixes for impacted call sites. Dry run unless --write.
    Fix {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        events: PathBuf,
        /// Recipe ndjson file with provider-authored fixes.
        #[arg(long)]
        recipes: Option<PathBuf>,
        /// Also run the project's test suite as a verifier.
        #[arg(long)]
        with_tests: bool,
        /// Differential replay command, run in unpatched and patched
        /// stagings, requiring identical output.
        #[arg(long)]
        replay: Option<String>,
        #[arg(long)]
        write: bool,
    },
    /// Fix on a new branch, commit, push, and open a PR. Requires a clean worktree.
    Pr {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        events: PathBuf,
        /// Recipe ndjson file with provider-authored fixes.
        #[arg(long)]
        recipes: Option<PathBuf>,
        /// Also run the project's test suite as a verifier.
        #[arg(long)]
        with_tests: bool,
        /// Differential replay command, run in unpatched and patched
        /// stagings, requiring identical output.
        #[arg(long)]
        replay: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        draft: bool,
        /// Open the PR even when no verifier could prove the patches.
        #[arg(long)]
        allow_unproven: bool,
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
        Command::Scan { path, providers } => {
            let sites = firstwatch_scan::scan_dir(&path, &load_providers(providers)?)?;
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
        Command::Check {
            path,
            events,
            providers,
        } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(events)?))?;
            let sites = firstwatch_scan::scan_dir(&path, &load_providers(providers)?)?;
            let impacts = firstwatch_match::match_impacts(&events, &sites);
            report(&impacts, sites.len());
        }
        Command::Fix {
            path,
            events,
            recipes,
            with_tests,
            replay,
            write,
        } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(events)?))?;
            let sites = firstwatch_scan::scan_dir(&path, &ProviderSet::default())?;
            let impacts = firstwatch_match::match_impacts(&events, &sites);
            let recipes = load_recipes(recipes)?;
            let outcome = firstwatch_mend::mend(&path, &impacts, &recipes)?;
            for patch in &outcome.patches {
                println!("{}:{}", patch.file, patch.line);
                println!("- {}", patch.old_line.trim());
                println!("+ {}", patch.new_line.trim());
            }
            for refusal in &outcome.refusals {
                println!(
                    "[refused] {}:{} {} ({})",
                    refusal.impact.site.file,
                    refusal.impact.site.line,
                    refusal.impact.site.surface,
                    refusal.reason
                );
            }
            if !write {
                println!(
                    "firstwatch: {} patches proposed, {} refused. Re-run with --write to apply.",
                    outcome.patches.len(),
                    outcome.refusals.len()
                );
                return Ok(());
            }
            if outcome.patches.is_empty() {
                println!(
                    "firstwatch: nothing to apply, {} refused.",
                    outcome.refusals.len()
                );
                return Ok(());
            }
            let mut verifiers = firstwatch_prove::detect_verifiers(&path);
            if with_tests {
                verifiers.extend(firstwatch_prove::detect_test_verifiers(&path));
            }
            if verifiers.is_empty() && replay.is_none() {
                let applied = firstwatch_mend::apply(&path, &outcome.patches)?;
                println!(
                    "firstwatch: applied {applied} patches UNPROVEN, no verifiers detected. Run your tests.",
                );
                return Ok(());
            }
            let mut verdict = firstwatch_prove::prove(&path, &outcome.patches, &verifiers)?;
            if let Some(cmd) = &replay {
                let check = firstwatch_prove::replay(&path, &outcome.patches, cmd)?;
                verdict.proven = verdict.proven && check.passed;
                verdict.checks.push(check);
                if verdict.checks.len() == 1 {
                    verdict.proven = verdict.checks[0].passed;
                }
            }
            for check in &verdict.checks {
                let status = if check.passed { "ok" } else { "FAILED" };
                println!("[prove] {} {status}", check.name);
                if !check.passed && !check.output_tail.is_empty() {
                    println!("{}", check.output_tail);
                }
            }
            if verdict.proven {
                let applied = firstwatch_mend::apply(&path, &outcome.patches)?;
                println!(
                    "firstwatch: applied {applied} patches, proven by {} check(s).",
                    verdict.checks.len()
                );
            } else {
                println!(
                    "firstwatch: refused to apply {} patches, proof failed. Your tree is untouched.",
                    outcome.patches.len()
                );
                std::process::exit(1);
            }
        }
        Command::Pr {
            path,
            events,
            recipes,
            with_tests,
            replay,
            branch,
            draft,
            allow_unproven,
        } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(events)?))?;
            let sites = firstwatch_scan::scan_dir(&path, &ProviderSet::default())?;
            let impacts = firstwatch_match::match_impacts(&events, &sites);
            let recipes = load_recipes(recipes)?;
            let outcome = firstwatch_mend::mend(&path, &impacts, &recipes)?;
            if outcome.patches.is_empty() {
                println!(
                    "firstwatch: nothing to patch, {} refused. No PR to open.",
                    outcome.refusals.len()
                );
                return Ok(());
            }
            let mut verifiers = firstwatch_prove::detect_verifiers(&path);
            if with_tests {
                verifiers.extend(firstwatch_prove::detect_test_verifiers(&path));
            }
            let verdict = if verifiers.is_empty() && replay.is_none() {
                None
            } else {
                let mut v = firstwatch_prove::prove(&path, &outcome.patches, &verifiers)?;
                if let Some(cmd) = &replay {
                    let check = firstwatch_prove::replay(&path, &outcome.patches, cmd)?;
                    if v.checks.is_empty() {
                        v.proven = check.passed;
                    } else {
                        v.proven = v.proven && check.passed;
                    }
                    v.checks.push(check);
                }
                for check in &v.checks {
                    let status = if check.passed { "ok" } else { "FAILED" };
                    println!("[prove] {} {status}", check.name);
                    if !check.passed && !check.output_tail.is_empty() {
                        println!("{}", check.output_tail);
                    }
                }
                if !v.proven && !allow_unproven {
                    println!("firstwatch: proof failed, no PR opened. Your tree is untouched.");
                    std::process::exit(1);
                }
                Some(v)
            };
            pr::open_pr(
                pr::PrArgs {
                    root: &path,
                    branch,
                    draft,
                    allow_unproven,
                },
                &outcome,
                verdict.as_ref(),
            )?;
        }
    }
    Ok(())
}

fn load_providers(path: Option<PathBuf>) -> Result<ProviderSet> {
    match path {
        Some(p) => Ok(ProviderSet::with_extra(read_ndjson(BufReader::new(
            File::open(p)?,
        ))?)),
        None => Ok(ProviderSet::default()),
    }
}

fn load_recipes(path: Option<PathBuf>) -> Result<Vec<firstwatch_schema::Recipe>> {
    match path {
        Some(p) => read_ndjson(BufReader::new(File::open(p)?)),
        None => Ok(Vec::new()),
    }
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
