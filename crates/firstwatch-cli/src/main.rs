mod fleet;
mod pr;
mod store;

use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

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
    /// Parse prose release notes into ChangeEvents via the Claude API.
    /// Uses ANTHROPIC_API_KEY; events are flagged low confidence.
    Changelog {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        provider: String,
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
        /// Synthesize fixes for refused impacts via the Claude API.
        /// Uses ANTHROPIC_API_KEY; synthesized patches still face the
        /// prove gate like any other.
        #[arg(long)]
        llm: bool,
        #[arg(long)]
        write: bool,
    },
    /// Emit shareable graph data: anonymized usage edges from a scan, or
    /// your locally proven fixes with --proven. Contribute either to the
    /// registry as a pull request; nothing is ever uploaded automatically.
    Share {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Emit the local proven-fix store instead of usage edges.
        #[arg(long)]
        proven: bool,
        /// ProviderSpec ndjson extending the built-in provider table.
        #[arg(long)]
        providers: Option<PathBuf>,
    },
    /// Blast radius: which repos does a changefeed break, before anything
    /// ships. Scan and match only; nothing is modified.
    Blast {
        /// File with one repo path or git URL per line. Omit with --usage
        /// to run in ecosystem mode over shared usage edges.
        #[arg(long)]
        repos: Option<PathBuf>,
        #[arg(long)]
        events: PathBuf,
        /// UsageEdge ndjson of shared usage data for ecosystem mode.
        #[arg(long)]
        usage: Option<PathBuf>,
        /// ProviderSpec ndjson extending the built-in provider table.
        #[arg(long)]
        providers: Option<PathBuf>,
        /// Working directory for clones.
        #[arg(long, default_value = ".firstwatch-fleet")]
        work_dir: PathBuf,
    },
    /// Run pr across a fleet of repos from a list of paths or git URLs.
    Fleet {
        /// File with one repo path or git URL per line.
        #[arg(long)]
        repos: PathBuf,
        #[arg(long)]
        events: PathBuf,
        #[arg(long)]
        recipes: Option<PathBuf>,
        /// Working directory for clones.
        #[arg(long, default_value = ".firstwatch-fleet")]
        work_dir: PathBuf,
        /// Synthesize fixes for refused impacts via the configured model.
        /// Proven fixes propagate to later repos, so each unique change
        /// is synthesized at most once per fleet run.
        #[arg(long)]
        llm: bool,
        /// Report what each repo needs without branching or opening PRs.
        #[arg(long)]
        dry_run: bool,
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
        Command::Changelog { file, provider } => {
            let llm = firstwatch_llm::Llm::from_env()
                .ok_or_else(|| anyhow::anyhow!("ANTHROPIC_API_KEY is not set"))?;
            let text = std::fs::read_to_string(&file)?;
            let events = llm.parse_changelog(&text, &provider)?;
            write_ndjson(io::stdout().lock(), &events)?;
        }
        Command::Fix {
            path,
            events,
            recipes,
            with_tests,
            replay,
            llm,
            write,
        } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(events)?))?;
            let sites = firstwatch_scan::scan_dir(&path, &ProviderSet::default())?;
            let impacts = firstwatch_match::match_impacts(&events, &sites);
            let recipes = load_recipes(recipes)?;
            let mut outcome = firstwatch_mend::mend(&path, &impacts, &recipes)?;
            let known = store::load_templates();
            let reused = propagate(&path, &known, &mut outcome);
            if reused > 0 {
                println!("firstwatch: reused {reused} previously proven fix(es).");
            }
            if llm {
                synthesize_refused(&path, &mut outcome)?;
            }
            for patch in &outcome.patches {
                let tag = if patch.origin == "llm" { " (llm)" } else { "" };
                println!("{}:{}{tag}", patch.file, patch.line);
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
                let checks: Vec<String> = verdict.checks.iter().map(|c| c.name.clone()).collect();
                let _ = store::record_proven(&outcome.patches, &checks);
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
        Command::Share {
            path,
            proven,
            providers,
        } => {
            if proven {
                let store = store::store_path()
                    .ok_or_else(|| anyhow::anyhow!("cannot resolve the store path"))?;
                match std::fs::read_to_string(&store) {
                    Ok(content) => print!("{content}"),
                    Err(_) => eprintln!("firstwatch: no proven fixes recorded yet."),
                }
                return Ok(());
            }
            let sites = firstwatch_scan::scan_dir(&path, &load_providers(providers)?)?;
            let mut counts: std::collections::BTreeMap<(String, String, String), u64> =
                std::collections::BTreeMap::new();
            for site in &sites {
                *counts
                    .entry((
                        site.provider.clone(),
                        site.surface.clone(),
                        site.language.clone(),
                    ))
                    .or_default() += 1;
            }
            let edges: Vec<firstwatch_schema::UsageEdge> = counts
                .into_iter()
                .map(
                    |((provider, surface, language), count)| firstwatch_schema::UsageEdge {
                        schema_version: firstwatch_schema::SCHEMA_VERSION.into(),
                        provider,
                        surface,
                        language,
                        count,
                    },
                )
                .collect();
            write_ndjson(io::stdout().lock(), &edges)?;
            eprintln!(
                "firstwatch: {} anonymized edge(s): provider, surface, language, count. \
                 No file paths, no repo names. Contribute by PR to the registry.",
                edges.len()
            );
        }
        Command::Blast {
            repos,
            events,
            usage,
            providers,
            work_dir,
        } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(&events)?))?;
            if let Some(usage) = usage {
                let edges: Vec<firstwatch_schema::UsageEdge> =
                    read_ndjson(BufReader::new(File::open(&usage)?))?;
                let mut total: u64 = 0;
                let mut lines = Vec::new();
                for event in &events {
                    let breaking = matches!(
                        event.kind,
                        firstwatch_schema::ChangeKind::Removed
                            | firstwatch_schema::ChangeKind::Renamed
                            | firstwatch_schema::ChangeKind::TypeChanged
                            | firstwatch_schema::ChangeKind::RequiredAdded
                    );
                    if !breaking {
                        continue;
                    }
                    let hits: Vec<&firstwatch_schema::UsageEdge> = edges
                        .iter()
                        .filter(|e| e.provider == event.provider && e.surface == event.surface)
                        .collect();
                    if hits.is_empty() {
                        continue;
                    }
                    let count: u64 = hits.iter().map(|e| e.count).sum();
                    let mut langs: Vec<&str> = hits.iter().map(|e| e.language.as_str()).collect();
                    langs.sort_unstable();
                    langs.dedup();
                    total += count;
                    lines.push(format!(
                        "  {} {}{} ({:?}) -> {count} known call site(s) across {}",
                        event.provider,
                        event.surface,
                        event
                            .field
                            .as_deref()
                            .map(|f| format!(" ({f})"))
                            .unwrap_or_default(),
                        event.kind,
                        langs.join(", ")
                    ));
                }
                if lines.is_empty() {
                    println!("firstwatch: no known consumer breaks.");
                } else {
                    println!("ecosystem blast radius:");
                    for line in lines {
                        println!("{line}");
                    }
                    println!("\nfirstwatch: {total} known call site(s) break.");
                    std::process::exit(1);
                }
                return Ok(());
            }
            let repos = repos
                .ok_or_else(|| anyhow::anyhow!("pass --repos, or --usage for ecosystem mode"))?;
            let provider_set = load_providers(providers)?;
            let targets = fleet::resolve(&repos, &work_dir)?;
            let mut total_breaking = 0;
            let mut by_change: std::collections::BTreeMap<String, Vec<String>> =
                std::collections::BTreeMap::new();
            println!("repo  breaking  deprecated  other");
            for target in &targets {
                let sites = firstwatch_scan::scan_dir(&target.path, &provider_set)?;
                let impacts = firstwatch_match::match_impacts(&events, &sites);
                let breaking = impacts
                    .iter()
                    .filter(|i| matches!(i.severity, Severity::BreaksNow | Severity::BreaksLater))
                    .count();
                let deprecated = impacts
                    .iter()
                    .filter(|i| i.severity == Severity::Deprecated)
                    .count();
                let other = impacts.len() - breaking - deprecated;
                println!("{}  {breaking}  {deprecated}  {other}", target.name);
                total_breaking += breaking;
                for impact in &impacts {
                    if matches!(impact.severity, Severity::BreaksNow | Severity::BreaksLater) {
                        by_change
                            .entry(format!(
                                "{} {}{}",
                                impact.change.provider,
                                impact.change.surface,
                                impact
                                    .change
                                    .field
                                    .as_deref()
                                    .map(|f| format!(" ({f})"))
                                    .unwrap_or_default()
                            ))
                            .or_default()
                            .push(format!("{}:{}", target.name, impact.site.line));
                    }
                }
            }
            if by_change.is_empty() {
                println!("firstwatch: no repo in the fleet breaks.");
            } else {
                println!("\nblast radius by change:");
                for (change, hits) in &by_change {
                    println!("  {change} -> {} site(s): {}", hits.len(), hits.join(", "));
                }
                println!(
                    "\nfirstwatch: {total_breaking} breaking site(s) across {} repo(s).",
                    targets.len()
                );
                std::process::exit(1);
            }
        }
        Command::Fleet {
            repos,
            events,
            recipes,
            work_dir,
            llm,
            dry_run,
        } => {
            let events: Vec<ChangeEvent> = read_ndjson(BufReader::new(File::open(&events)?))?;
            let recipes = load_recipes(recipes)?;
            let targets = fleet::resolve(&repos, &work_dir)?;
            let mut summary = Vec::new();
            let mut templates: std::collections::HashMap<String, (String, String)> =
                store::load_templates();
            for target in &targets {
                let sites = firstwatch_scan::scan_dir(&target.path, &ProviderSet::default())?;
                let impacts = firstwatch_match::match_impacts(&events, &sites);
                if impacts.is_empty() {
                    summary.push((target.name.clone(), 0, 0, "clean".to_string()));
                    continue;
                }
                let mut outcome = firstwatch_mend::mend(&target.path, &impacts, &recipes)?;
                let propagated = propagate(&target.path, &templates, &mut outcome);
                if propagated > 0 {
                    println!(
                        "firstwatch: {}: reused {propagated} proven fix(es) from earlier repos",
                        target.name
                    );
                }
                if llm && !dry_run {
                    synthesize_refused(&target.path, &mut outcome)?;
                }
                if dry_run || outcome.patches.is_empty() {
                    summary.push((
                        target.name.clone(),
                        outcome.patches.len(),
                        outcome.refusals.len(),
                        if dry_run { "dry run" } else { "needs human" }.to_string(),
                    ));
                    continue;
                }
                let verifiers = firstwatch_prove::detect_verifiers(&target.path);
                let verdict = if verifiers.is_empty() {
                    None
                } else {
                    Some(firstwatch_prove::prove(
                        &target.path,
                        &outcome.patches,
                        &verifiers,
                    )?)
                };
                let proven = verdict.as_ref().map(|v| v.proven).unwrap_or(false);
                if proven {
                    for patch in &outcome.patches {
                        templates.insert(
                            store::key_of_impact(&patch.impact),
                            (
                                patch.old_line.trim().to_string(),
                                patch.new_line.trim().to_string(),
                            ),
                        );
                    }
                    let checks: Vec<String> = verdict
                        .as_ref()
                        .map(|v| v.checks.iter().map(|c| c.name.clone()).collect())
                        .unwrap_or_default();
                    let _ = store::record_proven(&outcome.patches, &checks);
                }
                let status = if proven {
                    match pr::open_pr(
                        pr::PrArgs {
                            root: &target.path,
                            branch: None,
                            draft: false,
                            allow_unproven: false,
                        },
                        &outcome,
                        verdict.as_ref(),
                    ) {
                        Ok(()) => "pr opened",
                        Err(_) => "pr failed",
                    }
                } else {
                    "proof failed"
                };
                summary.push((
                    target.name.clone(),
                    outcome.patches.len(),
                    outcome.refusals.len(),
                    status.to_string(),
                ));
            }
            println!("repo  patches  refused  status");
            for (name, patches, refused, status) in &summary {
                println!("{name}  {patches}  {refused}  {status}");
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
                if v.proven {
                    let checks: Vec<String> = v.checks.iter().map(|c| c.name.clone()).collect();
                    let _ = store::record_proven(&outcome.patches, &checks);
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

fn synthesize_refused(root: &Path, outcome: &mut firstwatch_mend::MendOutcome) -> Result<()> {
    let Some(llm) = firstwatch_llm::Llm::from_env() else {
        println!("firstwatch: --llm set but ANTHROPIC_API_KEY is not, skipping synthesis.");
        return Ok(());
    };
    let mut remaining = Vec::new();
    for refusal in outcome.refusals.drain(..) {
        let site = &refusal.impact.site;
        let Ok(content) = std::fs::read_to_string(root.join(&site.file)) else {
            remaining.push(refusal);
            continue;
        };
        let lines: Vec<&str> = content.lines().collect();
        let Some(old_line) = lines.get(site.line - 1).copied() else {
            remaining.push(refusal);
            continue;
        };
        let start = site.line.saturating_sub(6);
        let end = (site.line + 5).min(lines.len());
        let context = lines[start..end].join(
            "
",
        );
        match llm.fix_line(&refusal.impact, old_line, &context) {
            Ok(new_line) => outcome.patches.push(firstwatch_schema::PatchCandidate {
                schema_version: firstwatch_schema::SCHEMA_VERSION.into(),
                file: site.file.clone(),
                line: site.line,
                old_line: old_line.to_string(),
                new_line,
                origin: "llm".into(),
                impact: refusal.impact.clone(),
            }),
            Err(reason) => {
                let mut refusal = refusal;
                refusal.reason = format!("{} (llm: {reason})", refusal.reason);
                remaining.push(refusal);
            }
        }
    }
    outcome.refusals = remaining;
    Ok(())
}

/// Cross-repo fix propagation: a patch proven in one repo becomes the
/// template for the same change elsewhere in the fleet. A refusal whose
/// site line matches a template's old line (modulo indentation) is
/// upgraded to a patch carrying the proven rewrite; every repo's prove
/// gate still runs on the result.
fn propagate(
    root: &Path,
    templates: &std::collections::HashMap<String, (String, String)>,
    outcome: &mut firstwatch_mend::MendOutcome,
) -> usize {
    if templates.is_empty() {
        return 0;
    }
    let mut remaining = Vec::new();
    let mut count = 0;
    for refusal in outcome.refusals.drain(..) {
        let Some((old_line, new_line)) = templates.get(&store::key_of_impact(&refusal.impact))
        else {
            remaining.push(refusal);
            continue;
        };
        let site = &refusal.impact.site;
        let Ok(content) = std::fs::read_to_string(root.join(&site.file)) else {
            remaining.push(refusal);
            continue;
        };
        let Some(site_line) = content.lines().nth(site.line - 1) else {
            remaining.push(refusal);
            continue;
        };
        if site_line.trim() != old_line.as_str() {
            remaining.push(refusal);
            continue;
        }
        let indent: String = site_line
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        outcome.patches.push(firstwatch_schema::PatchCandidate {
            schema_version: firstwatch_schema::SCHEMA_VERSION.into(),
            file: site.file.clone(),
            line: site.line,
            old_line: site_line.to_string(),
            new_line: format!("{indent}{new_line}"),
            origin: "propagated".into(),
            impact: refusal.impact.clone(),
        });
        count += 1;
    }
    outcome.refusals = remaining;
    count
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
