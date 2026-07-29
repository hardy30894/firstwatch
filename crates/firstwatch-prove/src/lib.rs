use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use firstwatch_schema::PatchCandidate;

pub struct Verifier {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

pub struct Check {
    pub name: String,
    pub passed: bool,
    pub output_tail: String,
}

pub struct Verdict {
    pub proven: bool,
    pub checks: Vec<Check>,
}

impl Verdict {
    fn no_verifiers() -> Self {
        Verdict {
            proven: false,
            checks: Vec::new(),
        }
    }
}

/// Detects verifiers the project already has. Typecheck only by default;
/// test suites are opt-in at the CLI because they can be arbitrarily slow.
pub fn detect_verifiers(root: &Path) -> Vec<Verifier> {
    let mut verifiers = Vec::new();
    if root.join("tsconfig.json").exists() {
        let local_tsc = root.join("node_modules/.bin/tsc");
        let program = if local_tsc.exists() {
            local_tsc.to_string_lossy().into_owned()
        } else {
            "tsc".into()
        };
        verifiers.push(Verifier {
            name: "typecheck (tsc --noEmit)".into(),
            program,
            args: vec!["--noEmit".into()],
        });
    }
    verifiers
}

pub fn detect_test_verifiers(root: &Path) -> Vec<Verifier> {
    let mut verifiers = Vec::new();
    if let Ok(pkg) = std::fs::read_to_string(root.join("package.json")) {
        let has_real_test_script = serde_json::from_str::<serde_json::Value>(&pkg)
            .ok()
            .and_then(|v| {
                v.pointer("/scripts/test")
                    .and_then(|s| s.as_str().map(String::from))
            })
            .is_some_and(|s| !s.contains("no test specified"));
        if has_real_test_script {
            verifiers.push(Verifier {
                name: "tests (npm test)".into(),
                program: "npm".into(),
                args: vec!["test".into(), "--silent".into()],
            });
        }
    }
    if root.join("pytest.ini").exists()
        || root.join("conftest.py").exists()
        || std::fs::read_to_string(root.join("pyproject.toml"))
            .is_ok_and(|s| s.contains("[tool.pytest"))
    {
        verifiers.push(Verifier {
            name: "tests (pytest)".into(),
            program: "pytest".into(),
            args: vec!["-q".into()],
        });
    }
    verifiers
}

/// Differential replay: runs the same command in an unpatched staging and
/// a patched staging, then requires identical exit status and stdout.
/// The command should exercise the impacted code deterministically,
/// against fixtures or a sandbox.
pub fn replay(root: &Path, patches: &[PatchCandidate], cmd: &str) -> Result<Check> {
    let baseline_dir = tempfile::tempdir().context("creating baseline staging")?;
    stage_repo(root, baseline_dir.path())?;
    let patched_dir = tempfile::tempdir().context("creating patched staging")?;
    stage_repo(root, patched_dir.path())?;
    firstwatch_mend::apply(patched_dir.path(), patches)?;

    let baseline = run_shell(cmd, baseline_dir.path())?;
    let patched = run_shell(cmd, patched_dir.path())?;

    let passed = baseline.0 && patched.0 && baseline.1 == patched.1;
    let output_tail = if passed {
        String::new()
    } else if !baseline.0 {
        format!("baseline run failed:\n{}", tail(&baseline.1))
    } else if !patched.0 {
        format!("patched run failed:\n{}", tail(&patched.1))
    } else {
        format!(
            "output diverged\nbaseline:\n{}\npatched:\n{}",
            tail(&baseline.1),
            tail(&patched.1)
        )
    };
    Ok(Check {
        name: format!("replay ({cmd})"),
        passed,
        output_tail,
    })
}

fn run_shell(cmd: &str, dir: &Path) -> Result<(bool, String)> {
    let output = Command::new("sh")
        .args(["-c", cmd])
        .current_dir(dir)
        .output()
        .with_context(|| format!("running replay command: {cmd}"))?;
    let combined = [output.stdout, output.stderr].concat();
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&combined).into_owned(),
    ))
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().rev().take(10).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// Stages the repo in a temp dir, applies the patches there, runs every
/// verifier in the staged copy. The real tree is never touched.
pub fn prove(root: &Path, patches: &[PatchCandidate], verifiers: &[Verifier]) -> Result<Verdict> {
    if verifiers.is_empty() {
        return Ok(Verdict::no_verifiers());
    }
    let staging = tempfile::tempdir().context("creating staging dir")?;
    stage_repo(root, staging.path())?;
    firstwatch_mend::apply(staging.path(), patches)?;

    let mut checks = Vec::new();
    let mut proven = true;
    for verifier in verifiers {
        let output = Command::new(&verifier.program)
            .args(&verifier.args)
            .current_dir(staging.path())
            .output()
            .with_context(|| format!("running {}", verifier.name))?;
        let passed = output.status.success();
        proven &= passed;
        let combined = [output.stdout, output.stderr].concat();
        let text = String::from_utf8_lossy(&combined);
        let tail: Vec<&str> = text.lines().rev().take(10).collect();
        checks.push(Check {
            name: verifier.name.clone(),
            passed,
            output_tail: tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
        });
    }
    Ok(Verdict { proven, checks })
}

fn stage_repo(root: &Path, staging: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git" && e.file_name() != "node_modules")
    {
        let entry = entry?;
        let rel = entry.path().strip_prefix(root)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dest = staging.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    let node_modules = root.join("node_modules");
    if node_modules.exists() {
        let absolute = std::fs::canonicalize(&node_modules)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&absolute, staging.join("node_modules"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use firstwatch_schema::{
        ChangeEvent, ChangeKind, ChangeSource, Confidence, Impact, Severity, UsageSite,
        SCHEMA_VERSION,
    };

    fn patch(file: &str, old_line: &str, new_line: &str) -> PatchCandidate {
        PatchCandidate {
            schema_version: SCHEMA_VERSION.into(),
            file: file.into(),
            line: 1,
            old_line: old_line.into(),
            new_line: new_line.into(),
            origin: "tier1".into(),
            impact: Impact {
                schema_version: SCHEMA_VERSION.into(),
                severity: Severity::Deprecated,
                change: ChangeEvent {
                    schema_version: SCHEMA_VERSION.into(),
                    provider: "stripe".into(),
                    surface: "charges.create".into(),
                    field: None,
                    kind: ChangeKind::Deprecated,
                    from: None,
                    to: Some("payment_intents.create".into()),
                    effective_version: None,
                    effective_date: None,
                    confidence: Confidence::High,
                    source: ChangeSource::Changelog,
                    evidence_url: None,
                },
                site: UsageSite {
                    schema_version: SCHEMA_VERSION.into(),
                    provider: "stripe".into(),
                    surface: "charges.create".into(),
                    file: file.into(),
                    line: 1,
                    column: 1,
                    language: "typescript".into(),
                    snippet: None,
                },
            },
        }
    }

    fn shell(name: &str, script: &str) -> Verifier {
        Verifier {
            name: name.into(),
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
        }
    }

    #[test]
    fn passing_verifier_proves_and_real_tree_untouched() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "old line\n").unwrap();
        let verdict = prove(
            dir.path(),
            &[patch("a.ts", "old line", "new line")],
            &[shell("grep patched", "grep -q 'new line' a.ts")],
        )
        .unwrap();
        assert!(verdict.proven);
        let original = std::fs::read_to_string(dir.path().join("a.ts")).unwrap();
        assert_eq!(original, "old line\n");
    }

    #[test]
    fn failing_verifier_fails_the_verdict_with_output() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "old line\n").unwrap();
        let verdict = prove(
            dir.path(),
            &[patch("a.ts", "old line", "new line")],
            &[shell("always fails", "echo type error in a.ts; exit 1")],
        )
        .unwrap();
        assert!(!verdict.proven);
        assert!(verdict.checks[0].output_tail.contains("type error"));
    }

    #[test]
    fn replay_passes_when_output_is_identical() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "unrelated line\n").unwrap();
        std::fs::write(dir.path().join("out.txt"), "same\n").unwrap();
        let check = replay(
            dir.path(),
            &[patch("a.ts", "unrelated line", "still unrelated")],
            "cat out.txt",
        )
        .unwrap();
        assert!(check.passed);
    }

    #[test]
    fn replay_fails_when_output_diverges() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "old line\n").unwrap();
        let check = replay(
            dir.path(),
            &[patch("a.ts", "old line", "new line")],
            "cat a.ts",
        )
        .unwrap();
        assert!(!check.passed);
        assert!(check.output_tail.contains("output diverged"));
    }

    #[test]
    fn no_verifiers_means_unproven() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.ts"), "old line\n").unwrap();
        let verdict = prove(dir.path(), &[patch("a.ts", "old line", "new line")], &[]).unwrap();
        assert!(!verdict.proven);
        assert!(verdict.checks.is_empty());
    }
}
