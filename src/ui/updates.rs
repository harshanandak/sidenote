//! Quiet update notices and the device's Updates settings.
use super::{
    Event, State, hex, roles,
    theme::Palette,
    widgets::{ButtonDisabled, hairline},
};
use rune::{El, col, el, row};
use sidenote::core::{CHANNEL, Channel};
use sidenote::listener::heartbeat;
use sidenote::updates::{self, Phase, Status, WindowStart};
use std::time::Duration;

const LISTENER_INTERVAL: Duration = Duration::from_secs(5);

pub struct Updates {
    pub status: Status,
    channel: Channel,
    listener: updates::ListenerStatus,
    listener_busy: bool,
    pub(super) listener_open: bool,
    listener_probe_busy: bool,
    listener_probed: Option<std::time::Instant>,
    recorder_gap: Option<heartbeat::Gap>,
    dismissed_gap: Option<heartbeat::Gap>,
    service: Option<std::sync::mpsc::Sender<updates::CheckRequest>>,
    dismissed: bool,
    notes_open: bool,
    scene: bool,
    applying: bool,
    reconcile_pending: bool,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            status: updates::load(),
            channel: updates::channel(),
            listener: updates::listener_status(),
            listener_busy: false,
            listener_open: false,
            listener_probe_busy: false,
            listener_probed: None,
            recorder_gap: None,
            dismissed_gap: None,
            service: None,
            dismissed: false,
            notes_open: false,
            scene: false,
            applying: false,
            reconcile_pending: false,
        }
    }
}
impl Updates {
    fn worker_busy(&self) -> bool {
        self.listener_busy || self.listener_probe_busy || self.applying
    }
    fn reconcile_retry_due(&self, elapsed: Duration) -> bool {
        self.reconcile_pending && elapsed >= LISTENER_INTERVAL && !self.worker_busy()
    }
    fn can_apply(&self) -> bool {
        self.status.phase == Phase::Ready && !self.applying && !self.reconcile_pending
    }
    fn feed_status(&mut self, status: Status) {
        self.status = status;
    }
    fn listener_probed(&mut self, status: Option<updates::ListenerStatus>) {
        self.listener_probe_busy = false;
        if !self.listener_busy
            && !self.applying
            && let Some(status) = status
        {
            if status.error.is_none() {
                self.listener_open = false;
            }
            self.listener = status;
        }
    }
}
impl State {
    pub fn listener_tick(&mut self, tx: &mut rune::Tx) {
        if !updates::installed() || self.updates.scene || rune::tooling::headless_run().is_some() {
            return;
        }
        let interval = LISTENER_INTERVAL;
        let elapsed = self
            .updates
            .listener_probed
            .map(|at| at.elapsed())
            .unwrap_or(interval);
        tx.wake_after(if elapsed < interval {
            interval - elapsed
        } else {
            interval
        });
        if self.updates.reconcile_retry_due(elapsed) {
            self.updates_start();
            return;
        }
        if elapsed < interval
            || self.updates.worker_busy()
            || matches!(
                self.updates.status.phase,
                Phase::Preparing | Phase::StoppingListener
            )
        {
            return;
        }
        self.updates.listener_probe_busy = true;
        self.updates.listener_probed = Some(std::time::Instant::now());
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let status = updates::probe_listener();
            let gap = heartbeat::pending_gap();
            let _ = events.send(Event::ListenerProbed(status, gap));
            if let Some(wake) = wake {
                wake();
            }
        });
    }
    pub fn listener_probed(
        &mut self,
        status: Option<updates::ListenerStatus>,
        gap: Option<heartbeat::Gap>,
    ) {
        if !self.updates.listener_busy && !self.updates.applying && status.is_some() {
            self.updates.recorder_gap =
                gap.filter(|gap| self.updates.dismissed_gap.as_ref() != Some(gap));
        }
        self.updates.listener_probed(status);
    }
    pub fn updates_start(&mut self) {
        if !updates::installed() || self.updates.worker_busy() {
            return;
        }
        self.updates.listener_busy = true;
        self.updates.applying = true;
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        let status = self.updates.status.clone();
        std::thread::spawn(move || {
            let result = updates::window_start(|phase| {
                let _ = events.send(Event::Update(Status {
                    phase,
                    ..status.clone()
                }));
                if let Some(wake) = &wake {
                    wake();
                }
            });
            let _ = events.send(Event::UpdateFinished(result));
            if let Some(wake) = &wake {
                wake();
            }
        });
    }
    pub fn update_status(&mut self, status: Status) {
        if status
            .channel
            .as_deref()
            .is_some_and(|name| !name.eq_ignore_ascii_case(channel_label(self.updates.channel)))
        {
            return;
        }
        let preparing = matches!(status.phase, Phase::Preparing | Phase::StoppingListener);
        if self.updates.applying && !preparing {
            return;
        }
        if preparing {
            self.updates.applying = true;
        }
        if self.updates.status.version != status.version && !status.version.is_empty() {
            self.updates.dismissed = false;
        }
        self.updates.feed_status(status);
    }
    pub fn update_finished(&mut self, result: Result<WindowStart, String>) {
        self.updates.applying = false;
        self.updates.listener_busy = false;
        self.updates.listener = updates::listener_status();
        self.updates.reconcile_pending = result
            .as_ref()
            .is_ok_and(|start| start.pending_status.is_some());
        if self.updates.reconcile_pending {
            self.updates.listener_probed = Some(std::time::Instant::now());
        } else if self.updates.service.is_none() && updates::installed() {
            let events = self.events_tx.clone();
            let wake = self.waker.clone();
            self.updates.service = updates::start(move |status| {
                let _ = events.send(Event::Update(status));
                if let Some(wake) = &wake {
                    wake();
                }
            });
        }
        match result {
            Ok(start) => self.updates.status = start.pending_status.unwrap_or_else(updates::load),
            Err(_) if !self.updates.listener.ready => self.updates.status = updates::load(),
            Err(error) => {
                self.updates.status.phase = if self.updates.status.version.is_empty() {
                    Phase::Error
                } else {
                    Phase::Ready
                };
                self.updates.status.error = Some(error);
            }
        }
    }
    pub fn listener_finished(&mut self, result: Result<(), String>) {
        self.updates.listener_busy = false;
        if result.is_ok() {
            self.updates.listener_open = false;
        }
        self.updates.listener = updates::ListenerStatus {
            ready: result.is_ok(),
            preparing: false,
            error: result.err(),
        };
    }
    pub(super) fn retry_listener(&mut self) -> Option<std::thread::JoinHandle<()>> {
        if self.updates.scene
            || self.updates.listener_busy
            || self.updates.applying
            || self.updates.reconcile_pending
            || rune::tooling::headless_run().is_some()
        {
            return None;
        }
        self.updates.listener_busy = true;
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        Some(std::thread::spawn(move || {
            let _ = events.send(Event::ListenerFinished(updates::ensure_listener()));
            if let Some(wake) = wake {
                wake();
            }
        }))
    }
    fn switch_update_channel(&mut self, channel: Channel) {
        if self.updates.applying || busy(&self.updates.status) {
            return;
        }
        if !self.updates.scene {
            if let Err(error) = updates::set_channel(channel) {
                self.updates.status.error = Some(error);
                self.updates.status.phase = Phase::Error;
                return;
            }
            self.updates.status = updates::load();
        }
        self.updates.channel = channel;
        self.updates.notes_open = false;
        self.check_updates();
    }

    fn check_updates(&mut self) {
        if self.updates.scene
            || self.updates.applying
            || self.updates.status.phase == Phase::Manual
            || busy(&self.updates.status)
        {
            return;
        }
        if let Some(service) = &self.updates.service {
            self.updates.status.phase = Phase::Checking;
            self.updates.status.error = None;
            if service.send(updates::CheckRequest::Manual).is_err() {
                self.updates.status.phase = Phase::Error;
                self.updates.status.error =
                    Some("The update check stopped. Reopen Sidenote to try again.".into());
            }
        }
    }
    pub fn updates_focus(&mut self) {
        if !updates::installed()
            || self.updates.scene
            || self.updates.applying
            || busy(&self.updates.status)
            || !self
                .updates
                .status
                .checked
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .is_some_and(|checked| {
                    chrono::Utc::now().signed_duration_since(checked)
                        > chrono::Duration::seconds(updates::FOCUS_CHECK_AFTER.as_secs() as i64)
                })
        {
            return;
        }
        if let Some(service) = &self.updates.service {
            let _ = service.send(updates::CheckRequest::Focus);
        }
    }
    fn install_update(&mut self) {
        if self.updates.scene
            || !self.updates.can_apply()
            || rune::tooling::headless_run().is_some()
        {
            return;
        }
        if self.hex_recording()
            || self.hex_record_busy()
            || !self.asks.is_empty()
            || self.live.is_some()
        {
            self.updates.status.error =
                Some("Finish recording and wait for your jobs before installing.".into());
            return;
        }
        self.updates.applying = true;
        self.updates.status.phase = Phase::Preparing;
        self.updates.status.error = None;
        let status = self.updates.status.clone();
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let result = updates::apply(true, Vec::new(), |phase| {
                let _ = events.send(Event::Update(Status {
                    phase,
                    ..status.clone()
                }));
                if let Some(wake) = &wake {
                    wake();
                }
            });
            let _ = events.send(Event::UpdateFinished(
                result.map(|()| WindowStart::default()),
            ));
            if let Some(wake) = &wake {
                wake();
            }
        });
    }
    fn open_updates(&mut self) {
        self.open_hex();
        self.hex.section = hex::Section::Updates;
        self.hex.filter = rune::TextEdit::default();
    }
}
fn busy(status: &Status) -> bool {
    matches!(
        status.phase,
        Phase::Checking
            | Phase::Downloading
            | Phase::Available
            | Phase::Preparing
            | Phase::StoppingListener
    )
}
fn install(s: &State, p: Palette, footer: bool) -> El {
    let ready = s.updates.can_apply()
        && !s.hex_recording()
        && !s.hex_record_busy()
        && s.asks.is_empty()
        && s.live.is_none()
        && !s.edit_busy
        && !s.annotations.working()
        && s.ai_notes.pending.is_none()
        && !s.ai_notes.jobs.values().any(|job| {
            matches!(
                job.state,
                sidenote::core::notes::JobState::Queued | sidenote::core::notes::JobState::Running
            )
        });
    let title = match s.updates.status.phase {
        Phase::Preparing => "Preparing update",
        Phase::StoppingListener => "Stopping the listener",
        _ => "Install and restart",
    };
    let b = crate::ui::widgets::button(
        title,
        if footer {
            "update-footer-install"
        } else {
            "update-install"
        },
        crate::ui::widgets::ButtonRole::Primary,
        p,
    )
    .label(if ready {
        "Install update and restart Sidenote"
    } else {
        "Update progress"
    })
    .button_disabled(!ready);
    if ready {
        b.on_press(|s: &mut State| s.install_update())
    } else {
        b.help(if s.updates.status.phase == Phase::Ready {
            "Finish recording, Ask and background jobs before installing."
        } else {
            "Sidenote is getting the update ready to install."
        })
    }
}
fn actions(s: &State, p: Palette, footer: bool) -> El {
    let mut actions = row().w_full().flex_wrap().gaps(10.0, 12.0).items_center();
    if matches!(
        s.updates.status.phase,
        Phase::Available
            | Phase::Downloading
            | Phase::Ready
            | Phase::Preparing
            | Phase::StoppingListener
    ) {
        if !matches!(
            s.updates.status.phase,
            Phase::Available | Phase::Downloading
        ) {
            actions = actions.child(install(s, p, footer));
        }
        if !s.updates.applying
            && !matches!(
                s.updates.status.phase,
                Phase::Preparing | Phase::StoppingListener
            )
        {
            actions = actions.child(
                crate::ui::widgets::action("Later", p)
                    .key(if footer {
                        "update-footer-later"
                    } else {
                        "update-later"
                    })
                    .on_press(|s: &mut State| s.updates.dismissed = true),
            );
        }
    }
    if !s.updates.status.notes.is_empty() {
        actions = actions.child(
            crate::ui::widgets::action(
                if s.updates.notes_open {
                    "Hide what changed"
                } else {
                    "What changed"
                },
                p,
            )
            .key(if footer {
                "update-footer-notes"
            } else {
                "update-notes"
            })
            .on_press(|s: &mut State| s.updates.notes_open = !s.updates.notes_open),
        );
    }
    actions
}
fn channel_label(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "Stable",
        Channel::Dev => "Dev",
        Channel::Latest => "Latest",
    }
}

pub fn section(s: &State, p: Palette) -> El {
    if cfg!(target_os = "linux") {
        return col().w_full().gap(12.0).key("updates-unavailable")
            .child(roles::SECTION.wrap("Updates and tray", p.ink))
            .child(roles::SMALL.wrap("Automatic updates and the tray indicator are unavailable on Linux. Close the window to quit Sidenote.", p.soft));
    }
    let status = &s.updates.status;
    let current_channel = s.updates.channel;
    let channel_help = match current_channel {
        Channel::Dev => {
            "Get prerelease builds. If you switch to Stable, this build stays installed until a newer stable release is available."
        }
        Channel::Stable if CHANNEL == Channel::Dev => {
            "This build stays installed until a newer stable release is available."
        }
        Channel::Stable => "Get stable releases.",
        Channel::Latest => "Get the newest build from main that passed its release checks.",
    };
    let checked = status
        .checked
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map_or_else(
            || "Not checked yet".into(),
            |d| format!("Last checked {}", d.format("%-d %b %Y at %H:%M")),
        );
    let mut application = super::hex::settings::panel(p).key("updates-section").child(
        super::hex::settings::setting_row(
            s,
            "update-version",
            &format!("Installed version · Sidenote {}", updates::VERSION),
            &checked,
            if status.phase == Phase::Manual {
                roles::SMALL.wrap("Updates are off for this manual install", p.soft)
            } else {
                crate::ui::widgets::button(
                    if status.phase == Phase::Checking {
                        "Checking…"
                    } else {
                        "Check now"
                    },
                    "update-check",
                    crate::ui::widgets::ButtonRole::Secondary,
                    p,
                )
                .disabled(busy(status))
                .on_press(|s: &mut State| s.check_updates())
            },
            false,
            p,
        ),
    );
    let channel_switch_disabled =
        status.phase == Phase::Manual || busy(status) || s.updates.applying;
    let channel_switches = [
        (Channel::Stable, "update-channel-stable"),
        (Channel::Dev, "update-channel-dev"),
        (Channel::Latest, "update-channel-latest"),
    ];
    let channel_control = row().items_center().gap(8.0).flex_wrap().children(
        channel_switches
            .into_iter()
            .filter(|(channel, _)| *channel != current_channel)
            .map(|(channel, key)| {
                super::widgets::button(
                    format!("Switch to {}", channel_label(channel)),
                    key,
                    super::widgets::ButtonRole::Secondary,
                    p,
                )
                .disabled(channel_switch_disabled)
                .on_press(move |s: &mut State| s.switch_update_channel(channel))
            }),
    );
    application = application
        .child(hairline(p))
        .child(super::hex::settings::setting_row(
            s,
            "update-channel",
            &format!("Get updates from · {}", channel_label(current_channel)),
            channel_help,
            channel_control,
            false,
            p,
        ));
    let title = match status.phase {
        Phase::Manual => "Updates are off for this manual install".into(),
        Phase::Preparing if s.updates.reconcile_pending => "Finishing the update".into(),
        Phase::Preparing => "Preparing update".into(),
        Phase::StoppingListener => "Stopping the listener".into(),
        Phase::Idle => "Sidenote is up to date.".into(),
        Phase::Checking => "Checking the update folder...".into(),
        Phase::Available | Phase::Downloading | Phase::Ready => {
            format!("Update available: {}", status.version)
        }
        Phase::Error => "Could not update Sidenote.".into(),
    };
    let mut card = col()
        .w_full()
        .padding(15.0)
        .gap(10.0)
        .child(roles::NAME.wrap(title, p.ink).role("status"));
    match status.phase {
        Phase::Available => {
            card =
                card.child(roles::SMALL.wrap("The download will start in the background.", p.soft));
        }
        Phase::Downloading => {
            let width =
                (s.app_w.get() - if s.narrow.get() { 140.0 } else { 430.0 }).clamp(180.0, 720.0);
            card = card
                .child(roles::CONTROL.wrap(format!("Downloading: {}%", status.progress), p.soft))
                .child(
                    el().w(width)
                        .max_w(width)
                        .h(5.0)
                        .radius(3.0)
                        .fill(p.line)
                        .child(
                            el().w(width * f32::from(status.progress) / 100.0)
                                .h(5.0)
                                .radius(3.0)
                                .fill(p.ink),
                        )
                        .role("progressbar")
                        .range_value(f64::from(status.progress), 0.0, 100.0)
                        .label(format!("Update download {} percent", status.progress)),
                );
        }
        Phase::Ready => {
            card = card.child(roles::SMALL.wrap("Ready to install. Restart now, or keep working and install next time you open Sidenote.", p.soft));
        }
        _ => {}
    }
    if let Some(error) = &status.error {
        card = card.child(roles::CONTROL.wrap(error, p.warn).role("alert"));
    }
    if status.phase == Phase::Error || status.error.is_some() {
        card = card.child(
            crate::ui::widgets::action("Try again", p)
                .key("update-retry")
                .on_press(|s: &mut State| {
                    if s.updates.status.phase == Phase::Ready {
                        s.install_update();
                    } else {
                        s.check_updates();
                    }
                }),
        );
    }
    card = card.child(actions(s, p, false));
    if s.updates.dismissed {
        card = card.child(roles::META.wrap(
            "You can keep working. The update will wait until the next restart.",
            p.soft,
        ));
    }
    if s.updates.notes_open && !status.notes.is_empty() {
        card = card
            .child(hairline(p))
            .child(roles::SMALL.bold().wrap("What changed", p.ink))
            .child(roles::SMALL.wrap(&status.notes, p.soft));
    }
    if !matches!(status.phase, Phase::Idle | Phase::Manual) {
        application = application.child(hairline(p)).child(card);
    }
    col().w_full().mt(18.0).gap(8.0)
        .child(roles::GROUP.wrap("Application",p.soft)).child(application)
        .child(roles::META.wrap("Sidenote checks every 6 hours and installs when recording, Ask and jobs are idle.",p.faint).mt(8.0))
}
pub fn source(s: &State, p: Palette) -> El {
    if cfg!(target_os = "linux") {
        return roles::SMALL.wrap("Linux update packaging is not available yet.", p.soft);
    }
    col().w_full()
        .child(roles::SMALL.wrap(if s.updates.scene {
                "E:\\Sidenote updates".into()
            } else {
                updates::source_label()
            }, p.ink))
        .child(roles::SMALL.wrap(if updates::feed().is_some() {
            "Anyone who can change files in this folder can change what gets installed. Use a folder only you can write to."
        } else {
            "Updates download from Sidenote's public GitHub Releases over HTTPS."
        }, p.soft))
}
pub fn footer(s: &State, p: Palette, wide: bool) -> Option<El> {
    let u = &s.updates;
    if u.dismissed || u.status.phase != Phase::Ready {
        return None;
    }
    if !wide {
        return Some(
            super::widgets::action("?", p)
                .key("update-footer")
                .label("Update ready. Open Updates settings")
                .on_press(|s: &mut State| s.open_updates()),
        );
    }
    Some(
        col()
            .w_full()
            .gap(3.0)
            .py(10.0)
            .child(roles::CONTROL.line(format!("Sidenote {} is ready", u.status.version), p.soft))
            .child(
                crate::ui::widgets::action("Restart to update", p)
                    .key("update-footer-install")
                    .on_press(|s: &mut State| s.install_update()),
            ),
    )
}
pub fn scene(s: &mut State, phase: Phase) {
    s.open_updates();
    hex::scene(s, false);
    s.updates.scene = true;
    s.updates.status = Status { version: if phase == Phase::Idle { String::new() } else { "0.2.0".into() },
        progress: if phase == Phase::Downloading { 40 } else { 100 },
        notes: "Updates now download in the background. Install when you are ready.\nYour meetings and notes stay on this device.".into(),
        checked: Some("2026-11-09T12:40:00+05:30".into()),
        error: (phase == Phase::Error).then(|| "Could not verify the update. Your current version is still running.".into()),
        phase, ..Default::default() };
    if s.updates.status.phase == Phase::Idle {
        s.updates.status.notes.clear();
    }
    s.updates.notes_open = s.updates.status.phase == Phase::Ready;
}
pub(super) fn settings_fixture(s: &mut State) {
    s.updates.scene = true;
    s.updates.status.phase = Phase::Idle;
}
pub fn settings_scene(s: &mut State) {
    s.open_updates();
}

pub fn recorder_gap_notice(s: &State, p: Palette) -> Option<El> {
    if listener_problem(s).is_some() {
        return None;
    }
    let gap = s.updates.recorder_gap.as_ref()?;
    let dismiss = super::widgets::action("Dismiss", p)
        .key("recorder-gap-dismiss")
        .on_press(|s: &mut State| {
            if let Some(gap) = s.updates.recorder_gap.clone()
                && heartbeat::dismiss(&gap).is_ok()
            {
                s.updates.dismissed_gap = Some(gap);
                s.updates.recorder_gap = None;
            }
        });
    Some(super::widgets::alert_strip(gap.notice(), dismiss, p).key("recorder-gap"))
}

pub fn listener_problem(s: &State) -> Option<&str> {
    if cfg!(windows) {
        s.updates.listener.error.as_deref()
    } else {
        None
    }
}

pub fn listener_popover(
    s: &State,
    trigger: El,
    error: &str,
    p: Palette,
    layer: &mut Vec<El>,
) -> El {
    use super::widgets;
    let trigger = trigger
        .child(widgets::dot(6.0, p.warn).key("listener-error-dot"))
        .label("Record a meeting. Recording is unavailable")
        .on_press(|s: &mut State| s.updates.listener_open = !s.updates.listener_open);
    let retry = widgets::retry_action(
        widgets::notice_action(
            if s.updates.listener_busy {
                "Trying again\u{2026}"
            } else {
                "Try again"
            },
            p,
        )
        .key("listener-retry")
        .help("You can also reopen Sidenote to retry.")
        .self_start(),
        s.updates.listener_busy || s.updates.applying,
        |s: &mut State| {
            let _ = s.retry_listener();
        },
    );
    rune_kit::Popover::new("listener-error-popover")
        .trigger(trigger)
        .label("Recording is unavailable")
        .open(s.updates.listener_open)
        .placement(rune_kit::Placement::BottomEnd)
        .on_close(|s: &mut State| s.updates.listener_open = false)
        .child(
            col()
                .w(270.0)
                .gap(8.0)
                .child(roles::NAME.wrap("Recording is unavailable", p.ink))
                .child(roles::SMALL.wrap(error, p.soft))
                .child(retry),
        )
        .build(&super::theme::kit(&p), layer)
}
pub fn listener_error_scene(s: &mut State) {
    hex::scene(s, false);
    s.updates.scene = true;
    s.updates.listener = updates::ListenerStatus { ready: false,
            preparing: false, error: Some("Sidenote could not prepare its meeting listener. Try again. If this continues, reinstall Sidenote for your Windows account.".into()) };
}

pub fn listener_preparing_scene(s: &mut State) {
    hex::scene(s, false);
    s.updates.scene = true;
    s.updates.listener = updates::ListenerStatus {
        ready: false,
        preparing: true,
        error: None,
    };
}

pub fn listener_start_blocked(s: &State) -> Option<&'static str> {
    let status = if s.updates.scene {
        s.updates.listener.clone()
    } else {
        updates::listener_status()
    };
    if status.preparing {
        Some("Checking the meeting listener")
    } else if !status.ready {
        Some("Recording is unavailable. Restart Sidenote to retry.")
    } else {
        None
    }
}

pub fn listener_ready(s: &State) -> bool {
    if s.updates.scene {
        s.updates.listener.ready
    } else {
        updates::listener_ready()
    }
}

#[cfg(test)]
mod listener_tests {
    use super::*;
    #[test]
    fn background_recovery_clears_the_error_and_later_death_restores_it() {
        let mut updates = Updates {
            listener: updates::ListenerStatus {
                ready: false,
                preparing: false,
                error: Some("The meeting listener did not start.".into()),
            },
            listener_probe_busy: true,
            ..Default::default()
        };
        updates.listener_probed(Some(updates::ListenerStatus {
            ready: true,
            preparing: false,
            error: None,
        }));
        assert!(updates.listener.ready);
        assert!(updates.listener.error.is_none());
        assert!(!updates.listener_probe_busy);
        updates.listener_probed(Some(updates::ListenerStatus {
            ready: false,
            preparing: false,
            error: Some("The meeting listener is not running. Try again.".into()),
        }));
        assert!(!updates.listener.ready);
        assert!(updates.listener.error.is_some());
    }
    #[test]
    fn stale_probe_results_do_not_override_retry_or_apply() {
        for applying in [false, true] {
            let mut updates = Updates {
                listener_busy: !applying,
                applying,
                listener: updates::ListenerStatus {
                    ready: false,
                    preparing: false,
                    error: Some("retry in progress".into()),
                },
                ..Default::default()
            };
            updates.listener_probed(Some(updates::ListenerStatus {
                ready: true,
                preparing: false,
                error: None,
            }));
            assert!(!updates.listener.ready);
            assert_eq!(updates.listener.error.as_deref(), Some("retry in progress"));
        }
    }
    #[test]
    fn install_action_waits_for_a_verified_download() {
        use sidenote::hex_engine::{HexError, sample_status};

        crate::test_paths::isolate();
        // Missing Hex opens setup and queues a summary load on Windows startup.
        let detect = hex::SKIP_INSTALL_DETECT.replace(true);
        let mut state = State::new();
        state.hex_apply(hex::Msg::ObservedStatus(sample_status(|| {
            Err(HexError::HexNotRunning)
        })));
        hex::SKIP_INSTALL_DETECT.set(detect);
        state.updates.status.phase = Phase::Available;
        let mut rt = rune::Runtime::new(
            state,
            |s, _| actions(s, super::super::theme::palette(false), false),
            (390.0, 180.0),
            rune::Env::default(),
        );
        for phase in [Phase::Available, Phase::Downloading, Phase::Ready] {
            let ready = phase == Phase::Ready;
            rt.update(|s, _| s.updates.status.phase = phase);
            rt.frame(0.0);
            assert_eq!(rt.find("update-install").is_some(), ready);
            assert!(rt.find("update-later").is_some());
        }
        assert!(rt.elements().iter().any(|e| {
            e.key.as_deref() == Some("update-install")
                && e.label.as_deref() == Some("Install update and restart Sidenote")
        }));
    }

    #[test]
    fn pending_reconcile_schedules_one_worker() {
        let mut updates = Updates {
            reconcile_pending: true,
            ..Default::default()
        };
        updates.status.phase = Phase::Preparing;
        assert!(!updates.reconcile_retry_due(LISTENER_INTERVAL - Duration::from_millis(1)));
        assert!(updates.reconcile_retry_due(LISTENER_INTERVAL));
        updates.listener_busy = true;
        assert!(!updates.reconcile_retry_due(LISTENER_INTERVAL));
    }

    #[test]
    fn pending_reconcile_does_not_start_another_apply() {
        let mut updates = Updates {
            reconcile_pending: true,
            ..Default::default()
        };
        updates.status.phase = Phase::Ready;
        assert!(!updates.can_apply());
        updates.reconcile_pending = false;
        assert!(updates.can_apply());
    }

    #[test]
    fn feed_checks_never_clear_a_listener_failure() {
        let mut updates = Updates {
            listener: updates::ListenerStatus {
                ready: false,
                preparing: false,
                error: Some("mismatched task".into()),
            },
            ..Default::default()
        };
        for phase in [
            Phase::Checking,
            Phase::Downloading,
            Phase::Idle,
            Phase::Ready,
            Phase::Error,
        ] {
            updates.feed_status(Status {
                phase,
                ..Default::default()
            });
            assert!(!updates.listener.ready);
            assert_eq!(updates.listener.error.as_deref(), Some("mismatched task"));
        }
    }
}
