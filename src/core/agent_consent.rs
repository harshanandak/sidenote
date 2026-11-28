//! Device-local permission for OpenCode Ask recipients.
use serde::{Deserialize, Serialize};
use std::{fs::File, path::Path};

const AGENT: &str = "opencode";
const PROVIDER: &str = "opencode";
const PURPOSE: &str = "ask";
const SCHEMA: u8 = 1;
pub const TERMS_SOURCE: &str = "https://opencode.ai/docs/console/models/";
const TERMS_REVISION: &str = "2026-11-28";
const STORE_ERROR: &str = "Could not update OpenCode Ask permission.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms {
    pub model: &'static str,
    pub data: &'static str,
    pub restricted: bool,
}

impl Terms {
    const fn new(model: &'static str, data: &'static str, restricted: bool) -> Self {
        Self {
            model,
            data,
            restricted,
        }
    }
}
const NVIDIA_TRIAL: &str = "The NVIDIA trial terms prohibit personal or confidential content.";
const FREE_CLOUD_TERMS: &[Terms] = &[
    Terms::new(
        "opencode/big-pickle",
        "Big Pickle is a temporary free offer.",
        false,
    ),
    Terms::new(
        "opencode/beam",
        "Beam does not train on your data and retains it for 30 days for safety.",
        false,
    ),
    Terms::new("opencode/nemotron-3.5-lightning", NVIDIA_TRIAL, true),
    Terms::new("opencode/nemotron-3.5-super-vl", NVIDIA_TRIAL, true),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Permission(&'static Terms),
    Restricted(&'static Terms),
    UnreviewedFree,
    Unresolved,
}
impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::Permission(_) => {
                "This free cloud model needs Ask permission on this device. Choose another model."
            }
            Self::Restricted(_) => {
                "This model's terms do not allow sensitive meeting content. Choose another model."
            }
            Self::UnreviewedFree => {
                "This free OpenCode model has no reviewed data terms. Choose another model."
            }
            Self::Unresolved => {
                "Choose an explicit OpenCode model with reviewed data terms before sending meeting text."
            }
        }
    }
}

pub fn terms(model: Option<&str>) -> Result<&'static Terms, Refusal> {
    let model = model.ok_or(Refusal::Unresolved)?;
    match FREE_CLOUD_TERMS.iter().find(|terms| terms.model == model) {
        Some(terms) if terms.restricted => Err(Refusal::Restricted(terms)),
        Some(terms) => Ok(terms),
        None if model.contains('/') && model.ends_with("-free") => Err(Refusal::UnreviewedFree),
        None => Err(Refusal::Unresolved),
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    schema: u8,
    generation: u64,
    grants: Vec<Grant>,
}
#[derive(Serialize, Deserialize)]
struct Grant {
    purpose: String,
    agent: String,
    provider: String,
    model: String,
    terms_revision: String,
    accepted_at_ms: i64,
    generation: u64,
}
impl Grant {
    fn matches(&self, terms: &Terms) -> bool {
        self.purpose == PURPOSE
            && self.agent == AGENT
            && self.provider == PROVIDER
            && self.model == terms.model
    }
}

fn path(root: &Path) -> std::path::PathBuf {
    root.join("local/agent-consent.json")
}
fn load(root: &Path) -> Store {
    super::store_io::read(&path(root))
        .ok()
        .filter(|store: &Store| store.schema == SCHEMA)
        .unwrap_or_default()
}
pub fn allowed(root: &Path, terms: &Terms) -> bool {
    let store = load(root);
    store.grants.iter().any(|grant| {
        grant.matches(terms)
            && grant.terms_revision == TERMS_REVISION
            && grant.accepted_at_ms > 0
            && (1..=store.generation).contains(&grant.generation)
    })
}

pub fn set_permission(root: &Path, terms: &Terms, allow: bool) -> Result<(), &'static str> {
    let terms = self::terms(Some(terms.model)).map_err(Refusal::message)?;
    let _guard = super::config::lock(root).map_err(|_| STORE_ERROR)?;
    let mut store = load(root);
    store.schema = SCHEMA;
    store.generation = store.generation.checked_add(1).ok_or(STORE_ERROR)?;
    store.grants.retain(|grant| !grant.matches(terms));
    if allow {
        store.grants.push(Grant {
            purpose: PURPOSE.into(),
            agent: AGENT.into(),
            provider: PROVIDER.into(),
            model: terms.model.into(),
            terms_revision: TERMS_REVISION.into(),
            accepted_at_ms: chrono::Utc::now().timestamp_millis(),
            generation: store.generation,
        });
    }
    super::store_io::write(&path(root), &store).map_err(|_| STORE_ERROR)
}

pub(super) fn authorize(root: &Path, model: Option<&str>) -> Result<File, Refusal> {
    let terms = terms(model)?;
    let guard = super::config::lock(root).map_err(|_| Refusal::Permission(terms))?;
    if !allowed(root, terms) {
        return Err(Refusal::Permission(terms));
    }
    Ok(guard)
}
