use super::*;
use crate::core::summary_edits::{Field, Store};

#[test]
fn regeneration_keeps_edits_and_sends_corrections_to_notes() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let path = folder.join("english-summary.json");
    let mut old: serde_json::Value = serde_json::from_str(&summary()).unwrap();
    old["context"] = serde_json::json!(["Old context"]);
    fs::write(&path, old.to_string()).unwrap();
    let store = Store::new(&root.0, &folder, ID);
    for (field, text) in [
        (Field::Title, "Correct title"),
        (Field::Context, "Correct context"),
    ] {
        store
            .save(field, &store.current(field).unwrap(), text)
            .unwrap();
    }
    let s = settings_fixture();
    let mut fresh = old.clone();
    fresh["title"] = serde_json::json!("Regenerated title");
    let mut fake = Fake::new(vec![Ok(fresh.to_string()), Ok(sorted())]);
    queue::regenerate_in(&root.0, folder.parent().unwrap(), ID, &s).unwrap();
    drain(&root.0, &s, &mut fake);

    assert_eq!(
        fake.calls.len(),
        2,
        "confirmed TryAgain must run notes and tasks"
    );
    let prompt = &fake.calls[0].0;
    assert!(prompt.contains(r#""Plan the delivery" -> "Correct title""#));
    assert!(prompt.contains(r#""Old context" -> "Correct context""#));
    let mut view = store.generated().unwrap();
    assert_eq!(view["title"], "Regenerated title");
    assert_eq!(view["context"], serde_json::json!(["Old context"]));
    let aside = store.overlay(&mut view).unwrap();
    assert_eq!(aside.len(), 1);
    assert_eq!(aside[0].text, "Correct title");
    assert_eq!(view["context"], serde_json::json!(["Correct context"]));
}

#[test]
fn regeneration_retries_only_sorting_after_publication() {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let path = folder.join("english-summary.json");
    let original = summary();
    fs::write(&path, &original).unwrap();
    let s = settings_fixture();
    queue::regenerate_in(&root.0, folder.parent().unwrap(), ID, &s).unwrap();

    let mut busy = Fake::new(vec![Err(Failure::Busy)]);
    drain(&root.0, &s, &mut busy);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(!queue::read(&root.0, ID).unwrap().unwrap().sorting_pending);

    let mut first = Fake::new(vec![
        Ok(original.replace("Plan the delivery", "Regenerated title")),
        Err(Failure::Busy),
    ]);
    drain(&root.0, &s, &mut first);
    let published = fs::read_to_string(&path).unwrap();
    assert!(published.contains("Regenerated title"));
    let mut retry = Fake::new(vec![Ok(sorted())]);
    drain(&root.0, &s, &mut retry);
    assert_eq!(retry.calls.as_slice(), &first.calls[1..]);
    assert_eq!(fs::read_to_string(&path).unwrap(), published);
    assert!(matches!(state(&root.0), JobState::Done { .. }));
    assert!(folder.join("tasks.json").is_file());
}

fn assert_task_retry_uses_edited_notes(request: Request) {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let path = folder.join("english-summary.json");
    let original = summary();
    fs::write(&path, &original).unwrap();
    let store = Store::new(&root.0, &folder, ID);
    store
        .save(
            Field::Title,
            &store.current(Field::Title).unwrap(),
            "Edited for tasks",
        )
        .unwrap();
    let s = settings_fixture();
    enqueue(&root.0, &s, Some(request));
    let mut responses = Vec::new();
    if request == Request::RedoTasks {
        responses.push(Ok(
            r#"{"todo":[{"title":"Send report","description":"Share the figures"}]}"#.into(),
        ));
    }
    responses.push(Ok(sorted()));
    let mut fake = Fake::new(responses);
    drain(&root.0, &s, &mut fake);

    assert!(fake.calls[0].0.contains("Edited for tasks"), "{request:?}");
    if request == Request::SortAgain {
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
    assert_eq!(store.generated().unwrap()["title"], "Plan the delivery");
    assert_eq!(
        store.current(Field::Title).unwrap().text,
        "Edited for tasks"
    );
}

#[test]
fn sorting_reads_edited_notes() {
    assert_task_retry_uses_edited_notes(Request::SortAgain);
}

#[test]
fn redoing_tasks_reads_edited_notes() {
    assert_task_retry_uses_edited_notes(Request::RedoTasks);
}
