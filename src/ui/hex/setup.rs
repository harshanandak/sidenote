use super::*;
use crate::ui::widgets::{ButtonRole, button};
use sidenote::hex_install::{self, Installation, State as InstallState};

impl State {
    fn enable_hex_meetings(&mut self) {
        if super::super::isolated()
            || meetings_step(&self.hex) != Some(true)
            || self.hex.busy_keys.contains("meetings.enabled")
        {
            return;
        }
        self.hex.busy_keys.insert("meetings.enabled".into());
        self.hex.row_errors.remove("meetings.enabled");
        self.hex_spawn(|engine| Msg::Meetings(engine.enable_meetings()));
    }

    fn recheck_hex_meetings(&mut self) {
        if super::super::isolated() || self.hex.busy_keys.contains("meetings.enabled") {
            return;
        }
        self.hex.data_dirty = true;
        self.hex_refresh_if_due();
        self.hex_spawn(|engine| Msg::ObservedStatus(sample_status(|| engine.status())));
    }

    pub fn hex_needs_install(&self) -> bool {
        self.hex.installation == Installation::Missing
    }

    fn install_hex(&mut self) {
        if super::super::isolated() || self.hex.running() {
            return;
        }
        self.hex.installation = hex_install::detect();
        if !self.hex.install.begin(&self.hex.installation) {
            return;
        }
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        if thread::Builder::new()
            .name("hex-install".into())
            .spawn(move || {
                hex_install::install_after_click(|state| {
                    let _ = events.send(Event::Hex(Msg::Install(state)));
                    if let Some(waker) = &waker {
                        waker();
                    }
                });
            })
            .is_err()
        {
            self.hex.install = InstallState::Failed("Could not start Hex setup. Try again.");
        }
    }

    #[cfg(windows)]
    fn open_installed_hex(&mut self) {
        if super::super::isolated() {
            return;
        }
        self.hex.installation = hex_install::detect();
        if let Installation::Installed { exe, .. } = &self.hex.installation {
            use std::{
                os::windows::process::CommandExt,
                process::{Command, Stdio},
            };
            if Command::new(exe)
                .arg("app")
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_err()
            {
                self.hex.install =
                    InstallState::Failed("Could not open Hex. Open it from the Start menu.");
            }
        }
    }

    #[cfg(windows)]
    fn hex_download_page(&mut self) {
        if super::super::isolated() {
            return;
        }
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let url: Vec<u16> = hex_install::DOWNLOAD_PAGE
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        // SAFETY: both strings are NUL-terminated and live through this call; URL is constant.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            self.hex.install = InstallState::Failed(
                "Could not open the download page. Visit github.com/anomalyco/hex/releases/latest.",
            );
        }
    }
    #[cfg(not(windows))]
    fn open_installed_hex(&mut self) {
        self.hex.install = InstallState::Failed("Opening Hex is unavailable on Linux.");
    }
    #[cfg(not(windows))]
    fn hex_download_page(&mut self) {
        self.hex.install = InstallState::Failed("Hex installation is unavailable on Linux.");
    }
}

/// None means no step, false means manual instructions, true allows an explicit supported write.
fn meetings_step(h: &HexState) -> Option<bool> {
    (h.state()?.labs.state("meetings") == Some(false)).then(|| {
        h.can(Feature::SettingsGet)
            && h.can(Feature::SettingsSet)
            && sidenote::hex_labs::meetings_setting_can_enable(&h.settings)
    })
}

fn meetings_card(s: &State, p: Palette) -> Option<El> {
    let can_enable = meetings_step(&s.hex)?;
    let busy = s.hex.busy_keys.contains("meetings.enabled");
    let mut actions = row().w_full().flex_wrap().gap(8.0);
    if can_enable {
        actions = actions.child(
            button(
                "Turn on Meetings",
                "hex-meetings-enable",
                ButtonRole::Primary,
                p,
            )
            .disabled(busy)
            .on_press(|s: &mut State| s.enable_hex_meetings()),
        );
    }
    actions = actions.child(
        button(
            "Check again",
            "hex-meetings-recheck",
            ButtonRole::Secondary,
            p,
        )
        .disabled(busy)
        .on_press(|s: &mut State| s.recheck_hex_meetings()),
    );
    Some(col().key("hex-meetings-setup").w_full().px(20.0).py(20.0).mt(16.0).mb(16.0).gap(12.0)
        .fill(p.card).border(1.0, p.line).radius(10.0)
        .child(roles::SECTION.wrap("Turn on Meetings in Hex", p.ink))
        .child(roles::SMALL.wrap("Meetings is a beta feature in Hex Labs. Turn it on so Sidenote can record and transcribe meetings.", p.soft))
        .child(roles::SMALL.wrap("In Hex, open Settings > Labs > Meetings and turn it on, then check again.", p.soft).key("hex-meetings-instructions"))
        .children(busy.then(|| roles::SMALL.wrap("Turning on Meetings and checking Hex.", p.soft).role("status")))
        .children(s.hex.row_errors.get("meetings.enabled").map(|error| roles::SMALL.wrap(error, p.warn).role("alert")))
        .child(actions))
}

pub(super) fn card(s: &State, p: Palette) -> Option<El> {
    let h = &s.hex;
    if cfg!(target_os = "linux") && !h.running() {
        return Some(col().w_full().gap(12.0).key("hex-setup-unavailable")
            .child(roles::SECTION.wrap("Hex on Linux", p.ink))
            .child(roles::SMALL.wrap("Hex installation and recording control are unavailable in this Linux desktop launch layer.", p.soft)));
    }
    if h.running() && !h.install.busy() {
        return meetings_card(s, p);
    }
    // Without a fresh answer Hex may well be running; the page note says so instead.
    if !h.stopped()
        && matches!(h.install, InstallState::Idle)
        && matches!(h.installation, Installation::Installed { .. })
    {
        return None;
    }
    let (title, message) = match (&h.install, &h.installation) {
        (InstallState::Idle, Installation::Installed { version, .. }) => (
            "Open Hex", format!("Hex {version} is installed. Open Hex to record and transcribe."),
        ),
        (InstallState::Idle, Installation::Incomplete) => (
            "Check the Hex installation", "Sidenote found Hex files but could not read a working installation. Use the download page to repair it, then check again.".into(),
        ),
        (InstallState::Complete, _) => ("Hex is installed", h.install.message()),
        (InstallState::Failed(_), _) => ("Hex setup needs attention", h.install.message()),
        _ => ("Install Hex", h.install.message()),
    };
    let mut actions = row().w_full().flex_wrap().gap(8.0);
    if matches!(h.installation, Installation::Installed { .. }) && h.stopped() {
        actions = actions.child(
            button("Open Hex", "hex-setup-open", ButtonRole::Primary, p)
                .disabled(h.install.busy())
                .on_press(|s: &mut State| s.open_installed_hex()),
        );
    } else {
        actions = actions
            .children(
                (h.installation == Installation::Missing && !h.running()).then(|| {
                    button(
                        if matches!(h.install, InstallState::Failed(_)) {
                            "Try again"
                        } else {
                            "Install Hex"
                        },
                        "hex-install",
                        ButtonRole::Primary,
                        p,
                    )
                    .disabled(h.install.busy())
                    .on_press(|s: &mut State| s.install_hex())
                }),
            )
            .child(
                button(
                    "Download page",
                    "hex-download-page",
                    ButtonRole::Secondary,
                    p,
                )
                .on_press(|s: &mut State| s.hex_download_page()),
            )
            .child(
                button("Check again", "hex-install-recheck", ButtonRole::Ghost, p)
                    .disabled(h.install.busy())
                    .on_press(|s: &mut State| {
                        if super::super::isolated() {
                            return;
                        }
                        s.hex.installation = hex_install::detect();
                        s.hex.install = InstallState::Idle;
                        s.hex_refresh_if_due();
                    }),
            );
    }
    Some(col().key("hex-setup").w_full().px(20.0).py(20.0).mt(16.0).mb(16.0).gap(12.0)
        .fill(p.card).border(1.0, p.line).radius(10.0)
        .child(roles::SECTION.wrap(title, p.ink))
        .child(roles::SMALL.wrap(message, if matches!(h.install, InstallState::Failed(_)) { p.warn } else { p.soft })
            .key("hex-install-progress").role(if matches!(h.install, InstallState::Failed(_)) { "alert" } else { "status" }))
        .children((h.install == InstallState::Idle && h.installation == Installation::Missing).then(||
            roles::META.wrap("Downloads the latest stable Windows installer from Hex on GitHub. Checks its SHA-256 and publisher before installing. No administrator access. Hex may download a speech model when you open it.", p.faint)))
        .child(actions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meetings_off_requires_setup_and_only_offers_a_supported_writable_command() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../tools/ui-fixtures/hex.json")).unwrap();
        let mut hex = HexState {
            status: Some(Ok(EngineState {
                running: true,
                version: "3.4.0".into(),
                protocol: 4,
                recording: None,
                meeting_state: "idle".into(),
                dictating: Some(false),
                capture_state: "idle".into(),
                model: fixture.model,
                labs: Box::new(sidenote::hex_labs::Snapshot::status(&serde_json::json!({
                    "features": {"meetings": false}, "labs": []
                }))),
            })),
            ..HexState::default()
        };
        assert_eq!(meetings_step(&hex), Some(false));
        hex.caps = Some(
            [
                (Feature::SettingsGet, Support::Supported),
                (Feature::SettingsSet, Support::Supported),
            ]
            .into(),
        );
        assert_eq!(meetings_step(&hex), Some(false));
        hex.settings = vec![serde_json::from_value(serde_json::json!({
            "key": "meetings.enabled", "value": false, "kind": "bool", "writable": true, "available": true
        })).unwrap()];
        assert_eq!(meetings_step(&hex), Some(true));
        for support in [Support::Unsupported, Support::Unknown] {
            hex.caps
                .as_mut()
                .unwrap()
                .insert(Feature::SettingsSet, support);
            assert_eq!(meetings_step(&hex), Some(false));
        }
        hex.caps
            .as_mut()
            .unwrap()
            .insert(Feature::SettingsSet, Support::Supported);
        hex.settings[0].writable = false;
        assert_eq!(meetings_step(&hex), Some(false));
        hex.settings[0].writable = true;
        hex.settings[0].available = Some(false);
        assert_eq!(meetings_step(&hex), Some(false));
        hex.status
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .labs
            .states
            .insert("meetings".into(), Some(true));
        assert_eq!(meetings_step(&hex), None);
        hex.status
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()
            .labs
            .states
            .clear();
        assert_eq!(meetings_step(&hex), None);
        hex.status = Some(Err(HexError::HexNotRunning));
        assert_eq!(meetings_step(&hex), None);
    }
}
