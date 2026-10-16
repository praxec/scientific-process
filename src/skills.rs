//! `scientific-process skills install|uninstall|list`: copies the agent skills
//! under `skills/` (embedded at compile time) into the directories each agent
//! tool reads.
//!
//! Every skills root we write to gets a manifest, [`MANIFEST_FILE`], that
//! records the sha256 of each file as we wrote it and the targets that
//! installed into the root. A later run uses it to tell our files from the
//! user's:
//!
//! - a file that is not in the manifest is someone else's ("foreign, skipped");
//! - a file whose hash no longer matches the manifest was edited by the user
//!   ("modified, skipped");
//! - a file that still matches the manifest is ours to update or remove.
//!
//! `--force` overwrites foreign and modified files. Each run validates
//! everything before it writes, and writes atomically (a temp file in the same
//! directory, then a rename). The installer never writes MCP client
//! configuration; it prints nothing but the operations it performed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

include!(concat!(env!("OUT_DIR"), "/embedded_skills.rs"));

/// The manifest file written beside the installed skills in each skills root.
pub const MANIFEST_FILE: &str = ".scientific-process-skills.json";
/// The manifest schema version this binary writes and reads.
pub const MANIFEST_SCHEMA: u32 = 1;
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exit code for a successful run (skipped files included).
pub const EXIT_OK: u8 = 0;
/// Exit code for an IO error or a refused change.
pub const EXIT_IO: u8 = 1;
/// Exit code for a usage error.
pub const EXIT_USAGE: u8 = 2;

/// Usage text for the `skills` subcommand.
pub const USAGE: &str = "\
Usage:
  scientific-process skills install   --target <target> (--dir <path> | --user) [--dry-run] [--force]
  scientific-process skills uninstall --target <target> (--dir <path> | --user) [--dry-run]
  scientific-process skills list      [--dir <path> | --user]

Targets:
  claude     Claude Code      .claude/skills/   (user: ~/.claude/skills/)
  codex      OpenAI Codex     .agents/skills/   (user: ~/.agents/skills/)
  both       .claude/skills/ + .agents/skills/

A file you edited, or one scientific-process did not write, is kept and
reported as skipped; --force overwrites it. No MCP configuration is written.
";

/// Every embedded file under `skills/` as (path relative to `skills/`, contents).
pub fn embedded_skills() -> &'static [(&'static str, &'static str)] {
    EMBEDDED_SKILLS
}

/// The names of the embedded skills (directories holding a `SKILL.md`), sorted.
pub fn skill_names() -> Vec<&'static str> {
    SKILL_DESCRIPTIONS.iter().map(|(name, _)| *name).collect()
}

/// The front-matter `description` of an embedded skill (validated at build time).
pub fn skill_description(name: &str) -> Option<&'static str> {
    SKILL_DESCRIPTIONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| *d)
}

/// Lower-case hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The per-root record of what scientific-process wrote.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// [`MANIFEST_SCHEMA`] at write time.
    pub schema: u32,
    /// The scientific-process version that wrote the manifest.
    pub scientific_process_version: String,
    /// The targets (`claude`, `codex`, `both`) that installed into this root.
    #[serde(default)]
    pub targets: BTreeSet<String>,
    /// Path relative to the root (`/` separators) to the sha256 of the content
    /// scientific-process wrote there.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

// ---------------------------------------------------------------- targets

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Claude,
    Codex,
    Both,
}

impl Target {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Scope {
    Project(PathBuf),
    User(PathBuf),
}

impl Scope {
    fn is_user(&self) -> bool {
        matches!(self, Self::User(_))
    }

    fn base(&self) -> &Path {
        match self {
            Self::Project(dir) | Self::User(dir) => dir,
        }
    }
}

/// The skills root for one target under one scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Root {
    dir: PathBuf,
}

fn roots_for(target: Target, scope: &Scope) -> Vec<Root> {
    let mut dirs = Vec::new();
    if matches!(target, Target::Claude | Target::Both) {
        dirs.push(scope.base().join(".claude").join("skills"));
    }
    if matches!(target, Target::Codex | Target::Both) {
        dirs.push(scope.base().join(".agents").join("skills"));
    }
    dirs.into_iter().map(|dir| Root { dir }).collect()
}

// ---------------------------------------------------------------- errors

enum CliError {
    Usage(String),
    Io(String),
}

fn io_err(context: impl std::fmt::Display, err: io::Error) -> CliError {
    CliError::Io(format!("{context}: {err}"))
}

fn home_dir() -> Result<PathBuf, CliError> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| CliError::Io("cannot determine the home directory (set HOME)".into()))
}

// ---------------------------------------------------------------- file report

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Created,
    Updated,
    Unchanged,
    Modified,
    Foreign,
    Removed,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
            Self::Modified => "modified, skipped",
            Self::Foreign => "foreign, skipped",
            Self::Removed => "removed",
        }
    }
}

#[derive(Debug, Default)]
struct Report {
    lines: Vec<String>,
}

impl Report {
    fn record(&mut self, status: Status, path: &Path) {
        self.lines
            .push(format!("{}  {}", status.label(), path.display()));
    }
}

// ---------------------------------------------------------------- options

#[derive(Debug, Default)]
struct Options {
    target: Option<Target>,
    scope: Option<Scope>,
    dry_run: bool,
    force: bool,
}

fn set_scope(opts: &mut Options, scope: Scope) -> Result<(), CliError> {
    if let Some(existing) = &opts.scope {
        if existing.is_user() != scope.is_user() {
            return Err(CliError::Usage("give one of --user or --dir <path>".into()));
        }
    }
    opts.scope = Some(scope);
    Ok(())
}

fn parse_options(args: &[String], allow: &[&str]) -> Result<Options, CliError> {
    let mut opts = Options::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        if !allow.contains(&flag) {
            return Err(CliError::Usage(format!("unexpected argument `{arg}`")));
        }
        let mut value = |name: &str| -> Result<String, CliError> {
            match inline.clone().or_else(|| iter.next().cloned()) {
                Some(v) if !v.is_empty() => Ok(v),
                _ => Err(CliError::Usage(format!("{name} needs a value"))),
            }
        };
        if matches!(flag, "--user" | "--project" | "--dry-run" | "--force") && inline.is_some() {
            return Err(CliError::Usage(format!("{flag} takes no value")));
        }
        match flag {
            "--target" => {
                let raw = value("--target")?;
                opts.target = Some(Target::parse(&raw).ok_or_else(|| {
                    CliError::Usage(format!("unknown target `{raw}` (claude, codex, both)"))
                })?);
            }
            "--dir" => {
                let raw = value("--dir")?;
                let dir = std::path::absolute(&raw)
                    .map_err(|e| CliError::Usage(format!("--dir {raw}: {e}")))?;
                if !dir.is_dir() {
                    return Err(CliError::Usage(format!(
                        "--dir {raw}: not an existing directory"
                    )));
                }
                set_scope(&mut opts, Scope::Project(dir))?;
            }
            "--project" => {
                let dir = std::env::current_dir()
                    .map_err(|e| CliError::Io(format!("current directory: {e}")))?;
                set_scope(&mut opts, Scope::Project(dir))?;
            }
            "--user" => set_scope(&mut opts, Scope::User(home_dir()?))?,
            "--dry-run" => opts.dry_run = true,
            "--force" => opts.force = true,
            _ => unreachable!("flag is in the allow list"),
        }
    }
    Ok(opts)
}

fn require(opts: &Options) -> Result<(Target, Scope), CliError> {
    let target = opts
        .target
        .ok_or_else(|| CliError::Usage("--target <target> is required".into()))?;
    let scope = opts
        .scope
        .clone()
        .ok_or_else(|| CliError::Usage("give one of --dir <path> or --user".into()))?;
    Ok((target, scope))
}

/// Run `scientific-process skills <args>`; returns the process exit code.
pub fn run(args: &[String]) -> u8 {
    let mut stdout = io::stdout().lock();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("install") => parse_options(
            rest,
            &[
                "--target",
                "--dir",
                "--project",
                "--user",
                "--dry-run",
                "--force",
            ],
        )
        .and_then(|o| install(&o, &mut stdout)),
        Some("uninstall") => parse_options(
            rest,
            &["--target", "--dir", "--project", "--user", "--dry-run"],
        )
        .and_then(|o| uninstall(&o, &mut stdout)),
        Some("list") => parse_options(rest, &["--target", "--dir", "--project", "--user"])
            .and_then(|o| list(&o, &mut stdout)),
        Some("--help" | "-h" | "help") => {
            let _ = stdout.write_all(USAGE.as_bytes());
            Ok(())
        }
        Some(other) => Err(CliError::Usage(format!("unknown skills command `{other}`"))),
        None => Err(CliError::Usage("missing skills command".into())),
    };
    let _ = stdout.flush();
    match result {
        Ok(()) => EXIT_OK,
        Err(CliError::Usage(msg)) => {
            eprintln!("scientific-process skills: {msg}\n\n{USAGE}");
            EXIT_USAGE
        }
        Err(CliError::Io(msg)) => {
            eprintln!("scientific-process skills: error: {msg}");
            EXIT_IO
        }
    }
}

fn out_line(out: &mut dyn io::Write, line: &str) -> Result<(), CliError> {
    writeln!(out, "{line}").map_err(|e| io_err("stdout", e))
}

fn print_report(out: &mut dyn io::Write, report: &Report) -> Result<(), CliError> {
    for line in &report.lines {
        out_line(out, line)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- install

/// Decide what install does with one file.
fn decide(
    current: Option<&[u8]>,
    content: &[u8],
    recorded: Option<&String>,
    force: bool,
) -> (Status, bool, Option<String>) {
    let new_hash = sha256_hex(content);
    let Some(current) = current else {
        return (Status::Created, true, Some(new_hash));
    };
    if current == content {
        return (Status::Unchanged, false, Some(new_hash));
    }
    if force || recorded == Some(&sha256_hex(current)) {
        (Status::Updated, true, Some(new_hash))
    } else if recorded.is_some() {
        (Status::Modified, false, None)
    } else {
        (Status::Foreign, false, None)
    }
}

fn install(opts: &Options, out: &mut dyn io::Write) -> Result<(), CliError> {
    let (target, scope) = require(opts)?;
    let mut report = Report::default();
    for root in roots_for(target, &scope) {
        install_root(&root, target, opts.force, opts.dry_run, &mut report)?;
    }
    print_report(out, &report)
}

fn install_root(
    root: &Root,
    target: Target,
    force: bool,
    dry_run: bool,
    report: &mut Report,
) -> Result<(), CliError> {
    let old = load_manifest(&root.dir)?;
    let mut manifest = old.clone().unwrap_or_default();
    manifest.schema = MANIFEST_SCHEMA;
    manifest.scientific_process_version = VERSION.to_string();
    manifest.targets.insert(target.id().to_string());
    for (rel, content) in embedded_skills() {
        let path = join_rel(&root.dir, rel);
        check_inside(&root.dir, &path)?;
        let current = read_existing(&path)?;
        let recorded = old.as_ref().and_then(|m| m.files.get(*rel));
        let (status, write, hash) = decide(current.as_deref(), content.as_bytes(), recorded, force);
        if write && !dry_run {
            write_atomic(&path, content.as_bytes())?;
        }
        if let Some(hash) = hash {
            manifest.files.insert((*rel).to_string(), hash);
        }
        report.record(status, &path);
    }
    if !dry_run {
        save_manifest(&root.dir, &manifest)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- uninstall

fn uninstall(opts: &Options, out: &mut dyn io::Write) -> Result<(), CliError> {
    let (target, scope) = require(opts)?;
    let mut report = Report::default();
    for root in roots_for(target, &scope) {
        uninstall_root(&root, target, opts.dry_run, &mut report)?;
    }
    print_report(out, &report)
}

fn uninstall_root(
    root: &Root,
    target: Target,
    dry_run: bool,
    report: &mut Report,
) -> Result<(), CliError> {
    let Some(mut manifest) = load_manifest(&root.dir)? else {
        report
            .lines
            .push(format!("not installed  {}", root.dir.display()));
        return Ok(());
    };
    if !manifest.targets.remove(target.id()) {
        report
            .lines
            .push(format!("not installed  {}", root.dir.display()));
        return Ok(());
    }
    if !manifest.targets.is_empty() {
        let kept = manifest
            .targets
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if !dry_run {
            save_manifest(&root.dir, &manifest)?;
        }
        report.lines.push(format!(
            "kept (still used by {kept})  {}",
            root.dir.display()
        ));
        return Ok(());
    }
    for (rel, recorded) in &manifest.files {
        let path = join_rel(&root.dir, rel);
        check_inside(&root.dir, &path)?;
        match read_existing(&path)? {
            Some(current) if sha256_hex(&current) == *recorded => {
                if !dry_run {
                    remove_file(&path)?;
                }
                report.record(Status::Removed, &path);
            }
            Some(_) => report.record(Status::Modified, &path),
            None => {}
        }
    }
    if !dry_run {
        let empty = Manifest {
            schema: MANIFEST_SCHEMA,
            scientific_process_version: VERSION.to_string(),
            targets: BTreeSet::new(),
            files: BTreeMap::new(),
        };
        save_manifest(&root.dir, &empty)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- list

fn list(opts: &Options, out: &mut dyn io::Write) -> Result<(), CliError> {
    let scope = opts
        .scope
        .clone()
        .ok_or_else(|| CliError::Usage("give one of --dir <path> or --user".into()))?;
    let targets: Vec<Target> = match opts.target {
        Some(target) => vec![target],
        None => vec![Target::Claude, Target::Codex],
    };
    let mut seen = BTreeSet::new();
    for target in targets {
        for root in roots_for(target, &scope) {
            if !seen.insert(root.dir.clone()) {
                continue;
            }
            let manifest = load_manifest(&root.dir)?;
            out_line(out, &format!("{}:", root.dir.display()))?;
            for (rel, _) in embedded_skills() {
                if !rel.ends_with("/SKILL.md") {
                    continue;
                }
                let path = join_rel(&root.dir, rel);
                let current = read_existing(&path)?;
                let state = match (&manifest, &current) {
                    (Some(m), Some(cur)) if m.files.get(*rel) == Some(&sha256_hex(cur)) => {
                        "installed"
                    }
                    (Some(m), Some(_)) if m.files.contains_key(*rel) => "modified",
                    (Some(_), Some(_)) => "foreign",
                    (_, None) => "not installed",
                    (None, Some(_)) => "foreign",
                };
                out_line(out, &format!("  {state}  {}", path.display()))?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- manifest

fn load_manifest(root: &Path) -> Result<Option<Manifest>, CliError> {
    let path = root.join(MANIFEST_FILE);
    let Some(bytes) = read_existing(&path)? else {
        return Ok(None);
    };
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| CliError::Io(format!("parse {}: {e}", path.display())))?;
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(CliError::Io(format!(
            "{}: unsupported manifest schema {} (this binary writes {MANIFEST_SCHEMA})",
            path.display(),
            manifest.schema
        )));
    }
    Ok(Some(manifest))
}

fn save_manifest(root: &Path, manifest: &Manifest) -> Result<(), CliError> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| CliError::Io(format!("serialize manifest: {e}")))?;
    bytes.push(b'\n');
    write_atomic(&root.join(MANIFEST_FILE), &bytes)
}

// ---------------------------------------------------------------- filesystem

fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        path.push(part);
    }
    path
}

fn check_inside(root: &Path, path: &Path) -> Result<(), CliError> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    for comp in rel.components() {
        if matches!(
            comp,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(CliError::Io(format!(
                "refusing path outside {}",
                root.display()
            )));
        }
    }
    Ok(())
}

fn read_existing(path: &Path) -> Result<Option<Vec<u8>>, CliError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CliError::Io(format!("read {}: {e}", path.display()))),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let dir = path
        .parent()
        .ok_or_else(|| CliError::Io(format!("{}: no parent directory", path.display())))?;
    fs::create_dir_all(dir).map_err(|e| io_err(format!("create {}", dir.display()), e))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("manifest");
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| io_err(format!("write {}", tmp.display()), e))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(format!("rename {}", path.display()), e));
    }
    Ok(())
}

fn remove_file(path: &Path) -> Result<(), CliError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(format!("remove {}", path.display()), e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SKILLS: [&str; 3] = ["scientific-process", "sp-investigate", "sp-adjudicate"];

    fn call(args: &[&str]) -> u8 {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        run(&owned)
    }

    fn install(dir: &Path, extra: &[&str]) {
        let dir_text = dir.to_string_lossy().into_owned();
        let mut args = vec!["install", "--target", "claude"];
        args.extend_from_slice(extra);
        args.push("--dir");
        args.push(&dir_text);
        let _ = call(&args);
    }

    #[test]
    fn install_for_claude_writes_all_three_skills() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), &[]);
        let root = dir.path().join(".claude").join("skills");
        assert!(
            SKILLS
                .iter()
                .all(|name| root.join(name).join("SKILL.md").is_file())
        );
    }

    #[test]
    fn install_for_claude_writes_a_manifest_naming_each_skill() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), &[]);
        let manifest =
            std::fs::read_to_string(dir.path().join(".claude/skills").join(MANIFEST_FILE)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        let mut keys: Vec<String> = parsed["files"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "scientific-process/SKILL.md",
                "sp-adjudicate/SKILL.md",
                "sp-investigate/SKILL.md",
            ]
        );
    }

    #[test]
    fn install_for_claude_keeps_a_skill_file_the_user_modified() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), &[]);
        let skill = dir
            .path()
            .join(".claude/skills/scientific-process/SKILL.md");
        std::fs::write(&skill, "edited by the user").unwrap();
        install(dir.path(), &[]);
        assert_eq!(
            std::fs::read_to_string(&skill).unwrap(),
            "edited by the user"
        );
    }

    #[test]
    fn install_with_force_overwrites_a_skill_file_the_user_modified() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), &[]);
        let skill = dir
            .path()
            .join(".claude/skills/scientific-process/SKILL.md");
        std::fs::write(&skill, "edited by the user").unwrap();
        install(dir.path(), &["--force"]);
        assert_ne!(
            std::fs::read_to_string(&skill).unwrap(),
            "edited by the user"
        );
    }

    #[test]
    fn install_for_codex_writes_skills_under_the_agents_directory() {
        let dir = tempfile::tempdir().unwrap();
        let dir_text = dir.path().to_string_lossy().into_owned();
        let args = ["install", "--target", "codex", "--dir", &dir_text];
        let _ = call(&args);
        assert!(
            dir.path()
                .join(".agents/skills/scientific-process/SKILL.md")
                .is_file()
        );
    }

    #[test]
    fn install_for_both_writes_the_claude_and_codex_roots() {
        let dir = tempfile::tempdir().unwrap();
        let dir_text = dir.path().to_string_lossy().into_owned();
        let args = ["install", "--target", "both", "--dir", &dir_text];
        let _ = call(&args);
        assert!(
            dir.path()
                .join(".claude/skills/scientific-process/SKILL.md")
                .is_file()
                && dir
                    .path()
                    .join(".agents/skills/scientific-process/SKILL.md")
                    .is_file()
        );
    }

    #[test]
    fn uninstall_for_claude_removes_the_installed_skills() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), &[]);
        let dir_text = dir.path().to_string_lossy().into_owned();
        let args = ["uninstall", "--target", "claude", "--dir", &dir_text];
        let _ = call(&args);
        assert!(
            !dir.path()
                .join(".claude/skills/scientific-process/SKILL.md")
                .exists()
        );
    }
}
