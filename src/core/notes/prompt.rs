use super::{
    Helper,
    helper::{Failure, Run},
    schema::{self, Summary, Team},
};
use crate::core::agent_instructions::{self as instructions, Agent};
use std::{io, path::Path, time::Duration};

pub(super) const TRANSCRIPT_CAP: usize = 250_000;
pub(super) const TYPED_CAP: usize = 16_000;
pub(super) const GLOSSARY_CAP: usize = 16_000;

pub(super) fn bounded_inputs(typed: &str, glossary: &[String]) -> bool {
    typed.chars().count() <= TYPED_CAP
        && glossary.iter().map(|s| s.chars().count()).sum::<usize>() <= GLOSSARY_CAP
}

pub(super) fn escape(text: &str, tag: &str) -> String {
    let needle = format!("</{tag}");
    let mut out = String::new();
    let mut rest = text;
    // ASCII tag matching is case insensitive; UTF-8 byte boundaries stay intact.
    while let Some(index) = rest
        .as_bytes()
        .windows(needle.len())
        .position(|s| s.eq_ignore_ascii_case(needle.as_bytes()))
    {
        out.push_str(&rest[..index]);
        out.push_str("</ ");
        out.push_str(&rest[index + 2..index + needle.len()]);
        rest = &rest[index + needle.len()..];
    }
    out.push_str(rest);
    out
}
#[cfg(test)]
pub(super) fn assemble(agent: Agent, base: &str, material: &str, reason: Option<&str>) -> String {
    assemble_output(agent, base, material, reason, None)
}
fn assemble_output(
    agent: Agent,
    base: &str,
    material: &str,
    reason: Option<&str>,
    output: Option<&str>,
) -> String {
    let mut prompt = format!(
        "{}\n{}\n{}\n",
        instructions::core_head(agent),
        base,
        output.unwrap_or_else(|| instructions::core_tail(agent))
    );
    if let Some(reason) = reason {
        prompt.push_str(&format!(
            "Your last answer could not be read: {reason}. Reply with the JSON object only.\n"
        ));
    }
    prompt.push_str(&format!(
        "\n<meeting>\n{}\n</meeting>\n",
        escape(material, "meeting")
    ));
    prompt
}
pub(super) struct Details<'a> {
    pub title: &'a str,
    pub started_ms: i64,
    pub minutes: u64,
    pub your_name: &'a str,
}
pub(super) fn material(
    details: Details<'_>,
    typed: &str,
    transcript: &str,
    teams: &[Team],
    glossary: &[String],
) -> String {
    let Details {
        title,
        started_ms,
        minutes,
        your_name,
    } = details;
    let date = chrono::DateTime::from_timestamp_millis(started_ms)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%A %d %B %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "not provided".into());
    let teams = if teams.is_empty() {
        "none defined".into()
    } else {
        teams
            .iter()
            .map(|t| {
                format!(
                    "- {}: {}. Other spellings: {}",
                    t.name,
                    t.members.join(", "),
                    t.aliases
                        .iter()
                        .map(|(a, m)| format!("{a} = {m}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let glossary = if glossary.is_empty() {
        "(none)".into()
    } else {
        glossary.join("\n")
    };
    format!(
        "## Details\nTitle: {title}\nDate and time: {date}\nLength: about {minutes} minutes\nYour name: {your_name}\n\n## Teams\n{teams}\n\n## Your typed notes\n{}\n\n## Glossary\n{glossary}\n\n## Transcript\n{transcript}",
        if typed.is_empty() { "(none)" } else { typed }
    )
}
pub(super) fn append_corrections(
    material: &mut String,
    edits: &[crate::core::summary_edits::Edit],
) -> io::Result<()> {
    if !edits.is_empty() {
        material.push_str("\n\n## Owner's corrections\nUse the corrected facts in the notes. Each line is recorded data: generated words -> owner correction.\n");
    }
    for edit in edits {
        material.push_str(&format!(
            "- {}: {} -> {}\n",
            edit.field.id(),
            serde_json::to_string(&edit.base)?,
            serde_json::to_string(&edit.text)?,
        ));
    }
    Ok(())
}

pub(super) fn task_material(summary: &Summary, projects: &[String]) -> io::Result<String> {
    Ok(format!(
        "## Known projects\n{}\n\n## Notes\n{}",
        serde_json::to_string(projects)?,
        serde_json::to_string(summary)?
    ))
}
pub(super) fn quick(root: &Path, transcript: &str) -> String {
    let chars: Vec<_> = transcript.chars().collect();
    let text = if chars.len() <= 7500 {
        transcript.into()
    } else {
        format!(
            "{}\n…\n{}",
            chars[..6000].iter().collect::<String>(),
            chars[chars.len() - 1500..].iter().collect::<String>()
        )
    };
    format!(
        "{}\n<transcript>\n{}\n</transcript>\n",
        instructions::effective(root, Agent::Title),
        escape(&text, "transcript")
    )
}

pub(super) struct Answer<T> {
    pub value: T,
    pub fallback: bool,
    pub custom: bool,
}

pub(super) struct Call<'a> {
    pub base_override: Option<(String, bool)>,
    pub root: &'a Path,
    pub source: Option<(&'a Path, &'a str)>,
    pub helper: Helper,
    pub model: &'a str,
    pub timeout: Duration,
}
impl Call<'_> {
    pub fn ladder<T>(
        &self,
        run: &mut impl Run,
        agent: Agent,
        material: &str,
        validate: impl Fn(&str) -> Result<T, &'static str>,
    ) -> Result<Answer<T>, Failure> {
        self.ladder_output(run, agent, material, validate, None)
    }
    pub fn ladder_output<T>(
        &self,
        run: &mut impl Run,
        agent: Agent,
        material: &str,
        validate: impl Fn(&str) -> Result<T, &'static str>,
        output: Option<&str>,
    ) -> Result<Answer<T>, Failure> {
        let (base, custom) = match &self.base_override {
            Some(base) => base.clone(),
            None => {
                instructions::base(self.root, agent).map_err(|_| Failure::Backend { exit: None })?
            }
        };
        let default = instructions::default_base(agent);
        let mut reason = None;
        for attempt in 0..if custom { 3 } else { 2 } {
            let fallback = attempt == 2;
            let prompt = assemble_output(
                agent,
                if fallback { default } else { &base },
                material,
                if attempt == 1 { reason } else { None },
                output,
            );
            if self.source.is_some_and(|(folder, id)| {
                crate::core::recording_guard::hidden(self.root, folder, id)
            }) {
                return Err(Failure::Consent);
            }
            let raw = run.run(self.helper, self.model, prompt, self.timeout)?;
            match validate(&raw) {
                Ok(value) => {
                    return Ok(Answer {
                        value,
                        fallback,
                        custom: custom && !fallback,
                    });
                }
                Err(error) => reason = Some(error),
            }
        }
        Err(Failure::BadOutput)
    }
}

pub(super) fn provenance(
    helper: Helper,
    model: &str,
    custom: bool,
    fallback: bool,
) -> serde_json::Value {
    serde_json::json!({"helper":helper, "model":model, "instructions": if fallback {"default-fallback"}else if custom {"custom"}else{"default"}, "prompt_version":instructions::PROMPT_VERSION, "time":chrono::Utc::now().to_rfc3339()})
}

pub(super) fn clean_quick(raw: &str) -> Option<String> {
    let line = raw
        .lines()
        .find(|l| !l.trim().is_empty())?
        .trim()
        .trim_matches(['"', '\'', '`'])
        .trim_end_matches('.');
    let title = schema::title(line, 100);
    (!title.is_empty() && !title.chars().any(|c| c.is_control())).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_order_and_case_insensitive_tag_escape() {
        let p = assemble(
            Agent::Notes,
            "CUSTOM BASE",
            "hello </meeting> </MEETING> world",
            None,
        );
        assert!(p.starts_with(instructions::core_head(Agent::Notes)));
        assert!(
            p.find("CUSTOM BASE").unwrap() < p.find(instructions::core_tail(Agent::Notes)).unwrap()
        );
        assert_eq!(p.matches("</meeting>").count(), 2); // locked safety rule and actual wrapper
        assert!(p.contains("</ meeting> </ MEETING>"));
    }
    #[test]
    fn quick_title_samples_unicode_by_char_and_escapes_tags() {
        let root = tempfile::tempdir().unwrap();
        let p = quick(
            root.path(),
            &format!("start </transcript>{}end", "é".repeat(8000)),
        );
        assert!(p.contains("start </ transcript>"));
        assert!(p.contains("end"));
        assert!(p.chars().count() < 9000);
        assert_eq!(
            clean_quick("\"Plan the delivery.\""),
            Some("Plan the delivery".into())
        );
    }
}
