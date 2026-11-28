use super::VERSION;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{IgnoredAny, IntoDeserializer},
};
use std::{collections::BTreeMap, fs, io, path::Path};

pub(super) const FILE: &str = "install-state.json";
const STORAGE: &str = "Could not read install-state.json. Check the update directory permissions or restore a valid record before retrying.";
const NEWER: &str =
    "The install state belongs to a newer Sidenote version. Open the newer installation.";
const REQUIRED: [&str; 4] = [
    "schema_version",
    "app_version",
    "listener_work",
    "pipeline_work",
];

#[derive(PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum ListenerWork {
    Pending,
    Complete,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum PipelineWork {
    Pending,
    Complete,
    NotConfigured,
}

#[derive(Serialize, Deserialize)]
pub(super) enum Diagnostic {
    ListenerRecovery,
    PipelineInstall,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Record {
    pub(super) schema_version: u32,
    pub(super) app_version: String,
    #[serde(deserialize_with = "deserialize_work")]
    pub(super) listener_work: ListenerWork,
    #[serde(deserialize_with = "deserialize_work")]
    pub(super) pipeline_work: PipelineWork,
    pub(super) listener_diagnostic: Option<Diagnostic>,
    pub(super) pipeline_diagnostic: Option<Diagnostic>,
}

// Decode the wire value as a string before letting the enum resolve its variant.
fn deserialize_work<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(String::deserialize(deserializer)?.into_deserializer())
}

#[derive(Deserialize)]
struct DecodedRecord {
    #[serde(flatten)]
    record: Record,
    #[serde(flatten)]
    extra: BTreeMap<String, IgnoredAny>,
}

pub(super) fn read(directory: &Path) -> Result<Option<Record>, String> {
    let path = directory.join(FILE);
    if path
        .symlink_metadata()
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
    {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|_| STORAGE)?;
    let DecodedRecord { record, extra } = serde_json::from_slice(&bytes).map_err(|_| STORAGE)?;
    // Serde rejects exact duplicates. Also reject case variants of required fields.
    if record.schema_version != 1
        || extra
            .keys()
            .any(|key| REQUIRED.iter().any(|field| key.eq_ignore_ascii_case(field)))
    {
        return Err(STORAGE.into());
    }
    let version = semver::Version::parse(&record.app_version).map_err(|_| STORAGE)?;
    if version
        .cmp_precedence(&semver::Version::parse(VERSION).expect("valid app version"))
        .is_gt()
    {
        return Err(NEWER.into());
    }
    Ok(Some(record))
}
