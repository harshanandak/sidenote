//! English-only previews never enter final-summary publication or its downstream stores.
use super::{
    Engine, Helper, NotesSettings,
    helper::{self, Failure, Run},
    prompt, schema, transcript,
};
use crate::core::{self, agent_instructions::Agent, file_reads, store_io};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn path(root: &Path, id: &str) -> PathBuf {
    root.join("local/draft-summaries")
        .join(format!("{id}.json"))
}

pub fn discard_final(root: &Path, folder: &Path, id: &str) {
    if super::queue::valid_id(id)
        && !core::paths::read_only()
        && let Ok(Some(_claim)) = core::meeting_delete::claim_write(root, id)
        && folder.join("english-summary.json").is_file()
    {
        let _ = fs::remove_file(path(root, id));
    }
}

pub(crate) fn read(root: &Path, folder: &Path, id: &str) -> io::Result<String> {
    if !super::queue::valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    let path = path(root, id);
    if folder.join("english-summary.json").is_file() {
        // A locked preview must not hide the final; the next scan retries cleanup.
        discard_final(root, folder, id);
        return Ok(String::new());
    }
    match file_reads::text(&path, file_reads::SUMMARY_MAX) {
        Ok(raw) => Ok(raw),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

pub fn start(root: PathBuf, meetings: PathBuf, id: String, settings: NotesSettings) {
    if !super::queue::valid_id(&id) {
        return;
    }
    std::thread::spawn(move || run_claimed(root, meetings, id, settings));
}

fn run_claimed(root: PathBuf, meetings: PathBuf, id: String, settings: NotesSettings) {
    let Ok(Some(_claim)) = core::meeting_delete::claim_write(&root, &id) else {
        return;
    };
    let started = Instant::now();
    let mut run = DraftCli { root: root.clone() };
    let result = generate(&root, &meetings, &id, &settings, &mut run);
    let outcome = match result {
        Ok(Outcome::Skipped) => "skipped",
        Ok(Outcome::Written) => "written",
        Ok(Outcome::NoHelper) => "skipped helper unavailable",
        Err(_) => "skipped helper or local data unavailable",
    };
    let _ = store_io::write(
        &root.join("local/draft-outcomes").join(format!("{id}.json")),
        &if outcome == "written" {
            "written"
        } else {
            "deferred"
        },
    );
    crate::listener::log(&format!(
        "draft {id} duration={:.1}s {outcome}",
        started.elapsed().as_secs_f64()
    ));
}

struct DraftCli {
    root: PathBuf,
}
impl Run for DraftCli {
    fn installed(&self, helper: Helper) -> bool {
        helper::Cli {
            root: Some(self.root.clone()),
        }
        .installed(helper)
    }
    fn glossary(&self) -> Vec<String> {
        helper::Cli {
            root: Some(self.root.clone()),
        }
        .glossary()
    }
    fn run(
        &mut self,
        helper: Helper,
        model: &str,
        prompt: String,
        timeout: Duration,
    ) -> Result<String, Failure> {
        let settings = core::Config::load_in(&self.root)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    Failure::Busy
                } else {
                    Failure::Consent
                }
            })?
            .notes;
        if !settings.is_some_and(|s| {
            matches!(s.engine(), Engine::Scripts(_)) && s.effective_helper() == Some(helper)
        }) {
            return Err(Failure::Consent);
        }
        helper::Cli { root: None }.run(helper, model, prompt, timeout)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Skipped,
    Written,
    NoHelper,
}

pub(super) fn generate(
    root: &Path,
    meetings: &Path,
    id: &str,
    settings: &NotesSettings,
    run: &mut impl Run,
) -> io::Result<Outcome> {
    if !super::queue::valid_id(id) {
        return Err(store_io::invalid("Invalid meeting id"));
    }
    if !matches!(settings.engine(), Engine::Scripts(_))
        || core::recording_guard::eligibility(root, meetings, id)?
            != core::recording_guard::Eligibility::Eligible
    {
        return Ok(Outcome::Skipped);
    }
    let folder = meetings.join(id);
    store_io::reject_reparse(&folder)?;
    if folder.join("english-summary.json").is_file() || path(root, id).is_file() {
        return Ok(Outcome::Skipped);
    }
    let _lock = match store_io::lock(&path(root, id).with_extension("lock")) {
        Ok(lock) => lock,
        Err(e)
            if (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)))
                || (cfg!(unix) && e.kind() == io::ErrorKind::WouldBlock) =>
        {
            return Ok(Outcome::Skipped);
        }
        Err(e) => return Err(e),
    };
    if folder.join("english-summary.json").is_file() || path(root, id).is_file() {
        return Ok(Outcome::Skipped);
    }
    let manifest = super::queue::manifest(&folder)?;
    if manifest["status"] != "complete" {
        return Ok(Outcome::Skipped);
    }
    let Some(helper) = settings.effective_helper().filter(|h| run.installed(*h)) else {
        return Ok(Outcome::NoHelper);
    };
    let transcript = transcript::read(&folder)?;
    if transcript.is_empty() || transcript.chars().count() > prompt::TRANSCRIPT_CAP {
        return Ok(Outcome::Skipped);
    }
    let typed = match file_reads::text(&folder.join("notes.md"), file_reads::JOURNAL_MAX) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let glossary = run.glossary();
    if !prompt::bounded_inputs(&typed, &glossary) {
        return Ok(Outcome::Skipped);
    }
    let started = manifest["created_at_ms"]
        .as_i64()
        .or_else(|| id.split('-').next()?.parse().ok())
        .ok_or_else(|| store_io::invalid("Missing meeting date"))?;
    let date = chrono::DateTime::from_timestamp_millis(started)
        .ok_or_else(|| store_io::invalid("Invalid meeting date"))?
        .with_timezone(&chrono::Local)
        .date_naive();
    let teams = super::queue::teams(root)?;
    let material = prompt::material(
        prompt::Details {
            title: manifest["title"].as_str().unwrap_or("not provided"),
            started_ms: started,
            minutes: manifest["duration_ms"]
                .as_f64()
                .map_or(0, |d| (d / 60_000.0).ceil() as u64),
            your_name: &settings.your_name,
        },
        &typed,
        &transcript,
        &teams,
        &glossary,
    );
    let model = if settings.model.trim().is_empty() {
        helper.default_model()
    } else {
        &settings.model
    };
    let call = prompt::Call {
        source: None,
        root,
        helper,
        model,
        timeout: helper::NOTES_TIMEOUT,
        base_override: None,
    };
    let answer = call
        .ladder(run, Agent::Notes, &material, |raw| {
            schema::parse(raw, id, date, &transcript, &teams)
        })
        .map_err(|_| store_io::invalid("Draft helper failed"))?;
    let mut summary = answer.value;
    summary.todo.clear();
    summary.made_by = Some(prompt::provenance(
        helper,
        model,
        answer.custom,
        answer.fallback,
    ));
    // The final script can finish during the helper call; never resurrect its draft.
    if folder.join("english-summary.json").is_file() {
        return Ok(Outcome::Skipped);
    }
    store_io::write(&path(root, id), &summary)?;
    let _ = read(root, &folder, id)?;
    Ok(Outcome::Written)
}

#[cfg(test)]
mod recycle_tests {
    use super::*;
    use crate::core::meeting_delete::{Eligibility, Storage};

    #[test]
    fn recycled_draft_does_not_publish_an_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("meetings");
        let root = temp.path().join("state");
        let meeting = source.join("100-1");
        fs::create_dir_all(&meeting).unwrap();
        fs::create_dir_all(root.join("local/draft-outcomes")).unwrap();
        fs::write(meeting.join("manifest.json"), b"{}").unwrap();
        let outcome = root.join("local/draft-outcomes/100-1.json");
        fs::write(&outcome, b"original").unwrap();
        let storage = Storage::new(&source, &root).unwrap();
        let id = storage
            .delete(
                "100-1",
                "Synthetic".into(),
                1,
                |_| Ok(Eligibility::Ready),
                |_| Ok(()),
            )
            .unwrap();
        run_claimed(
            root.clone(),
            source,
            "100-1".into(),
            NotesSettings::default(),
        );
        assert!(!outcome.exists());
        storage.restore(&id).unwrap();
        assert_eq!(fs::read(outcome).unwrap(), b"original");
    }
}
