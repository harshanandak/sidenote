//! Native window setup; the shared screens own the close decision.
#[cfg(windows)]
use super::DEFAULT_SIZE;
use super::State;
static WINDOW: std::sync::OnceLock<rune::AppHandle<State>> = std::sync::OnceLock::new();

thread_local! {
    // A close approved by either entry point can be followed by Rune's quit-time close hook.
    static CLOSE_ACCEPTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn configure(app: rune::App<State>) -> rune::App<State> {
    let close_handle = app.handle();
    let _ = WINDOW.set(close_handle.clone());
    let app = app.keep_running().on_close(rune_close);
    #[cfg(target_os = "linux")]
    let app = app.turn_scope(
        |state| state.turn_lock_budget = Some(sidenote::core::UiLockBudget::begin()),
        |state| state.turn_lock_budget = None,
    );
    // The OS places a new window at a cascade offset, which at 125% scaling on a 1080p screen
    // pushes the bottom (and the Ask bar) off screen. Fit the size to the work area and centre.
    app.on_window(move |win| {
        native_window(win, &close_handle);
        note_shown();
    })
    .on_ready(|_| note_shown())
}

fn note_shown() {
    if cfg!(not(windows)) || super::hex::window_shown() {
        super::startup::observe(super::startup::WINDOW_SHOWN, std::time::Instant::now());
    }
}

#[cfg(windows)]
fn native_window(win: &rune::WindowHandle, close_handle: &rune::AppHandle<State>) {
    let hwnd = win
        .with_raw_handle(|h| match h.as_raw() {
            rune::raw_window_handle::RawWindowHandle::Win32(w) => w.hwnd.get(),
            _ => 0,
        })
        .unwrap_or(0);
    if hwnd != 0 {
        if let Err(error) = protect_close(hwnd, close_handle) {
            sidenote::platform::show_error(&format!("Could not protect unsaved notes: {error}"));
            close_handle.quit();
            return;
        }
        super::hex::remember_window(hwnd);
        sidenote::platform::set_window_icon(hwnd);
        if super::benchmark::show_offscreen(win, hwnd) {
            return;
        }
        win.resize_instant(sidenote::platform::fit_to_work_area(hwnd, DEFAULT_SIZE));
    }
    win.center();
    if hwnd != 0 {
        meeting_icon::register(hwnd as _, close_handle);
    }
}

#[cfg(windows)]
fn protect_close(hwnd: isize, handle: &rune::AppHandle<State>) -> std::io::Result<()> {
    let handle = handle.clone();
    // The native guard consumes WM_CLOSE before Rune can deliver a second close event.
    sidenote::platform::guard_window_close(hwnd, move || {
        handle.update(|state, tx| {
            request_close(state, tx);
        });
    })
}

fn request_close(state: &mut State, tx: &mut rune::Tx) -> bool {
    if CLOSE_ACCEPTED.get() {
        return true;
    }
    let safe = state.live_close();
    if safe {
        CLOSE_ACCEPTED.set(true);
        tx.quit();
    }
    safe
}

fn rune_close(state: &mut State, tx: &mut rune::Tx) {
    // Rune hides a kept-alive window before invoking its close hook.
    if !request_close(state, tx)
        && let Some(window) = WINDOW.get()
    {
        window.open();
    }
}

#[cfg(not(windows))]
fn native_window(win: &rune::WindowHandle, _: &rune::AppHandle<State>) {
    win.center();
}
#[cfg(not(windows))]
pub(super) fn shown() -> bool {
    WINDOW.get().is_none_or(rune::AppHandle::is_visible)
}

#[cfg(all(windows, test))]
mod close_tests {
    use super::*;
    use crate::ui::live::{Live, Phase};
    use std::{
        fs,
        os::windows::process::CommandExt,
        path::{Path, PathBuf},
        process::Command,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, SendMessageW, WM_CLOSE,
    };

    const DRAFT: &str = "Workshop notes with an unsaved edit";

    #[test]
    fn accessible_close_reopens_with_an_unsaved_draft() {
        run_offscreen(
            "accessible_close_reopens_with_an_unsaved_draft",
            false,
            true,
        );
    }

    #[test]
    fn native_close_keeps_the_unsaved_draft_open() {
        run_offscreen("native_close_keeps_the_unsaved_draft_open", true, true);
    }

    #[test]
    fn accessible_close_accepts_a_saved_draft_without_reopening() {
        run_offscreen(
            "accessible_close_accepts_a_saved_draft_without_reopening",
            false,
            false,
        );
    }

    #[test]
    fn native_close_accepts_a_saved_draft_without_reopening() {
        run_offscreen(
            "native_close_accepts_a_saved_draft_without_reopening",
            true,
            false,
        );
    }

    #[test]
    fn accepted_close_requests_quit_and_saves_only_once() {
        CLOSE_ACCEPTED.set(false);
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        let mut state = draft_state(root.path().to_owned());
        let mut first = rune::Tx::default();
        let mut quit = rune::Tx::default();
        quit.quit();
        assert!(request_close(&mut state, &mut first));
        assert!(CLOSE_ACCEPTED.get());
        assert_eq!(format!("{first:?}"), format!("{quit:?}"));
        assert_eq!(saved_drafts(root.path()), [DRAFT]);

        state.live_notes = rune::TextEdit::new("A different unsaved draft");
        let mut second = rune::Tx::default();
        let untouched = format!("{second:?}");
        assert!(request_close(&mut state, &mut second));
        assert_eq!(format!("{second:?}"), untouched);
        assert_eq!(saved_drafts(root.path()), [DRAFT]);
        CLOSE_ACCEPTED.set(false);
    }

    fn draft_state(root: PathBuf) -> State {
        let mut state = State::new();
        state.store.root = root;
        state.live = Some(Live::scene(Phase::Recording, None));
        state.live_notes = rune::TextEdit::new(DRAFT);
        state
    }

    fn saved_drafts(root: &Path) -> Vec<String> {
        fs::read_dir(root.join("unsaved-notes"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect()
    }

    fn run_offscreen(test: &str, native: bool, refuse_save: bool) {
        const FLAG: &str = "SIDENOTE_CLOSE_ROUTE_TEST";
        if std::env::var(FLAG).as_deref() != Ok(test) {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    &format!("ui::window::close_tests::{test}"),
                    "--nocapture",
                ])
                .env(FLAG, test)
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        CLOSE_ACCEPTED.set(false);
        crate::test_paths::isolate();
        let root = tempfile::tempdir().unwrap();
        if refuse_save {
            fs::write(
                root.path().join("unsaved-notes"),
                "block the recovery directory",
            )
            .unwrap();
        }
        let backdrop = root.path().join("backdrop.png");
        fs::write(
            &backdrop,
            include_bytes!("../../tests/fixtures/annotations/synthetic.scrawl/frames/frame-01.png"),
        )
        .unwrap();
        let tree = root.path().join("close-tree.json");
        let state_root = root.path().to_owned();
        let mut spec = rune::AppSpec::new("Close test", (300.0, 150.0), State::new, |state, _| {
            rune::el().child(rune::text(
                if state.live.as_ref().is_some_and(|live| live.close_failed) {
                    state.live_notes.text.clone()
                } else {
                    String::new()
                },
            ))
        });
        spec.setup = Some(if native {
            |_, _| {
                let handle = WINDOW.get().unwrap();
                let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
                // A message-only window executes the actual subclass without a visible desktop.
                unsafe {
                    let hwnd = CreateWindowExW(
                        0,
                        class.as_ptr(),
                        std::ptr::null(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        HWND_MESSAGE,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    );
                    assert!(!hwnd.is_null());
                    protect_close(hwnd as isize, handle).unwrap();
                    SendMessageW(hwnd, WM_CLOSE, 0, 0);
                    assert_ne!(DestroyWindow(hwnd), 0);
                }
            }
        } else {
            |_, _| WINDOW.get().unwrap().update(|_, tx| tx.close_window())
        });
        let app = rune::App::new(spec).init(move || draft_state(state_root));
        let handle = app.handle();
        configure(app)
            .args([
                "sidenote".into(),
                "--desktop".into(),
                backdrop.to_string_lossy().into_owned(),
                "--tree".into(),
                tree.to_string_lossy().into_owned(),
                "--size".into(),
                "400x250".into(),
                "--scale".into(),
                "1".into(),
            ])
            .try_run()
            .unwrap();
        if refuse_save {
            assert!(handle.is_visible());
            assert!(fs::read_to_string(tree).unwrap().contains(DRAFT));
            assert!(!CLOSE_ACCEPTED.get());
        } else {
            assert!(CLOSE_ACCEPTED.get());
            assert_eq!(saved_drafts(root.path()), [DRAFT]);
            // A virtual desktop runs Close but leaves Tx::quit to its host.
            assert_eq!(handle.is_visible(), native);
            let frames = rune::tooling::headless_run().unwrap().window_frames();
            assert!(
                !frames
                    .windows(2)
                    .any(|pair| !pair[0].visible && pair[1].visible)
            );
        }
        CLOSE_ACCEPTED.set(false);
    }
}

#[cfg(windows)]
pub(super) mod meeting_icon {
    use super::State;
    use sidenote::meeting_icon::{WINDOW_NUB_ARG, beside, can_restore, open_window_message};
    use std::{cell::Cell, mem::size_of, ptr, time::Instant};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    };

    #[derive(Clone, Copy)]
    struct Placement {
        hwnd: HWND,
        original: WINDOWPLACEMENT,
        placed: [i32; 4],
    }

    thread_local! {
        static PLACEMENT: Cell<Option<Placement>> = const { Cell::new(None) };
        static OPEN_LIVE: Cell<Option<Instant>> = const { Cell::new(None) };
        static MAXIMIZE_ON_FOCUS: Cell<Option<WINDOWPLACEMENT>> = const { Cell::new(None) };
    }

    fn rect(hwnd: HWND) -> Option<[i32; 4]> {
        let mut r: RECT = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetWindowRect(hwnd, &mut r) } != 0;
        ok.then_some([r.left, r.top, r.right - r.left, r.bottom - r.top])
    }

    fn move_window(hwnd: HWND, bounds: [i32; 4]) -> bool {
        let [x, y, width, height] = bounds;
        let set = || unsafe {
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
            ) != 0
        };
        // Winit adjusts the first move if it crosses into a different DPI.
        set() && (rect(hwnd) == Some(bounds) || set())
    }

    unsafe fn open(hwnd: HWND, icon: HWND, app: &rune::AppHandle<State>) {
        unsafe {
            let mut class = [0; 64];
            let len = GetClassNameW(icon, class.as_mut_ptr(), class.len() as i32) as usize;
            let class = String::from_utf16_lossy(&class[..len]);
            if !class.starts_with("SidenoteListenerMeetingIcon-") {
                return;
            }
            let Some(icon_rect) = rect(icon) else { return };
            let monitor = MonitorFromWindow(icon, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return;
            }
            let mut original: WINDOWPLACEMENT = std::mem::zeroed();
            original.length = size_of::<WINDOWPLACEMENT>() as u32;
            if GetWindowPlacement(hwnd, &mut original) == 0 {
                return;
            }
            if let Some(saved) = PLACEMENT.take() {
                original = saved.original;
            } else if let Some(saved) = MAXIMIZE_ON_FOCUS.take() {
                original = saved;
            }
            ShowWindow(hwnd, SW_RESTORE);
            let work = info.rcWork;
            let height = rect(hwnd).map_or(work.bottom - work.top, |r| r[3]);
            let dpi = GetDpiForWindow(icon).max(96) as i32;
            let target = beside(
                [work.left, work.top, work.right, work.bottom],
                icon_rect,
                [400 * dpi / 96, height],
                8 * dpi / 96,
            );
            if move_window(hwnd, target) {
                PLACEMENT.set(rect(hwnd).map(|placed| Placement {
                    hwnd,
                    original,
                    placed,
                }));
            }
            SetForegroundWindow(hwnd);
            OPEN_LIVE.set(Some(Instant::now()));
            app.update(|_, _| {});
        }
    }

    pub(super) fn register(hwnd: HWND, app: &rune::AppHandle<State>) {
        let data = Box::into_raw(Box::new(app.clone()));
        unsafe {
            if SetWindowSubclass(hwnd, Some(message), 1, data as usize) == 0 {
                drop(Box::from_raw(data));
            } else if let Some(icon) = std::env::args()
                .find_map(|arg| arg.strip_prefix(WINDOW_NUB_ARG)?.parse::<usize>().ok())
            {
                open(hwnd, icon as HWND, app);
            }
        }
    }

    unsafe extern "system" fn message(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        unsafe {
            let data = data as *mut rune::AppHandle<State>;
            if msg >= 0xc000 && msg == open_window_message() {
                open(hwnd, wp as HWND, &*data);
                return 0;
            }
            if msg == WM_WINDOWPOSCHANGED {
                let saved = PLACEMENT.get();
                PLACEMENT.set(saved.filter(|p| can_restore(p.placed, rect(hwnd))));
            }
            if msg == WM_ACTIVATE
                && wp as u16 != WA_INACTIVE as u16
                && let Some(original) = MAXIMIZE_ON_FOCUS.take()
            {
                SetWindowPlacement(hwnd, &original);
            }
            if msg == WM_NCDESTROY {
                PLACEMENT.set(None);
                OPEN_LIVE.set(None);
                MAXIMIZE_ON_FOCUS.set(None);
                RemoveWindowSubclass(hwnd, Some(message), id);
                drop(Box::from_raw(data));
            }
            DefSubclassProc(hwnd, msg, wp, lp)
        }
    }

    fn restore(saved: Placement) {
        let mut original = saved.original;
        let maximized = original.showCmd == SW_SHOWMAXIMIZED as u32;
        // Maximizing activates the window. Wait until it already has focus.
        let defer = maximized && unsafe { GetForegroundWindow() } != saved.hwnd;
        MAXIMIZE_ON_FOCUS.set(defer.then_some(original));
        if original.showCmd == SW_SHOWMINIMIZED as u32 {
            // It was minimized before the icon opened it: put it back minimized.
            original.showCmd = SW_SHOWMINNOACTIVE as u32;
        } else if !maximized || defer {
            original.showCmd = SW_SHOWNOACTIVATE as u32;
        }
        unsafe { SetWindowPlacement(saved.hwnd, &original) };
    }

    pub(in crate::ui) fn poll(s: &mut State) -> bool {
        let pending = OPEN_LIVE.get();
        if pending.is_some() && s.hex_recording() {
            OPEN_LIVE.set(None);
            s.open_live();
            return true;
        }
        if pending.is_some_and(|at| at.elapsed().as_secs() < 10) {
            return false;
        }
        OPEN_LIVE.set(None);
        let finished = ["idle", "stopped", "complete", "finished"];
        if let Some(Ok(state)) = &s.hex.status
            && finished.contains(&state.meeting_state.as_str())
            && let Some(saved) = PLACEMENT.take()
        {
            restore(saved);
        }
        false
    }
}
