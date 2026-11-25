//! Engine-neutral meeting job vocabulary. Discovery and retry scheduling belong to callers.
use serde::{Deserialize, Serialize};
use std::{fs::File, io, path::Path, sync::OnceLock};

pub mod checkpoint;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Pipeline,
    Discover,
    Prepare,
    Route,
    Transcribe,
    Translate,
    Notes,
    Render,
    Publish,
    Tasks,
    Compare,
    Title,
}
impl Stage {
    /// Legacy best-effort work; failure must not prevent required meeting outputs.
    pub fn is_optional(self) -> bool {
        matches!(self, Self::Tasks | Self::Compare | Self::Title)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cause {
    #[serde(rename = "HELPER_LIMIT")]
    Limit,
    #[serde(rename = "HELPER_AUTH")]
    Auth,
    #[serde(rename = "timeout")]
    TimedOut,
    #[serde(rename = "other")]
    Other,
}
impl Cause {
    pub fn label(self) -> &'static str {
        match self {
            Self::Limit => "Agent usage limit reached",
            Self::Auth => "Agent sign-in needed",
            Self::TimedOut => "Meeting processing took too long",
            Self::Other => "Processing failed. Open Sidenote.",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Limit => {
                "The summary agent has reached its usage limit. Your transcript is saved."
            }
            Self::Auth => "The summary agent needs you to sign in again. Your transcript is saved.",
            Self::TimedOut => {
                "Meeting processing took too long and was stopped. Your transcript is saved."
            }
            Self::Other => {
                "Meeting processing failed. Your transcript is saved. Check the notes setup before trying again."
            }
        }
    }
}

/// Accept only the existing content-free diagnostic protocol, never helper output.
fn diagnostic(line: &str) -> Option<(Stage, Cause)> {
    let line = line.strip_prefix("pipeline-failure ").or_else(|| {
        line.strip_prefix("try ")?
            .split_once(": pipeline-failure ")
            .map(|(_, tail)| tail)
    })?;
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() != 6 {
        return None;
    }
    let stage = serde_json::from_value(serde_json::Value::String(
        fields[0].strip_prefix("stage=")?.into(),
    ))
    .ok()?;
    let cause = serde_json::from_value(serde_json::Value::String(
        fields[1].strip_prefix("category=")?.into(),
    ))
    .ok()?;
    if !matches!(
        fields[2].strip_prefix("kind=")?,
        "git" | "claude" | "codex" | "python" | "opencode" | "other"
    ) {
        return None;
    }
    let exit = fields[3].strip_prefix("exit_code=")?;
    if exit != "none" {
        exit.parse::<i64>().ok()?;
        if exit.trim_start_matches('-').len() > 10 {
            return None;
        }
    }
    if fields[4]
        .strip_prefix("deadline_seconds=")?
        .parse::<u32>()
        .ok()?
        > 3600
    {
        return None;
    }
    let elapsed = fields[5]
        .strip_prefix("elapsed_seconds=")?
        .parse::<f64>()
        .ok()?;
    (elapsed.is_finite() && (0.0..=86400.0).contains(&elapsed)).then_some((stage, cause))
}

/// Both independently launched scripts and listener-launched scripts use these markers.
/// A completed summary wins over stale failure metadata after an interrupted cleanup.
pub fn script_failure(folder: &Path) -> Option<Cause> {
    if folder.join(&contract().summary_file).is_file() {
        return None;
    }
    match super::meeting_router::inputs::Snapshot::read_file(
        &folder.join("pipeline-failed.txt"),
        8192,
    ) {
        Ok(bytes) => Some(
            std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                .ok()
                .and_then(|text| text.lines().rev().find(|line| !line.trim().is_empty()))
                .and_then(|line| diagnostic(line).map(|(_, cause)| cause))
                .unwrap_or(Cause::Other),
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => folder
            .join("notes-failed")
            .is_file()
            .then_some(Cause::Other),
        Err(_) => Some(Cause::Other),
    }
}

// This is the existing notes helper failure type, shared rather than translated into a rival type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Busy,
    LinuxUnavailable,
    Consent,
    Missing,
    TimedOut,
    Backend { exit: Option<i32> },
    BadOutput,
    Limit { exit: Option<i32> },
    Auth { exit: Option<i32> },
}

#[derive(Deserialize)]
pub struct Contract {
    pub version: u32,
    pub stages: Vec<Stage>,
    pub categories: Vec<Cause>,
    pub lock_file: String,
    pub summary_file: String,
    pub preserve_completed: bool,
}
pub fn contract() -> &'static Contract {
    static CONTRACT: OnceLock<Contract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../scripts/meeting-pipeline/job-contract.json"
        ))
        .expect("compiled meeting job contract")
    })
}

/// Hold through final publication. Never unlink the lock: the OS releases it on crash.
/// A busy claim is deferral, without consuming an attempt or replacing durable job state.
pub fn claim(folder: &Path) -> io::Result<File> {
    super::store_io::lock(&folder.join(&contract().lock_file))
}

/// Recheck under the claim after restart. Running is reclaimable, not proof of ownership.
/// Existing output completes automatic work; explicit replay alone may replace it.
pub fn preserve_summary(folder: &Path, explicit_replay: bool) -> bool {
    contract().preserve_completed
        && !explicit_replay
        && folder.join(&contract().summary_file).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn legacy_adapter_contends_with_the_core_claim_and_reclaims_after_release() {
        use crate::platform::CommandExt;
        let temp = tempfile::tempdir().unwrap();
        let first = claim(temp.path()).unwrap();
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command",
            ". $env:JOB_CONTRACT_CLI; $claim=Enter-NotesEngine $env:JOB_CONTRACT_FOLDER; if ($null -eq $claim) { exit 10 }; $claim.Dispose(); exit 0"])
            .env("JOB_CONTRACT_CLI", Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/meeting-pipeline/hex-pipeline-cli.ps1"))
            .env("JOB_CONTRACT_FOLDER", temp.path())
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        assert_eq!(command.status().unwrap().code(), Some(10));
        drop(first);
        assert_eq!(command.status().unwrap().code(), Some(0));
    }

    #[test]
    fn diagnostics_are_allowlisted_and_the_latest_attempt_wins() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path();
        let path = folder.join("pipeline-failed.txt");
        assert_eq!(
            diagnostic(
                "pipeline-failure stage=notes category=HELPER_AUTH kind=claude exit_code=4294967295 deadline_seconds=600 elapsed_seconds=1.000"
            ),
            Some((Stage::Notes, Cause::Auth))
        );
        for (category, cause) in [
            ("HELPER_LIMIT", Cause::Limit),
            ("HELPER_AUTH", Cause::Auth),
            ("timeout", Cause::TimedOut),
            ("other", Cause::Other),
        ] {
            let safe = format!(
                "pipeline-failure stage=notes category={category} kind=claude exit_code=1 deadline_seconds=600 elapsed_seconds=1.000"
            );
            assert_eq!(diagnostic(&safe), Some((Stage::Notes, cause)));
            std::fs::write(&path, format!("\u{feff}try 2026-11-14T12:00:00: {safe}\n")).unwrap();
            assert_eq!(script_failure(folder), Some(cause));
        }
        for bad in [
            "stage=private",
            "category=private",
            "kind=private",
            "exit_code=private",
            "deadline_seconds=3601",
            "elapsed_seconds=NaN",
        ] {
            let valid = "stage=notes category=HELPER_AUTH kind=claude exit_code=1 deadline_seconds=600 elapsed_seconds=1.000";
            let prefix = bad.split_once('=').unwrap().0;
            let altered = valid
                .split_whitespace()
                .map(|field| {
                    if field.starts_with(prefix) {
                        bad
                    } else {
                        field
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            assert!(diagnostic(&format!("pipeline-failure {altered}")).is_none());
        }
        std::fs::write(&path, b"pipeline-failure stage=notes category=HELPER_AUTH kind=claude exit_code=1 deadline_seconds=600 elapsed_seconds=1.000\nsynthetic private text\n").unwrap();
        assert_eq!(script_failure(folder), Some(Cause::Other));
        std::fs::write(&path, vec![b'x'; 8193]).unwrap();
        assert_eq!(script_failure(folder), Some(Cause::Other));
        std::fs::write(folder.join(&contract().summary_file), b"{}").unwrap();
        assert_eq!(script_failure(folder), None);
    }

    #[test]
    fn wire_contract_keeps_existing_vocabulary() {
        let c = contract();
        assert_eq!(c.version, 1);
        assert_eq!(c.stages.len(), 10);
        assert_eq!(
            c.categories,
            [Cause::Limit, Cause::Auth, Cause::TimedOut, Cause::Other]
        );
        assert_eq!(c.lock_file, ".notes-engine.lock");
        assert_eq!(c.summary_file, "english-summary.json");
        assert!(c.preserve_completed);
    }

    #[test]
    fn claim_serializes_engines_and_restart_preserves_completed_output() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path();
        let first = claim(folder).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                tx.send(claim(folder).is_err()).unwrap();
            });
        });
        assert!(rx.recv().unwrap());
        drop(first);
        let _restarted = claim(folder).unwrap();
        assert!(!preserve_summary(folder, false));
        std::fs::write(folder.join(&contract().summary_file), b"synthetic summary").unwrap();
        assert!(preserve_summary(folder, false));
        assert!(!preserve_summary(folder, true));
    }
}
