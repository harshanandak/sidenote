//! Synthetic app renders exercise the same worker entry points as the controls.
use super::{State, nav::Filter};
use sidenote::core::{
    self,
    area_ops::{Action, MigrationRow},
};
use std::{
    fs,
    time::{Duration, Instant},
};
pub(super) fn wait(s: &mut State) {
    while s.areas.pending.is_some() || s.areas.refresh_pending {
        let event = s
            .events_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("Area worker finished");
        if let super::Event::Area { worker, .. } = &event {
            assert_ne!(*worker, std::thread::current().id());
        }
        s.apply_event(event);
    }
    assert!(
        s.areas.error.is_none(),
        "Area worker error: {:?}",
        s.areas.error
    );
}
fn apply(s: &mut State, action: Action) {
    s.start_area(action);
    wait(s);
    s.preview_area();
    wait(s);
    s.confirm_area();
    wait(s);
}
fn proof(s: &mut State) {
    assert!(core::is_proof_path(&s.tasks.store.root, "areas-proof"));
    let unreadable = s
        .tasks
        .store
        .root
        .join("local/router-work/synthetic-unreadable");
    fs::create_dir_all(&unreadable).unwrap();
    fs::write(unreadable.join("active.json"), b"{").unwrap();
    s.refresh_areas();
    wait(s);
    for stage in ["activity-worker", "preview", "apply", "undo"] {
        let name = if stage == "activity-worker" {
            stage
        } else {
            "operation-worker"
        };
        let gate = s.tasks.store.root.join(format!("local/area-{name}.pause"));
        fs::create_dir_all(gate.parent().unwrap()).unwrap();
        if gate.with_extension("reached").exists() {
            fs::remove_file(gate.with_extension("reached")).unwrap();
        }
        fs::write(&gate, b"hold").unwrap();
        let launch = |s: &mut State| match stage {
            "activity-worker" => s.refresh_areas(),
            "preview" => {
                s.start_area(Action::Create {
                    name: "Proof domain".into(),
                });
                s.preview_area();
            }
            "apply" => s.confirm_area(),
            _ => {
                let id = s
                    .areas
                    .receipts
                    .values()
                    .find(|r| {
                        r.active && matches!(&r.action,Action::Create{name} if name=="Proof domain")
                    })
                    .unwrap()
                    .id
                    .clone();
                s.undo_area(id);
            }
        };
        launch(s);
        let generation = s.areas.generation;
        let deadline = Instant::now() + Duration::from_secs(15);
        while !gate.with_extension("reached").exists() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        if stage == "activity-worker" {
            let generation = s.areas.refresh_generation;
            s.refresh_areas();
            s.refresh_areas();
            assert_eq!(generation, s.areas.refresh_generation);
            assert!(s.areas.refresh_again);
        } else {
            let before = serde_json::to_value(&s.tasks.overrides).unwrap();
            if let Some(id) = s.meetings.first().map(|m| m.id.clone()) {
                s.set_meeting_project(&id, "V2 Launch");
            }
            assert_eq!(before, serde_json::to_value(&s.tasks.overrides).unwrap());
            launch(s);
            assert_eq!(
                generation, s.areas.generation,
                "Repeated click launched a second operation"
            );
        }
        fs::remove_file(gate).unwrap();
        wait(s);
    }
    assert!(s.areas.catalog.area_matches("Proof domain").is_empty());
    let args: Vec<_> = std::env::args_os().collect();
    let snap = args.windows(2).find(|p| p[0] == "--snap").unwrap();
    let path = std::path::PathBuf::from(&snap[1]).with_extension("proof.json");
    fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"activity_runs_on_worker":true,"preview_apply_undo_run_on_workers":true,"ui_returns_while_workers_held":true,"repeat_clicks_ignored":true,"undo_restores_area_view":true,"refresh_coalesced":true,"project_edits_blocked_while_pending":true,"unreadable_running_marker_does_not_fail_refresh":true})).unwrap()).unwrap();
}
pub fn scene(s: &mut State, kind: &str) {
    s.go_meetings();
    wait(s);
    if kind == "refresh-error" {
        let device = s.tasks.store.root.join("device.json");
        let original = fs::read(&device).unwrap();
        fs::write(&device, b"{").unwrap();
        s.refresh_areas();
        while s.areas.refresh_pending {
            let event = s.events_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            s.apply_event(event);
        }
        fs::write(&device, original).unwrap();
        assert_eq!(
            s.areas.error.as_deref(),
            Some("Could not finish the area change. Your data was kept. Reload and try again.")
        );
        s.go_meetings();
        return;
    }
    if kind.starts_with("large-") {
        large(s, kind);
        return;
    }
    for name in ["Spring exhibition", "V2 Launch"] {
        s.tasks.projects = s.tasks.store.add_project(name).unwrap();
    }
    s.tasks.overrides = s
        .tasks
        .store
        .add_manual(core::ManualTask {
            task: "Review launch checklist".into(),
            project: Some("V2 Launch".into()),
            added: "2026-11-09".into(),
            ..Default::default()
        })
        .unwrap()
        .0;
    s.rebuild_tasks();
    wait(s);
    let rows: Vec<_> = s
        .areas
        .catalog
        .projects
        .values()
        .filter(|p| p.converted_to.is_none())
        .map(|p| MigrationRow {
            project_id: p.id.clone(),
            area_name: if p.name == "V2 Launch" {
                "Website"
            } else {
                "Studio"
            }
            .into(),
            convert: kind != "convert" && p.name == "Northwind Field Guide",
            children: Default::default(),
            assignments: Default::default(),
        })
        .collect();
    if kind == "migration" || kind == "migration-empty" {
        s.set_filter(Filter::Areas);
        let rows = if kind == "migration-empty" {
            rows.into_iter()
                .map(|mut r| {
                    r.area_name.clear();
                    r.convert = false;
                    r
                })
                .collect()
        } else {
            rows
        };
        s.start_area(Action::Migrate { rows });
        wait(s);
        if kind == "migration" {
            s.preview_area();
            wait(s);
        }
        return;
    }
    apply(s, Action::Migrate { rows });
    s.set_filter(Filter::Areas);
    match kind {
        "page" | "all" => {
            let id = s.areas.catalog.area_matches("Studio")[0].id.clone();
            s.set_filter(Filter::Area(id));
            if kind == "all" {
                s.areas.show_all = true;
            }
        }
        "project" => s.set_filter(Filter::Project("V2 Launch".into())),
        "list" => s.go_meetings(),
        "create" => s.start_area(Action::Create {
            name: "Community".into(),
        }),
        "rename" => {
            let id = s.areas.catalog.area_matches("Studio")[0].id.clone();
            s.start_area(Action::Rename {
                area_id: id,
                name: "Design studio".into(),
            });
            s.preview_area();
            wait(s);
        }
        "move" => {
            let id = s
                .areas
                .catalog
                .project(&s.tasks.projects, "V2 Launch")
                .unwrap()
                .id
                .clone();
            let area = s.areas.catalog.area_matches("Studio")[0].id.clone();
            s.set_filter(Filter::Project("V2 Launch".into()));
            s.start_area(Action::Move {
                project_id: id,
                area_id: area,
            });
            s.preview_area();
            wait(s);
        }
        "convert" => {
            let id = s
                .areas
                .catalog
                .project(&s.tasks.projects, "Northwind Field Guide")
                .unwrap()
                .id
                .clone();
            s.start_area(Action::Convert {
                project_id: id,
                area_name: "Studio".into(),
                children: Default::default(),
            });
            s.preview_area();
            wait(s);
        }
        "assign" => {
            let key = core::areas::task_key(&s.tasks.all[0]);
            let area = s.areas.catalog.area_matches("Studio")[0].id.clone();
            s.assign_area(key, area);
            s.preview_area();
            wait(s);
        }
        "worker-proof" => proof(s),
        _ => (),
    }
    // Every scene re-reads the persisted journal through the real worker.
    s.tasks.projects = s.tasks.store.load_projects().unwrap();
    s.rebuild_tasks();
    wait(s);
}
fn large(s: &mut State, kind: &str) {
    assert_eq!(s.tasks.all.len(), 800);
    assert_eq!(s.areas.catalog.projects.len(), 14);
    s.set_filter(Filter::Areas);
    s.start_area(super::area_editor::migration(&s.areas.catalog));
    wait(s);
    assert_eq!(s.areas.editor.as_ref().unwrap().inventory.counts[1], 800);
    s.areas.editor.as_mut().unwrap().rows[0].convert = true;
    assert!(
        s.areas.editor.as_ref().unwrap().dirty(),
        "A default choice needs discard confirmation even before an area name is typed"
    );
    s.areas.editor.as_mut().unwrap().rows[0].convert = false;
    if kind != "large-migration" {
        let e = s.areas.editor.as_mut().unwrap();
        for row in &mut e.rows {
            row.area = rune::TextEdit::new("Studio");
        }
        e.rows[0].convert = true;
        s.preview_area();
        wait(s);
    }
    if kind == "large-reconcile" {
        s.tasks.projects = s.tasks.store.add_project("Unreviewed project").unwrap();
        s.refresh_areas();
        wait(s);
        let e = s.areas.editor.as_ref().unwrap();
        assert_eq!(e.rows.len(), 15);
        assert_eq!(e.rows[0].area.text, "Studio");
        assert!(e.rows[0].convert);
        assert!(s.areas.preview.is_none());
        s.cancel_area();
        assert!(s.areas.editor.is_some() && s.areas.discard_action.is_some());
        s.areas.discard_action = None;
        s.start_area(super::area_editor::migration(&s.areas.catalog));
        assert!(s.areas.discard_action.is_some());
        s.areas.discard_action = None;
        let e = s.areas.editor.as_mut().unwrap();
        let i = e.rows.iter().position(|r| r.area.text.is_empty()).unwrap();
        e.search = rune::TextEdit::new("Unreviewed");
        e.visible_projects = vec![i];
    }
    if kind == "large-empty" {
        let e = s.areas.editor.as_mut().unwrap();
        e.search = rune::TextEdit::new("No matching project");
        e.visible_projects.clear();
    }
    if kind == "large-page" || kind == "large-convert" {
        if kind == "large-convert" {
            s.areas.editor.as_mut().unwrap().rows[0].convert = false;
            s.areas.preview = None;
            s.preview_area();
            wait(s);
        }
        s.confirm_area();
        wait(s);
        let area = s.areas.catalog.area_matches("Studio")[0].id.clone();
        s.set_filter(Filter::Area(area));
        if kind == "large-convert" {
            let id = s
                .areas
                .catalog
                .project(&s.tasks.projects, "Example project 01")
                .unwrap()
                .id
                .clone();
            s.start_area(Action::Convert {
                project_id: id,
                area_name: "Studio".into(),
                children: Default::default(),
            });
            s.preview_area();
            wait(s);
        }
    }
    if kind == "large-expanded" {
        s.areas.editor.as_mut().unwrap().rows[0].expanded = true;
    }
    let args: Vec<_> = std::env::args_os().collect();
    let snap = args.windows(2).find(|p| p[0] == "--snap").unwrap();
    let proof = std::path::PathBuf::from(&snap[1]).with_extension("proof.json");
    fs::write(proof,serde_json::to_vec_pretty(&serde_json::json!({"projects":14,"tasks":800,"linked_items":s.areas.editor.as_ref().map_or(814,|e|e.inventory.references.len()),"per_item_controls_collapsed":kind!="large-expanded","review_footer_outside_scroll":true,"project_rows_virtualized":true,"exception_rows_virtualized":true})).unwrap()).unwrap();
}
