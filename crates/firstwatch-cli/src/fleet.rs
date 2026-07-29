use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

pub struct FleetTarget {
    pub name: String,
    pub path: PathBuf,
}

/// Resolves a repo list into local working copies. Lines are either
/// local paths or git URLs; URLs are cloned into the work dir, or
/// pulled when already present.
pub fn resolve(list: &Path, work_dir: &Path) -> Result<Vec<FleetTarget>> {
    let content = std::fs::read_to_string(list)
        .with_context(|| format!("reading repo list {}", list.display()))?;
    std::fs::create_dir_all(work_dir)?;
    let mut targets = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.contains("://") || line.starts_with("git@") {
            let name = line
                .rsplit('/')
                .next()
                .unwrap_or(line)
                .trim_end_matches(".git")
                .to_string();
            let dest = work_dir.join(&name);
            if dest.join(".git").exists() {
                git(&dest, &["pull", "--ff-only"])?;
            } else {
                let out = Command::new("git")
                    .args(["clone", "--depth", "1", line])
                    .arg(&dest)
                    .output()
                    .context("running git clone")?;
                if !out.status.success() {
                    bail!(
                        "cloning {line}: {}",
                        String::from_utf8_lossy(&out.stderr).trim()
                    );
                }
            }
            targets.push(FleetTarget { name, path: dest });
        } else {
            let path = PathBuf::from(line);
            if !path.is_dir() {
                bail!("repo path does not exist: {line}");
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| line.to_string());
            targets.push(FleetTarget { name, path });
        }
    }
    Ok(targets)
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
