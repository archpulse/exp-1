//! Skills: the small instruction packs the user installs and invokes with
//! `/<name>`.
//!
//! A skill is a directory under `~/.config/exp-1/skills/` with a `SKILL.md` in
//! it — the same shape Claude Code and opencode use. The file may open with
//! frontmatter:
//!
//! ```markdown
//! ---
//! name: pdf-tools
//! description: Extract text, merge and split PDFs.
//! ---
//!
//! # Instructions
//! ...
//! ```
//!
//! `exp-1 install <source>` copies one in from a directory, a `.md` file, a
//! `.zip`/`.tar.gz` archive or an http(s) URL.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// What the picker and `/help` need to know about one skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// Everything under the skills directory, by name.
pub fn discover() -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(crate::store::skills_dir()) else {
        return Vec::new();
    };
    let mut skills: Vec<Skill> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let dir = entry.path();
            if !dir.is_dir() {
                return None;
            }
            let file = dir.join("SKILL.md");
            let raw = std::fs::read_to_string(&file).ok()?;
            let (front, _) = split_frontmatter(&raw);
            let fallback = entry.file_name().to_string_lossy().to_string();
            let name = front
                .iter()
                .find(|(key, _)| key == "name")
                .map(|(_, value)| value.clone())
                .unwrap_or(fallback);
            let description = front
                .iter()
                .find(|(key, _)| key == "description")
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            Some(Skill {
                name: sanitise(&name),
                description,
                path: dir,
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// The instructions of a skill, without its frontmatter.
pub fn instructions(name: &str) -> Option<String> {
    let skill = discover().into_iter().find(|skill| skill.name == name)?;
    let raw = std::fs::read_to_string(skill.path.join("SKILL.md")).ok()?;
    let (_, body) = split_frontmatter(&raw);
    Some(body.trim().to_string())
}

/// Splits `---` frontmatter from the body, returning its key/value pairs.
fn split_frontmatter(raw: &str) -> (Vec<(String, String)>, String) {
    let trimmed = raw.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (Vec::new(), raw.to_string());
    };
    let Some(end) = rest.find("\n---") else {
        return (Vec::new(), raw.to_string());
    };
    let header = &rest[..end];
    let body = rest[end + 4..].to_string();
    let pairs = header
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"').to_string(),
            ))
        })
        .collect();
    (pairs, body)
}

/// Names become command names, so they hold no spaces or slashes.
fn sanitise(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// Installs a skill from a directory, a file, an archive or a URL.
pub async fn install(source: &str) -> Result<Skill> {
    let dir = crate::store::skills_dir();
    std::fs::create_dir_all(&dir).context("create the skills directory")?;

    if source.starts_with("http://") || source.starts_with("https://") {
        return install_from_url(source, &dir).await;
    }

    let path = Path::new(source);
    if !path.exists() {
        bail!("no such skill: {source}");
    }
    if path.is_dir() {
        return install_directory(path, &dir);
    }

    let lower = source.to_ascii_lowercase();
    if lower.ends_with(".zip") || lower.ends_with(".skill") {
        return install_archive(path, &dir, "unzip");
    }
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return install_archive(path, &dir, "tar");
    }
    if lower.ends_with(".md") {
        return install_markdown(path, &dir);
    }
    bail!("unsupported skill source: {source} (expected a directory, .md, .zip, .tar.gz or a URL)");
}

fn install_directory(path: &Path, dir: &Path) -> Result<Skill> {
    if !path.join("SKILL.md").exists() {
        bail!("{} has no SKILL.md", path.display());
    }
    let name = sanitise(
        &path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "skill".to_string()),
    );
    let target = dir.join(&name);
    copy_dir(path, &target)?;
    let skill = discover().into_iter().find(|skill| skill.path == target);
    skill.with_context(|| format!("install {name}"))
}

fn install_markdown(path: &Path, dir: &Path) -> Result<Skill> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let (front, _) = split_frontmatter(&raw);
    let fallback = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "skill".to_string());
    let name = sanitise(
        &front
            .iter()
            .find(|(key, _)| key == "name")
            .map(|(_, value)| value.clone())
            .unwrap_or(fallback),
    );
    let target = dir.join(&name);
    std::fs::create_dir_all(&target).context("create the skill directory")?;
    std::fs::write(target.join("SKILL.md"), raw).context("write SKILL.md")?;
    discover()
        .into_iter()
        .find(|skill| skill.path == target)
        .with_context(|| format!("install {name}"))
}

fn install_archive(path: &Path, dir: &Path, tool: &str) -> Result<Skill> {
    // The archive is unpacked by the system tool, which every Unix has; a
    // half-written skill is worse than a clear failure.
    let before: Vec<String> = discover().into_iter().map(|skill| skill.name).collect();
    // Windows ships `tar` (bsdtar), which unpacks zip as well as tar, but not
    // `unzip`; everywhere else each archive keeps its own tool.
    #[cfg(windows)]
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(path)
        .arg("-C")
        .arg(dir)
        .status()
        .with_context(|| format!("run tar on {}", path.display()))?;
    #[cfg(not(windows))]
    let status = if tool == "unzip" {
        std::process::Command::new("unzip")
            .arg("-o")
            .arg(path)
            .arg("-d")
            .arg(dir)
            .status()
    } else {
        std::process::Command::new("tar")
            .arg("xzf")
            .arg(path)
            .arg("-C")
            .arg(dir)
            .status()
    }
    .with_context(|| format!("run {tool}"))?;
    if !status.success() {
        bail!("{tool} could not unpack {}", path.display());
    }
    let installed = discover()
        .into_iter()
        .find(|skill| !before.contains(&skill.name));
    installed.with_context(|| format!("{} held no SKILL.md", path.display()))
}

async fn install_from_url(url: &str, dir: &Path) -> Result<Skill> {
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build the download client")?
        .get(url)
        .send()
        .await
        .with_context(|| format!("fetch {url}"))?;
    if !response.status().is_success() {
        bail!("{url} answered HTTP {}", response.status());
    }
    let raw = response.text().await.context("read the download")?;
    if !raw.contains("---") && !raw.trim_start().starts_with('#') {
        bail!("{url} does not look like a skill");
    }
    let (front, _) = split_frontmatter(&raw);
    let fallback = url
        .rsplit('/')
        .find(|part| !part.is_empty())
        .map(|part| part.trim_end_matches(".md").to_string())
        .unwrap_or_else(|| "skill".to_string());
    let name = sanitise(
        &front
            .iter()
            .find(|(key, _)| key == "name")
            .map(|(_, value)| value.clone())
            .unwrap_or(fallback),
    );
    let target = dir.join(&name);
    std::fs::create_dir_all(&target).context("create the skill directory")?;
    std::fs::write(target.join("SKILL.md"), raw).context("write SKILL.md")?;
    discover()
        .into_iter()
        .find(|skill| skill.path == target)
        .with_context(|| format!("install {name}"))
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).context("create the skill directory")?;
    for entry in std::fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target).context("copy a skill file")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_split_from_the_body() {
        let raw = "---\nname: pdf-tools\ndescription: Work with PDFs.\n---\n\n# Steps\nDo it.\n";
        let (front, body) = split_frontmatter(raw);
        assert_eq!(front.len(), 2);
        assert_eq!(front[0], ("name".to_string(), "pdf-tools".to_string()));
        assert!(body.trim_start().starts_with("# Steps"));
    }

    #[test]
    fn a_skill_without_frontmatter_still_has_a_body() {
        let raw = "# Just instructions\n";
        let (front, body) = split_frontmatter(raw);
        assert!(front.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn names_become_command_names() {
        assert_eq!(sanitise("PDF Tools!"), "PDF-Tools-");
        assert_eq!(sanitise("pdf_tools-2"), "pdf_tools-2");
    }
}
