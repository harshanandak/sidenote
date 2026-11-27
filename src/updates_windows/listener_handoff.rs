use super::*;

const SCRIPT: &str = include_str!("listener_handoff.ps1");

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Snapshot {
    exe: PathBuf,
    args: String,
    working_dir: String,
    xml: String,
    security: String,
    enabled: bool,
    running: bool,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TaskPlan {
    prior: Snapshot,
    disabled: String,
    switched: String,
    enabled: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum Definition {
    Prior,
    Disabled,
    Switched,
    Enabled,
}

impl TaskPlan {
    fn xml(&self, definition: Definition) -> &str {
        match definition {
            Definition::Prior => &self.prior.xml,
            Definition::Disabled => &self.disabled,
            Definition::Switched => &self.switched,
            Definition::Enabled => &self.enabled,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum Phase {
    Disable,
    Switch,
    Enable,
    Ready,
    Restore(Definition),
    Restored,
    Complete,
}

#[derive(Debug, Deserialize, Serialize)]
struct Journal {
    schema: u8,
    task: String,
    target: PathBuf,
    version: String,
    plan: TaskPlan,
    phase: Phase,
}

impl Journal {
    fn publish(&self, directory: &Path) -> io::Result<PathBuf> {
        let file = self.write_durable(directory)?;
        let path = file.path().with_extension("json");
        crate::platform::publish_file(file.path(), &path)?;
        Ok(path)
    }

    fn write_durable(&self, directory: &Path) -> io::Result<tempfile::NamedTempFile> {
        let mut file = tempfile::Builder::new()
            .prefix("listener-handoff-")
            .suffix(".tmp")
            .rand_bytes(16)
            .tempfile_in(directory)?;
        serde_json::to_writer(&mut file, self)?;
        file.as_file().sync_all()?;
        Ok(file)
    }

    fn persist_phase(&mut self, path: &Path, phase: Phase) -> io::Result<()> {
        let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let previous = std::mem::replace(&mut self.phase, phase);
        let result = self
            .write_durable(parent)
            .and_then(|file| crate::platform::replace_file(file.path(), path));
        if result.is_err() {
            self.phase = previous;
        }
        result
    }

    fn belongs_to(&self, task: &str, target: &Path) -> bool {
        self.schema == 1 && self.task == task && self.target == target && self.version == VERSION
    }

    fn restore_from(&self, current: &Snapshot) -> Result<Definition, ListenerTaskError> {
        use Definition::{Disabled, Enabled, Prior, Switched};
        let allowed: &[Definition] = match self.phase {
            Phase::Disable => &[Prior, Disabled],
            Phase::Switch => &[Disabled, Switched],
            Phase::Enable => &[Switched, Enabled],
            Phase::Ready => &[Enabled],
            Phase::Restore(ref previous) => std::slice::from_ref(previous),
            Phase::Restored | Phase::Complete => &[],
        };
        allowed
            .iter()
            .copied()
            .chain(matches!(self.phase, Phase::Restore(_)).then_some(Prior))
            .find(|&state| {
                self.plan.prior.enabled
                    && current.security == self.plan.prior.security
                    && current.xml == self.plan.xml(state)
            })
            .ok_or(ListenerTaskError::TaskChanged)
    }
}

#[path = "listener_handoff_tests.rs"]
mod tests;
