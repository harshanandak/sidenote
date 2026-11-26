use super::*;

pub(super) fn windows_only_hidden(st: &Setting) -> bool {
    cfg!(windows)
        && (st.key == "show_dock_icon"
            || st.key.starts_with("meetings.calendar.")
            || st.key.starts_with("experimental.hex_microphone"))
        && st.available == Some(false)
}
pub(super) fn supported_choice(value: &Value) -> bool {
    !cfg!(windows)
        || !value
            .as_str()
            .is_some_and(|v| v.starts_with("mac") || matches!(v, "apple" | "notch" | "menu_bar"))
}
pub(super) fn value_label(st: &Setting, value: &Value) -> String {
    let Some(raw) = value.as_str() else {
        return match value {
            Value::Object(_) | Value::Array(_) => "Managed in Hex".into(),
            Value::Bool(true) => "On".into(),
            Value::Bool(false) => "Off".into(),
            _ => setting_text(value),
        };
    };
    if !supported_choice(value) {
        return "Unavailable on Windows".into();
    }
    match raw {
        "off" => "Off",
        "on" => "On",
        "auto" => "Automatic",
        "system" => "Follow Windows",
        "en" => "English",
        "hi" => "Hindi",
        "pl" => "Polish",
        "zh" => "Chinese",
        "zh-Hant" => "Traditional Chinese",
        "ja" => "Japanese",
        "de" => "German",
        "es" => "Spanish",
        "uk" => "Ukrainian",
        "pt" => "Portuguese",
        "fr" => "French",
        "ru" => "Russian",
        "day" => "24 hours",
        "week" => "7 days",
        "month" => "1 month",
        "forever" => "Forever",
        "opencode" => "OpenCode",
        "top_right" => "Top right",
        "top_left" => "Top left",
        "bottom_right" => "Bottom right",
        "bottom_left" => "Bottom left",
        "gpu" => "GPU",
        "cpu" => "CPU",
        "dev" => "Development",
        "pause_media" => "Pause media",
        "do_nothing" => "Leave audio unchanged",
        "duck" => "Lower other audio",
        "mute" => "Mute other audio",
        "typing" => "Type text",
        "clipboard" => "Clipboard",
        "ask" => "Ask",
        "always_record" => "Always record",
        "never_offer" => "Never offer",
        "openai/gpt-6.1-terra-fast" => "GPT-6.1 Terra Fast",
        "parakeet-v3" => "Parakeet v3",
        _ if st.key.contains("model") => "Custom model",
        _ if st.kind == "choice" => return pretty(raw),
        _ => return raw.to_string(),
    }
    .into()
}
pub(super) fn current_label(st: &Setting) -> String {
    if st.secret {
        return "Hidden by Hex".into();
    }
    if !supported_choice(&st.value) {
        return "Unavailable on Windows".into();
    }
    if st.kind == "choice" && !st.choices.contains(&st.value) {
        return "Unrecognized option. Choose a supported option.".into();
    }
    value_label(st, &st.value)
}

fn policy(raw: &str) -> Option<&'static str> {
    match raw {
        "ask" => Some("Ask"),
        "always_record" => Some("Always record"),
        "never_offer" => Some("Never offer"),
        _ => None,
    }
}
fn changed_policy(value: &Value, index: usize, selected: &str) -> Option<Value> {
    let raw = match selected {
        "Ask" => "ask",
        "Always record" => "always_record",
        "Never offer" => "never_offer",
        _ => return None,
    };
    let mut out = value.clone();
    let app = out.as_array_mut()?.get_mut(index)?.as_object_mut()?;
    policy(app.get("policy")?.as_str()?)?;
    app.insert("policy".into(), Value::String(raw.into()));
    Some(out)
}
pub(super) fn providers(s: &State, k: &Kit, st: &Setting, p: Palette, layer: &mut Vec<El>) -> El {
    let mut body = col().w_full().key(format!("hs-{}", st.key));
    let Some(apps) = st.value.as_array() else {
        return body.child(
            roles::SMALL
                .wrap(
                    "Call apps could not be read. Open Hex to review this setting.",
                    p.warn,
                )
                .padding(14.0),
        );
    };
    for (index, app) in apps.iter().enumerate() {
        let label = app
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("Unrecognized app");
        let current = app.get("policy").and_then(Value::as_str).and_then(policy);
        let key = format!("hex-call-app-{label}-{index}");
        let open = s.hex.select_open.as_deref() == Some(&key);
        let toggle = key.clone();
        let setting_key = st.key.clone();
        let app_key = app.get("key").cloned();
        let can = editable(st)
            && s.hex.can(Feature::SettingsSet)
            && !s.hex.busy_keys.contains(&st.key)
            && current.is_some()
            && app_key.is_some();
        let control = Select::new(
            &key,
            &["Ask", "Always record", "Never offer"].map(str::to_string),
        )
        .label(format!("{label} recording policy"))
        .current(current)
        .placeholder("Unrecognized policy")
        .appearance(SelectAppearance::Field)
        .width(settings::short_select_width(s, 220.0))
        .disabled(!can)
        .open(open && can)
        .on_toggle(move |s: &mut State| {
            s.hex.select_open = if s.hex.select_open.as_deref() == Some(&toggle) {
                None
            } else {
                Some(toggle.clone())
            };
        })
        .on_close(|s: &mut State| s.hex.select_open = None)
        .on_choose(move |s: &mut State, v: &str, _: &mut Tx| {
            if let Some(value) = s.hex.setting(&setting_key).and_then(|st| {
                st.value
                    .as_array()
                    .and_then(|apps| {
                        apps.iter()
                            .position(|app| app.get("key") == app_key.as_ref())
                    })
                    .and_then(|at| changed_policy(&st.value, at, v))
            }) {
                s.hex_choose(&setting_key, &value.to_string());
            }
        })
        .build(&settings::row_field_kit(k, p), layer);
        body=body.child(settings::setting_row(s,&key,label,if current.is_some() { "When Hex detects a call in this app." } else { "This policy cannot be edited here. Review it in Hex. Other app choices are preserved." },control,false,p));
    }
    if let Some(error) = s.hex.row_errors.get(&st.key) {
        body = body.child(roles::SMALL.wrap(error, p.warn).role("alert").padding(14.0));
    }
    body
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_supported_provider_json_editor_is_writable() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../tools/ui-fixtures/hex.json")).unwrap();
        let mut st = fixture.settings[0].clone();
        st.writable = true;
        st.kind = "json".into();
        st.key = "meetings.providers".into();
        st.value = serde_json::json!([]);
        assert!(editable(&st));
        st.secret = true;
        assert!(!editable(&st));
        st.secret = false;
        st.key = "unknown.structured".into();
        assert!(!editable(&st));
    }
    #[test]
    fn policy_edit_preserves_unknown_apps_and_fields() {
        let original = serde_json::json!([{"key":"zoom","label":"Zoom","policy":"ask","future":7},{"key":"other","policy":"unknown"}]);
        let changed = changed_policy(&original, 0, "Always record").unwrap();
        assert_eq!(changed[0]["policy"], "always_record");
        assert_eq!(changed[0]["future"], 7);
        assert_eq!(changed[1], original[1]);
        assert!(changed_policy(&original, 1, "Ask").is_none());
        assert!(changed_policy(&original, 2, "Ask").is_none());
    }
    #[test]
    fn choices_are_readable_without_changing_transport_values() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../tools/ui-fixtures/hex.json")).unwrap();
        let mut st = fixture.settings[0].clone();
        st.key = "meetings.recording_indicator_corner".into();
        assert_eq!(
            value_label(&st, &Value::String("top_right".into())),
            "Top right"
        );
        st.key = "about.build".into();
        st.kind = "text".into();
        st.value = Value::String("2.4.1 (build 812)".into());
        assert_eq!(current_label(&st), "2.4.1 (build 812)");
        st.secret = true;
        st.value = Value::String("should never be displayed".into());
        assert_eq!(current_label(&st), "Hidden by Hex");
    }
}
