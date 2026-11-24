use super::{Control, Frame, RETIRE, callback_boundary};
use crate::core::notes::SummarySkip;
use crate::{
    core::{self, store_io},
    meeting_icon::{Notice, card_text, dismiss_notice, draft_seen, nub},
};
use std::{
    cell::{Cell, RefCell},
    io,
    mem::size_of,
    path::PathBuf,
    ptr,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
    thread::{Builder, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
            SetThreadDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            ReleaseCapture, SetCapture, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT,
            VK_PRIOR, VK_RETURN, VK_RIGHT, VK_UP,
        },
        WindowsAndMessaging::*,
    },
};

const WIDTH: f32 = 104.0;
const HEIGHT: f32 = 44.0;
const CAPTURE_TIMER: usize = 1;
const CAPTURE_POLL_MS: u32 = 100;
const TOGGLE_CAPTURE: usize = 1;
const CAPTURE_SAVE_FAILED: &str = "FI_CAPTURE_SAVE_FAILED";

fn capture_affinity(visible: bool) -> u32 {
    if visible {
        WDA_NONE
    } else {
        WDA_EXCLUDEFROMCAPTURE
    }
}

fn apply_capture_affinity(hwnd: HWND, visible: bool) -> io::Result<()> {
    unsafe {
        if SetWindowDisplayAffinity(hwnd, capture_affinity(visible)) != 0 {
            return Ok(());
        }
        if visible {
            crate::listener::log("FI_CAPTURE_ENABLE_FAILED");
            return Ok(());
        }
        let error = io::Error::last_os_error();
        ShowWindow(hwnd, SW_HIDE);
        Err(error)
    }
}
fn nub_font(height: i32) -> io::Result<Rc<Font>> {
    unsafe {
        let mut spec: LOGFONTW = std::mem::zeroed();
        spec.lfHeight = -height;
        spec.lfCharSet = DEFAULT_CHARSET;
        spec.lfQuality = ANTIALIASED_QUALITY;
        let face = wide("Consolas");
        spec.lfFaceName[..face.len()].copy_from_slice(&face);
        let font = CreateFontIndirectW(&raw const spec);
        if font.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Rc::new(Font(font)))
    }
}

fn elapsed_text(elapsed: u64) -> Vec<u16> {
    wide(&format!("{:02}:{:02}", elapsed / 60, elapsed % 60))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[derive(Clone, Copy)]
struct Layout {
    scale: f32,
    work: RECT,
    fraction: f32,
}
#[derive(Default, Clone)]
struct Card {
    text: String,
    id: String,
    raw: Option<String>,
    revision: u64,
    refreshed: Option<Instant>,
    updates: usize,
    minimum_live_gap_ms: Option<u128>,
}
struct Font(HFONT);
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            if DeleteObject(self.0) == 0 {
                crate::listener::log("FI_FONT_RELEASE");
            }
        }
    }
}
struct SelectedFont {
    dc: HDC,
    prior: HGDIOBJ,
    _font: Rc<Font>,
}
impl SelectedFont {
    unsafe fn new(dc: HDC, font: Rc<Font>) -> Self {
        unsafe {
            Self {
                dc,
                prior: SelectObject(dc, font.0),
                _font: font,
            }
        }
    }
}
impl Drop for SelectedFont {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.prior);
        }
    }
}
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    prior: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
}
impl Surface {
    fn new(width: i32, height: i32) -> io::Result<Self> {
        unsafe {
            let dc = CreateCompatibleDC(ptr::null_mut());
            if dc.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                dc,
                bitmap,
                prior: SelectObject(dc, bitmap),
                bits: bits.cast(),
                width,
                height,
            })
        }
    }
    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.width * self.height) as usize) }
    }
    fn save(&mut self, name: &str, alpha: bool) {
        let width = self.width;
        let height = self.height;
        let rgba: Vec<u8> = self
            .pixels()
            .iter()
            .flat_map(|p| {
                let a = if alpha { p >> 24 } else { 255 };
                [
                    ((p >> 16 & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                    ((p >> 8 & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                    ((p & 255) * 255).checked_div(a).unwrap_or(0) as u8,
                    a as u8,
                ]
            })
            .collect();
        let _ = image::save_buffer(
            core::state_dir().join("local").join(name),
            &rgba,
            width as u32,
            height as u32,
            image::ColorType::Rgba8,
        );
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.prior);
            if DeleteObject(self.bitmap) == 0 {
                crate::listener::log("FI_BITMAP_RELEASE");
            }
            if DeleteDC(self.dc) == 0 {
                crate::listener::log("FI_DC_RELEASE");
            }
        }
    }
}
struct Paint {
    hwnd: HWND,
    ps: PAINTSTRUCT,
}
impl Paint {
    unsafe fn new(hwnd: HWND) -> (Self, HDC) {
        unsafe {
            let mut ps = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            (Self { hwnd, ps }, dc)
        }
    }
}
impl Drop for Paint {
    fn drop(&mut self) {
        unsafe {
            EndPaint(self.hwnd, &self.ps);
        }
    }
}

// Cells hold copyable geometry/handles. RefCell guards cover only pure Rust state updates.
struct View {
    icon: Cell<HWND>,
    card: Cell<HWND>,
    config_root: PathBuf,
    capture_visible: Cell<bool>,
    capture_save: RefCell<Option<JoinHandle<()>>>,
    font: RefCell<Option<Rc<Font>>>,
    timer_font: RefCell<Option<(i32, Rc<Font>)>>,
    surface: RefCell<Option<Surface>>,
    region_geometry: Cell<Option<(i32, i32, i32, f32)>>,
    layout: Cell<Layout>,
    frame: RefCell<Frame>,
    motion: Cell<nub::Motion>,
    look: Cell<Option<nub::Look>>,
    timer_text: RefCell<Option<(u64, Vec<u16>)>>,
    content: RefCell<Card>,
    scroll: Cell<i32>,
    max_scroll: Cell<i32>,
    wheel: Cell<i32>,
    drag: Cell<Option<(i32, f32)>>,
    moved: Cell<bool>,
    reopen: Cell<bool>,
    replay: bool,
    control: Arc<Control>,
}
pub(super) struct NativeWindow {
    view: Box<View>,
    icon_class: Vec<u16>,
    card_class: Vec<u16>,
}
impl NativeWindow {
    pub(super) fn new(replay: bool, root: PathBuf, control: Arc<Control>) -> io::Result<Self> {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(ptr::null());
            let icon_class = wide(&control.icon_class);
            let card_class = wide(&control.card_class);
            control.startup.enter("position_read");
            let fraction =
                store_io::read::<Option<f32>>(&root.join("local/floating-icon-position.json"))
                    .unwrap_or_default()
                    .unwrap_or(0.6);
            let window = Self {
                view: Box::new(View {
                    icon: Cell::new(ptr::null_mut()),
                    card: Cell::new(ptr::null_mut()),
                    config_root: root,
                    capture_visible: Cell::new(false),
                    capture_save: RefCell::default(),
                    font: RefCell::new(None),
                    timer_font: RefCell::new(None),
                    surface: RefCell::default(),
                    region_geometry: Cell::default(),
                    layout: Cell::new(Layout {
                        scale: 1.0,
                        work: std::mem::zeroed(),
                        fraction: if fraction.is_finite() {
                            fraction.clamp(0.0, 1.0)
                        } else {
                            0.6
                        },
                    }),
                    frame: RefCell::new(Frame::default()),
                    motion: Cell::default(),
                    look: Cell::default(),
                    timer_text: RefCell::default(),
                    content: RefCell::new(Card::default()),
                    scroll: Cell::new(0),
                    max_scroll: Cell::new(0),
                    wheel: Cell::new(0),
                    drag: Cell::new(None),
                    moved: Cell::new(false),
                    reopen: Cell::new(false),
                    replay,
                    control,
                }),
                icon_class,
                card_class,
            };
            window.view.control.startup.enter("class_registration");
            for (name, callback) in [
                (
                    &window.icon_class,
                    Some(icon_proc as unsafe extern "system" fn(_, _, _, _) -> _),
                ),
                (
                    &window.card_class,
                    Some(card_proc as unsafe extern "system" fn(_, _, _, _) -> _),
                ),
            ] {
                let wc = WNDCLASSW {
                    lpfnWndProc: callback,
                    hInstance: instance,
                    lpszClassName: name.as_ptr(),
                    hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                    ..std::mem::zeroed()
                };
                if RegisterClassW(&wc) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            window.view.control.startup.enter("window_creation");
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
                window.icon_class.as_ptr(),
                wide("Sidenote meeting icon").as_ptr(),
                WS_POPUP,
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                (&*window.view as *const View).cast(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            window.view.icon.set(hwnd);
            window.view.control.publish(hwnd, false);
            window.view.control.startup.enter("capture_exclusion");
            let visible = core::Config::meeting_icon_capture_allowed(&window.view.config_root);
            apply_capture_affinity(hwnd, visible)?;
            window.view.capture_visible.set(visible);
            // A window timer also runs inside the card's native move/resize loop.
            if SetTimer(hwnd, CAPTURE_TIMER, CAPTURE_POLL_MS, None) == 0 {
                return Err(io::Error::last_os_error());
            }
            window.view.control.startup.enter("geometry");
            window.view.geometry();
            Ok(window)
        }
    }
    pub(super) fn update(&self, frame: Frame) {
        let prior = self.view.frame.replace(frame.clone());
        let changed =
            icon_changed(&prior, &frame) || unsafe { IsWindowVisible(self.view.icon.get()) == 0 };
        let card_changed = prior.id != frame.id
            || prior.recording != frame.recording
            || prior.raw != frame.raw
            || prior.notice != frame.notice
            || prior.skipped != frame.skipped;
        unsafe {
            if !self.view.refresh_capture() {
                return;
            }
            let geometry = self.view.geometry();
            if (changed || geometry) && self.view.paint_icon().is_err() {
                crate::listener::log("FI_PAINT_FAILED");
            }
            if changed {
                self.view.position_card();
            }
            if !self.view.control.stop.load(Ordering::Acquire)
                && IsWindowVisible(self.view.icon.get()) == 0
            {
                ShowWindow(self.view.icon.get(), SW_SHOWNOACTIVATE);
            }
            if !self.view.card.get().is_null() && IsWindowVisible(self.view.card.get()) != 0 {
                self.view.refresh_card(card_changed);
            }
        }
    }
    pub(super) fn pump(&self) -> bool {
        unsafe {
            let mut timer = std::mem::zeroed();
            // Service capture even if the rest of the queue is busy.
            if PeekMessageW(
                &mut timer,
                self.view.icon.get(),
                WM_TIMER,
                WM_TIMER,
                PM_REMOVE,
            ) != 0
            {
                if timer.message == WM_QUIT {
                    return false;
                }
                DispatchMessageW(&timer);
            }
        }
        let view = &self.view;
        let now = Instant::now();
        let interval = view.animation_interval();
        if interval != 0
            && view.motion.get().due(now, interval)
            && unsafe { IsWindowVisible(view.icon.get()) } != 0
            && view.advance(now, view.replay_level())
        {
            let _ = unsafe { view.present_icon() };
        }
        unsafe {
            let mut message = std::mem::zeroed();
            // Return to the stop/channel checks even when native messages keep arriving.
            for _ in 0..64 {
                if PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) == 0 {
                    break;
                }
                if message.message == WM_QUIT {
                    return false;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            let wait = match view.animation_interval() {
                0 => 50,
                ms => ms,
            };
            MsgWaitForMultipleObjectsEx(0, ptr::null(), wait, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
        true
    }
}
impl Drop for NativeWindow {
    fn drop(&mut self) {
        unsafe {
            self.view.close_windows();
            let font = self.view.font.replace(None);
            drop(font);
            let instance = GetModuleHandleW(ptr::null());
            UnregisterClassW(self.card_class.as_ptr(), instance);
            UnregisterClassW(self.icon_class.as_ptr(), instance);
        }
    }
}

#[derive(Clone, Copy)]
enum Scroll {
    Lines(i32),
    Page(i32),
    Home,
    End,
}
fn icon_changed(prior: &Frame, frame: &Frame) -> bool {
    prior.id != frame.id
        || prior.recording != frame.recording
        || prior.pill != frame.pill
        || prior.notice != frame.notice
        || prior.skipped != frame.skipped
        || (frame.recording && prior.elapsed != frame.elapsed)
}

impl View {
    fn toggle_capture(&self) {
        let root = self.config_root.clone();
        let worker = Builder::new()
            .name("nub-capture-save".into())
            .spawn(move || {
                if core::Config::toggle_meeting_icon_capture(&root).is_err() {
                    crate::listener::log(CAPTURE_SAVE_FAILED);
                }
            });
        match worker {
            Ok(worker) => {
                self.capture_save.replace(Some(worker));
            }
            Err(_) => crate::listener::log("FI_CAPTURE_WORKER_FAILED"),
        }
    }

    unsafe fn capture_menu(&self, hwnd: HWND, position: LPARAM) {
        if self
            .capture_save
            .borrow()
            .as_ref()
            .is_some_and(|w| !w.is_finished())
        {
            return;
        }
        unsafe {
            let (x, y) = if position == -1 {
                let mut rect = std::mem::zeroed();
                if GetWindowRect(hwnd, &mut rect) == 0 {
                    return;
                }
                (rect.left, rect.bottom)
            } else {
                (position as i16 as i32, (position >> 16) as i16 as i32)
            };
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            let checked = if self.capture_visible.get() {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
            let added = AppendMenuW(
                menu,
                MF_STRING | checked,
                TOGGLE_CAPTURE,
                wide("Show in screenshots and recordings").as_ptr(),
            );
            let command = if added != 0 {
                // Keep the meeting app in the foreground.
                TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                    x,
                    y,
                    0,
                    hwnd,
                    ptr::null(),
                )
            } else {
                0
            };
            DestroyMenu(menu);
            if command as usize == TOGGLE_CAPTURE && !self.control.stop.load(Ordering::Acquire) {
                self.toggle_capture();
            }
        }
    }

    fn refresh_capture(&self) -> bool {
        let (enabled, visible) = core::Config::meeting_icon_settings(&self.config_root);
        unsafe {
            if !enabled
                && (IsWindowVisible(self.icon.get()) != 0 || IsWindowVisible(self.card.get()) != 0)
            {
                self.hide_card();
                ShowWindow(self.icon.get(), SW_HIDE);
            }
            if self.capture_visible.replace(visible) == visible {
                return enabled && !self.icon.get().is_null();
            }
            let card = self.card.get();
            if !card.is_null() && apply_capture_affinity(card, visible).is_err() {
                self.destroy(card);
            }
            if apply_capture_affinity(self.icon.get(), visible).is_err() {
                self.close_windows();
            }
        }
        enabled && !self.icon.get().is_null()
    }

    unsafe fn open_meeting_window(icon: HWND) {
        use crate::meeting_icon::{WINDOW_NUB_ARG, open_window_message};
        unsafe {
            let main = FindWindowW(ptr::null(), wide("Sidenote").as_ptr());
            if !main.is_null() {
                let mut process = 0;
                GetWindowThreadProcessId(main, &mut process);
                AllowSetForegroundWindow(process);
                PostMessageW(main, open_window_message(), icon as usize, 0);
            } else if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe)
                    .arg(format!("{WINDOW_NUB_ARG}{}", icon as usize))
                    .spawn();
            }
        }
    }

    fn advance(&self, now: Instant, level: Option<f32>) -> bool {
        let mut motion = self.motion.get();
        let look = motion.advance(now, self.frame.borrow().recording, level);
        self.motion.set(motion);
        self.look.replace(look) != look
    }

    fn animation_interval(&self) -> u32 {
        match self.look.get() {
            Some(look) if look.level > 0.0 || !self.frame.borrow().recording => 16,
            Some(_) => 50,
            None => 0,
        }
    }

    fn timer_font(&self) -> io::Result<Rc<Font>> {
        let height = self.px(13.0);
        if let Some((cached_height, font)) = self.timer_font.borrow().as_ref()
            && *cached_height == height
        {
            return Ok(Rc::clone(font));
        }
        let font = nub_font(height)?;
        self.timer_font.replace(Some((height, Rc::clone(&font))));
        Ok(font)
    }
    fn height(&self) -> f32 {
        if self.frame.borrow().recording || self.look.get().is_some() {
            nub::HEIGHT
        } else {
            HEIGHT
        }
    }
    fn width(&self) -> f32 {
        let frame = self.frame.borrow();
        if frame.recording || self.look.get().is_some() {
            nub::WIDTH
        } else if frame.notice.is_some() {
            300.0
        } else {
            WIDTH
        }
    }
    fn px(&self, value: f32) -> i32 {
        (value * self.layout.get().scale).round() as i32
    }
    fn travel(&self) -> i32 {
        let g = self.layout.get();
        (g.work.bottom - g.work.top - self.px(self.height() + 16.0)).max(0)
    }
    fn point(&self) -> POINT {
        let g = self.layout.get();
        POINT {
            x: g.work.right - self.px(self.width() + 8.0),
            y: g.work.top + self.px(8.0) + (g.fraction * self.travel() as f32).round() as i32,
        }
    }
    fn set_fraction(&self, fraction: f32) {
        let mut g = self.layout.get();
        g.fraction = fraction.clamp(0.0, 1.0);
        self.layout.set(g);
    }
    unsafe fn geometry(&self) -> bool {
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            let (mut x, mut y) = (96, 96);
            if GetMonitorInfoW(monitor, &mut info) == 0
                || GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) != 0
            {
                return false;
            }
            let old = self.layout.get();
            let r = info.rcWork;
            let scale = x as f32 / 96.0;
            if old.scale == scale
                && (old.work.left, old.work.top, old.work.right, old.work.bottom)
                    == (r.left, r.top, r.right, r.bottom)
            {
                return false;
            }
            let font = CreateFontW(
                -(13.0 * scale).round() as i32,
                0,
                0,
                0,
                500,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            );
            if font.is_null() {
                crate::listener::log("FI_FONT_FAILED");
                return false;
            }
            let prior = self.font.replace(Some(Rc::new(Font(font))));
            drop(prior);
            self.layout.set(Layout {
                scale,
                work: r,
                fraction: old.fraction,
            });
            let p = self.point();
            SetWindowPos(
                self.icon.get(),
                HWND_TOPMOST,
                p.x,
                p.y,
                self.px(self.width()),
                self.px(self.height()),
                SWP_NOACTIVATE,
            );
            self.position_card();
            true
        }
    }
    unsafe fn position_card(&self) {
        unsafe {
            let hwnd = self.card.get();
            if hwnd.is_null() {
                return;
            }
            let g = self.layout.get();
            let p = self.point();
            let width = self.px(360.0).min(g.work.right - g.work.left);
            let height = self.px(300.0).min(g.work.bottom - g.work.top);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                (p.x - width - self.px(8.0)).max(g.work.left),
                p.y.min(g.work.bottom - height),
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
    }
    unsafe fn refresh_card(&self, force: bool) {
        unsafe {
            let frame = self.frame.borrow().clone();
            let card = self.content.borrow().clone();
            let now = Instant::now();
            let live = frame.raw.is_none() && frame.transcript_revision != card.revision;
            if !force
                && (!live
                    || card
                        .refreshed
                        .is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)))
            {
                return;
            }
            let text = if let Some(notice) = frame.notice {
                notice.text()
            } else if frame.skipped {
                "Skipped. Short recordings are not processed.".into()
            } else {
                card_text(&frame.id, frame.recording, frame.raw.as_deref())
            };
            let mut next = Card {
                text,
                id: frame.id,
                raw: frame.raw,
                revision: frame.transcript_revision,
                refreshed: Some(now),
                updates: card.updates + 1,
                minimum_live_gap_ms: card.minimum_live_gap_ms,
            };
            if !force && let Some(at) = card.refreshed {
                let gap = now.duration_since(at).as_millis();
                next.minimum_live_gap_ms =
                    Some(next.minimum_live_gap_ms.map_or(gap, |old| old.min(gap)));
            }
            self.content.replace(next);
            InvalidateRect(self.card.get(), ptr::null(), 0);
            if self.replay {
                self.render_card();
            }
        }
    }
    unsafe fn prepare_card(&self) -> HWND {
        unsafe {
            if self.control.stop.load(Ordering::Acquire) {
                return ptr::null_mut();
            }
            if !self.card.get().is_null() {
                self.refresh_capture();
                return self.card.get();
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                wide(&self.control.card_class).as_ptr(),
                wide("Sidenote meeting").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                360,
                300,
                ptr::null_mut(),
                ptr::null_mut(),
                GetModuleHandleW(ptr::null()),
                (self as *const View).cast(),
            );
            if hwnd.is_null() {
                return hwnd;
            }
            self.refresh_capture();
            if self.control.stop.load(Ordering::Acquire)
                || apply_capture_affinity(hwnd, self.capture_visible.get()).is_err()
            {
                self.destroy(hwnd);
                return ptr::null_mut();
            }
            self.card.set(hwnd);
            self.control.publish(hwnd, true);
            hwnd
        }
    }
    unsafe fn open_card(&self) {
        unsafe {
            let hwnd = self.prepare_card();
            if hwnd.is_null() {
                return;
            }
            self.scroll.set(0);
            self.max_scroll.set(0);
            self.position_card();
            self.refresh_card(true);
            // User clicks may activate this keyboard-usable card. Automatic show/refresh/resize never does.
            if !self.control.stop.load(Ordering::Acquire) {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }
    unsafe fn hide_card(&self) {
        unsafe {
            self.content.replace(Card::default());
            self.scroll.set(0);
            self.max_scroll.set(0);
            let hwnd = self.card.get();
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
    unsafe fn acknowledge(&self) {
        unsafe {
            let card = self.content.borrow().clone();
            if let Some(raw) = card.raw {
                draft_seen(&card.id, &raw);
            }
            self.hide_card();
        }
    }
    unsafe fn destroy(&self, hwnd: HWND) {
        unsafe {
            if hwnd.is_null() {
                return;
            }
            self.clear_handle(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if DestroyWindow(hwnd) == 0 && IsWindow(hwnd) != 0 {
                crate::listener::log("FI_DESTROY_FAILED");
            }
        }
    }
    fn clear_handle(&self, hwnd: HWND) {
        if self.card.get() == hwnd {
            self.card.set(ptr::null_mut());
            self.content.replace(Card::default());
        }
        if self.icon.get() == hwnd {
            self.icon.set(ptr::null_mut());
            self.control.stop.store(true, Ordering::Release);
        }
        self.control.clear(hwnd);
    }
    unsafe fn close_windows(&self) {
        unsafe {
            self.destroy(self.card.get());
            self.destroy(self.icon.get());
        }
    }
    unsafe fn scroll_by(&self, action: Scroll) {
        unsafe {
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(self.card.get(), &mut r);
            let page = (r.bottom - self.px(64.0)).max(1);
            let current = self.scroll.get();
            let next = match action {
                Scroll::Lines(n) => current.saturating_add(n.saturating_mul(self.px(20.0))),
                Scroll::Page(n) => current.saturating_add(n.saturating_mul(page)),
                Scroll::Home => 0,
                Scroll::End => self.max_scroll.get(),
            };
            self.scroll.set(next.clamp(0, self.max_scroll.get()));
            InvalidateRect(self.card.get(), ptr::null(), 0);
        }
    }
    unsafe fn draw_card(&self, dc: HDC, bounds: RECT, card: &Card) {
        unsafe {
            let text = wide(&card.text);
            let footer = wide(if card.raw.is_some() {
                "Enter: mark seen    Esc: close"
            } else {
                "Esc: close"
            });
            FillRect(dc, &bounds, GetStockObject(WHITE_BRUSH) as HBRUSH);
            let Some(font) = self.font.borrow().clone() else {
                return;
            };
            let _font = SelectedFont::new(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00261f19);
            let margin = self.px(12.0);
            let bottom = (bounds.bottom - self.px(40.0)).max(margin);
            let mut measured = RECT {
                left: margin,
                top: margin,
                right: (bounds.right - margin).max(margin + 1),
                bottom: margin,
            };
            let flags = DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL;
            DrawTextW(dc, text.as_ptr(), -1, &mut measured, flags | DT_CALCRECT);
            let max = (measured.bottom - measured.top - (bottom - margin)).max(0);
            self.max_scroll.set(max);
            let scroll = self.scroll.get().clamp(0, max);
            self.scroll.set(scroll);
            let saved = SaveDC(dc);
            IntersectClipRect(dc, margin, margin, bounds.right - margin, bottom);
            let mut r = RECT {
                left: margin,
                top: margin - scroll,
                right: bounds.right - margin,
                bottom: measured.bottom - scroll,
            };
            DrawTextW(dc, text.as_ptr(), -1, &mut r, flags);
            if saved != 0 {
                RestoreDC(dc, saved);
            }
            let mut footer_rect = RECT {
                left: margin,
                top: bottom + self.px(8.0),
                right: bounds.right - margin,
                bottom: bounds.bottom,
            };
            DrawTextW(
                dc,
                footer.as_ptr(),
                -1,
                &mut footer_rect,
                DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }
    unsafe fn paint_card(&self, hwnd: HWND) {
        unsafe {
            let card = self.content.borrow().clone();
            let (_paint, dc) = Paint::new(hwnd);
            let mut r = std::mem::zeroed();
            GetClientRect(hwnd, &mut r);
            self.draw_card(dc, r, &card);
        }
    }
    unsafe fn render_card(&self) {
        unsafe {
            let card = self.content.borrow().clone();
            let mut r: RECT = std::mem::zeroed();
            GetClientRect(self.card.get(), &mut r);
            if let Ok(mut surface) = Surface::new(r.right.max(1), r.bottom.max(1)) {
                self.draw_card(surface.dc, r, &card);
                surface.save("floating-icon-card.png", false);
            }
        }
    }
    fn draw_icon(&self) -> io::Result<(Surface, i32)> {
        unsafe {
            let frame = self.frame.borrow();
            let look = self.look.get();
            let live = look.is_some();
            let pill = live || frame.skipped || frame.notice.is_some();
            let (width, height) = (self.px(self.width()), self.px(self.height()));
            let left = if pill { 0 } else { width - height };
            let mut surface = match self.surface.take() {
                Some(surface) if (surface.width, surface.height) == (width, height) => surface,
                _ => Surface::new(width, height)?,
            };
            let mut timer = self.timer_text.borrow_mut();
            let other;
            let label = if live {
                if timer
                    .as_ref()
                    .is_none_or(|(elapsed, _)| *elapsed != frame.elapsed)
                {
                    *timer = Some((frame.elapsed, elapsed_text(frame.elapsed)));
                }
                &timer.as_ref().unwrap().1
            } else {
                other = wide(&frame.notice.map_or_else(|| "Skipped".into(), Notice::text));
                &other
            };
            let font = if live {
                self.timer_font()?
            } else {
                self.font
                    .borrow()
                    .clone()
                    .ok_or_else(|| io::Error::other("Icon font unavailable"))?
            };
            surface
                .pixels()
                .fill(if live { nub::FILL } else { 0x001f2630 });
            {
                let _font = SelectedFont::new(surface.dc, font);
                SetBkMode(surface.dc, TRANSPARENT as i32);
                SetTextColor(surface.dc, if live { 0x00aaa6a2 } else { 0x00faf3ea });
                if pill {
                    let pad = self.px(if live { nub::PAD } else { 8.0 });
                    let mut r = RECT {
                        left: pad,
                        top: 0,
                        right: width - pad,
                        bottom: height,
                    };
                    if let Some(look) = look {
                        let room = 92.0 - label.len().saturating_sub(6) as f32 * 8.0;
                        nub::draw_field(
                            surface.pixels(),
                            width as usize,
                            self.layout.get().scale,
                            room,
                            look,
                        );
                    }
                    DrawTextW(
                        surface.dc,
                        label.as_ptr(),
                        -1,
                        &mut r,
                        DT_SINGLELINE
                            | DT_VCENTER
                            | DT_NOPREFIX
                            | if live { DT_RIGHT } else { DT_CENTER },
                    );
                }
                if !pill {
                    let brush = CreateSolidBrush(0x00b5c9ed);
                    let prior = SelectObject(surface.dc, brush);
                    let pen = SelectObject(surface.dc, GetStockObject(NULL_PEN));
                    let c = left + height / 2;
                    let r = self.px(5.0);
                    Ellipse(surface.dc, c - r, height / 2 - r, c + r, height / 2 + r);
                    SelectObject(surface.dc, pen);
                    SelectObject(surface.dc, prior);
                    DeleteObject(brush);
                }
            }
            let radius = width.min(height) as f32 / 2.0;
            let border = radius - self.layout.get().scale;
            let opacity = look.map_or(1.0, |look| look.opacity);
            for y in 0..height {
                for x in 0..width {
                    let cx = (x as f32 + 0.5).clamp(left as f32 + radius, width as f32 - radius);
                    let cy = (y as f32 + 0.5).clamp(radius, height as f32 - radius);
                    let distance =
                        ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    let a = ((radius - distance + 0.5).clamp(0.0, 1.0) * opacity * 255.0).round()
                        as u32;
                    let p = &mut surface.pixels()[(y * width + x) as usize];
                    if live && distance > border {
                        *p = nub::BORDER;
                    }
                    *p = (a << 24)
                        | (((*p >> 16 & 255) * a / 255) << 16)
                        | (((*p >> 8 & 255) * a / 255) << 8)
                        | ((*p & 255) * a / 255);
                }
            }
            Ok((surface, left))
        }
    }
    unsafe fn paint_icon(&self) -> io::Result<()> {
        self.advance(Instant::now(), self.replay_level());
        unsafe { self.present_icon() }
    }

    fn replay_level(&self) -> Option<f32> {
        (self.replay && self.frame.borrow().pill).then_some(0.1)
    }

    unsafe fn present_icon(&self) -> io::Result<()> {
        unsafe {
            let (mut surface, left) = self.draw_icon()?;
            let (width, height) = (surface.width, surface.height);
            if self.replay {
                let frame = self.frame.borrow();
                let name = if let Some(notice) = frame.notice {
                    match notice {
                        Notice::Writing => "floating-icon-writing.png",
                        Notice::Later => "floating-icon-later.png",
                        Notice::Failed(_) => "floating-icon-failed.png",
                        Notice::Skipped(SummarySkip::Off) => "floating-icon-off.png",
                        Notice::Skipped(SummarySkip::Short) => "floating-icon-summary-skipped.png",
                        Notice::Skipped(SummarySkip::Empty) => "floating-icon-empty.png",
                        Notice::Unavailable => "floating-icon-unavailable.png",
                    }
                } else if frame.skipped {
                    "floating-icon-skipped.png"
                } else if frame.pill {
                    "floating-icon-pill.png"
                } else {
                    "floating-icon-bead.png"
                };
                if !core::state_dir().join("local").join(name).exists() {
                    surface.save(name, true);
                }
            }
            if self.icon.get().is_null() {
                self.surface.replace(Some(surface));
                return Ok(());
            }
            let key = (left, width, height, self.layout.get().scale);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            if UpdateLayeredWindow(
                self.icon.get(),
                ptr::null_mut(),
                &self.point(),
                &SIZE {
                    cx: width,
                    cy: height,
                },
                surface.dc,
                &POINT { x: 0, y: 0 },
                0,
                &blend,
                ULW_ALPHA,
            ) == 0
            {
                let error = io::Error::last_os_error();
                self.surface.replace(Some(surface));
                return Err(error);
            }
            self.surface.replace(Some(surface));
            self.set_region(key, |region| SetWindowRgn(self.icon.get(), region, 1))
        }
    }
    fn set_region(
        &self,
        key: (i32, i32, i32, f32),
        install: impl FnOnce(HRGN) -> i32,
    ) -> io::Result<()> {
        unsafe {
            if self.region_geometry.get() == Some(key) {
                return Ok(());
            }
            let (left, width, height, _) = key;
            let region = icon_region(left, width, height);
            if region.is_null() {
                return Err(io::Error::last_os_error());
            }
            if install(region) == 0 {
                DeleteObject(region);
                return Err(io::Error::last_os_error());
            }
            self.region_geometry.set(Some(key));
            Ok(())
        }
    }
}

fn icon_region(left: i32, width: i32, height: i32) -> HRGN {
    let diameter = width.min(height);
    unsafe { CreateRoundRectRgn(left, 0, width + 1, height + 1, diameter, diameter) }
}

unsafe fn view<'a>(hwnd: HWND, msg: u32, l: LPARAM) -> Option<&'a View> {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const View).as_ref()
    }
}
pub(super) unsafe fn failed(hwnd: HWND) {
    unsafe {
        if let Some(v) = view(hwnd, 0, 0) {
            v.control.retire();
        }
    }
}

unsafe fn common(hwnd: HWND, msg: u32, l: LPARAM) -> Option<LRESULT> {
    unsafe {
        let v = view(hwnd, msg, l)?;
        if msg == RETIRE {
            v.control.stop.store(true, Ordering::Release);
            EndMenu();
            v.close_windows();
            PostQuitMessage(0);
            return Some(0);
        }
        if matches!(msg, WM_CONTEXTMENU | WM_NCRBUTTONUP) {
            v.capture_menu(hwnd, l);
            return Some(0);
        }
        if msg == WM_NCDESTROY {
            v.clear_handle(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        None
    }
}
unsafe extern "system" fn icon_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    callback_boundary("FI_ICON_CALLBACK", hwnd, || unsafe {
        icon_message(hwnd, msg, w, l)
    })
}
unsafe fn icon_message(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if let Some(result) = common(hwnd, msg, l) {
            return result;
        }
        if let Some(v) = view(hwnd, msg, l) {
            match msg {
                WM_TIMER if w == CAPTURE_TIMER => {
                    v.refresh_capture();
                    return 0;
                }
                m if m == WM_APP + 40 && v.replay => {
                    v.open_card();
                    return 0;
                }
                m if m == WM_APP + 41 && v.replay => {
                    return if w == 6 {
                        v.frame.borrow().skipped as isize
                    } else {
                        0
                    };
                }
                m if m == WM_APP + 45 && v.replay => {
                    let _ = std::fs::write(
                        core::state_dir().join("local/floating-icon-stall-started"),
                        b"started",
                    );
                    let deadline =
                        Instant::now() + Duration::from_secs(if w == 1 { 20 } else { 5 });
                    while Instant::now() < deadline
                        && !core::state_dir()
                            .join("local/floating-icon-release-stalls")
                            .is_file()
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    return 0;
                }
                m if m == WM_APP + 46 && v.replay => panic!("synthetic callback failure"),
                WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
                WM_CLOSE => return 0,
                WM_LBUTTONDOWN => {
                    let mut p = POINT {
                        x: l as i16 as i32,
                        y: (l >> 16) as i16 as i32,
                    };
                    ClientToScreen(hwnd, &mut p);
                    v.drag.set(Some((p.y, v.layout.get().fraction)));
                    v.moved.set(false);
                    v.reopen
                        .set(!v.card.get().is_null() && IsWindowVisible(v.card.get()) != 0);
                    SetCapture(hwnd);
                    return 0;
                }
                WM_MOUSEMOVE => {
                    if let Some((start, fraction)) = v.drag.get() {
                        let mut p = POINT {
                            x: l as i16 as i32,
                            y: (l >> 16) as i16 as i32,
                        };
                        ClientToScreen(hwnd, &mut p);
                        let dy = p.y - start;
                        if dy.abs() > v.px(4.0) {
                            v.moved.set(true);
                        }
                        if v.moved.get() {
                            if v.reopen.get() {
                                v.hide_card();
                            }
                            v.set_fraction(fraction + dy as f32 / v.travel().max(1) as f32);
                            let p = v.point();
                            SetWindowPos(
                                hwnd,
                                HWND_TOPMOST,
                                p.x,
                                p.y,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOACTIVATE,
                            );
                            v.position_card();
                        }
                        return 0;
                    }
                }
                WM_LBUTTONUP => {
                    if v.drag.take().is_some() {
                        ReleaseCapture();
                        if v.moved.get() {
                            if store_io::write(
                                &core::state_dir().join("local/floating-icon-position.json"),
                                &v.layout.get().fraction,
                            )
                            .is_err()
                            {
                                crate::listener::log("FI_POSITION_SAVE");
                            }
                            if v.reopen.get() {
                                v.open_card();
                            }
                        } else if !v.refresh_capture() {
                            return 0;
                        } else if v.frame.borrow().recording && !v.replay {
                            v.hide_card();
                            View::open_meeting_window(hwnd);
                        } else if v
                            .frame
                            .borrow()
                            .notice
                            .is_some_and(|s| s != Notice::Writing)
                        {
                            dismiss_notice(&v.frame.borrow().id);
                            ShowWindow(v.icon.get(), SW_HIDE);
                        } else {
                            v.open_card();
                        }
                        return 0;
                    }
                }
                WM_CAPTURECHANGED => {
                    if let Some((_, fraction)) = v.drag.take() {
                        v.set_fraction(fraction);
                        let p = v.point();
                        SetWindowPos(
                            hwnd,
                            HWND_TOPMOST,
                            p.x,
                            p.y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                        v.position_card();
                        if v.reopen.get() {
                            v.open_card();
                        }
                    }
                    return 0;
                }
                WM_DPICHANGED | WM_DISPLAYCHANGE => {
                    v.geometry();
                    let _ = v.paint_icon();
                    return 0;
                }
                _ => (),
            }
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}
unsafe extern "system" fn card_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    callback_boundary("FI_CARD_CALLBACK", hwnd, || unsafe {
        card_message(hwnd, msg, w, l)
    })
}
unsafe fn card_message(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if let Some(result) = common(hwnd, msg, l) {
            return result;
        }
        if let Some(v) = view(hwnd, msg, l) {
            match msg {
                WM_GETTEXT | WM_GETTEXTLENGTH | WM_GETOBJECT | WM_PRINT | WM_PRINTCLIENT => {
                    return 0;
                }
                WM_CLOSE => {
                    v.hide_card();
                    return 0;
                }
                WM_PAINT => {
                    v.paint_card(hwnd);
                    return 0;
                }
                WM_ERASEBKGND => return 1,
                WM_SIZE => {
                    InvalidateRect(hwnd, ptr::null(), 0);
                    return 0;
                }
                WM_KEYDOWN => {
                    let action = match w as u16 {
                        VK_UP | VK_LEFT => Some(Scroll::Lines(-1)),
                        VK_DOWN | VK_RIGHT => Some(Scroll::Lines(1)),
                        VK_PRIOR => Some(Scroll::Page(-1)),
                        VK_NEXT => Some(Scroll::Page(1)),
                        VK_HOME => Some(Scroll::Home),
                        VK_END => Some(Scroll::End),
                        _ => None,
                    };
                    if let Some(action) = action {
                        v.scroll_by(action);
                        return 0;
                    }
                    if w as u16 == VK_ESCAPE {
                        v.hide_card();
                        return 0;
                    }
                    if w as u16 == VK_RETURN {
                        v.acknowledge();
                        return 0;
                    }
                }
                WM_MOUSEWHEEL => {
                    let delta = (w >> 16) as i16 as i32;
                    let sum = v.wheel.get() + delta;
                    v.wheel.set(sum % 120);
                    v.scroll_by(Scroll::Lines(-3 * (sum / 120)));
                    return 0;
                }
                m if m == WM_APP + 41 && v.replay => {
                    return match w {
                        0 => v.content.borrow().updates as isize,
                        1 => v.content.borrow().text.len() as isize,
                        2 => v.scroll.get() as isize,
                        3 => v.max_scroll.get() as isize,
                        4 => IsWindowVisible(hwnd) as isize,
                        5 => v.content.borrow().minimum_live_gap_ms.unwrap_or(0) as isize,
                        _ => 0,
                    };
                }
                m if m == WM_APP + 42 && v.replay => {
                    let action = match w {
                        0 => Scroll::Lines(-1),
                        1 => Scroll::Lines(1),
                        2 => Scroll::Page(-1),
                        3 => Scroll::Page(1),
                        4 => Scroll::Home,
                        5 => Scroll::End,
                        6 => Scroll::Lines(-3),
                        _ => Scroll::Lines(3),
                    };
                    v.scroll_by(action);
                    return v.scroll.get() as isize;
                }
                m if m == WM_APP + 43 && v.replay => {
                    DestroyWindow(hwnd);
                    return 0;
                }
                m if m == WM_APP + 44 && v.replay => {
                    v.acknowledge();
                    return 0;
                }
                m if m == WM_APP + 46 && v.replay => panic!("synthetic callback failure"),
                _ => (),
            }
        }
        // In particular, SC_CLOSE can synchronously re-enter WM_CLOSE without any state borrow.
        DefWindowProcW(hwnd, msg, w, l)
    }
}

fn synthetic_view(scale: f32) -> io::Result<View> {
    unsafe {
        let font = CreateFontW(
            -(13.0 * scale).round() as i32,
            0,
            0,
            0,
            500,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        if font.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(View {
            icon: Cell::new(ptr::null_mut()),
            card: Cell::new(ptr::null_mut()),
            config_root: PathBuf::new(),
            capture_visible: Cell::new(false),
            capture_save: RefCell::default(),
            font: RefCell::new(Some(Rc::new(Font(font)))),
            timer_font: RefCell::new(None),
            surface: RefCell::default(),
            region_geometry: Cell::default(),
            layout: Cell::new(Layout {
                scale,
                work: RECT {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
                fraction: 0.6,
            }),
            frame: RefCell::new(Frame::default()),
            motion: Cell::default(),
            look: Cell::default(),
            timer_text: RefCell::default(),
            content: RefCell::new(Card::default()),
            scroll: Cell::new(0),
            max_scroll: Cell::new(0),
            wheel: Cell::new(0),
            drag: Cell::new(None),
            moved: Cell::new(false),
            reopen: Cell::new(false),
            replay: true,
            control: Arc::new(Control::new()),
        })
    }
}

#[test]
fn meeting_icon_capture_affinity_round_trips_through_config() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let mut defaults = core::Config::default();
    assert!(!defaults.meeting_icon_in_captures);
    defaults.notes = Some(core::notes::NotesSettings::default());
    let mut config = serde_json::to_value(defaults).unwrap();
    config
        .as_object_mut()
        .unwrap()
        .remove("meeting_icon_in_captures");
    store_io::write(&root.join("config.json"), &config).unwrap();

    let affinity = || capture_affinity(core::Config::meeting_icon_capture_allowed(root));
    assert_eq!(affinity(), WDA_EXCLUDEFROMCAPTURE);
    let mut config = core::Config::load_in(root).unwrap();
    config.meeting_icon_in_captures = true;
    config.save(root).unwrap();
    assert_eq!(affinity(), WDA_NONE);
    config.meeting_icon_in_captures = false;
    config.save(root).unwrap();
    assert_eq!(affinity(), WDA_EXCLUDEFROMCAPTURE);
}

#[test]
fn meeting_icon_capture_affinity_updates_live_windows() {
    use std::{io::Seek, os::windows::fs::OpenOptionsExt};

    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let mut defaults = core::Config::default();
    defaults.notes = Some(core::notes::NotesSettings::default());
    let config_path = root.join("config.json");
    store_io::write(&config_path, &defaults).unwrap();
    let mut config = core::Config::load_in(root).unwrap();
    let mut save = |visible| {
        config.meeting_icon_in_captures = visible;
        config.save(root).unwrap();
    };
    let window = NativeWindow::new(false, root.to_owned(), Arc::new(Control::new())).unwrap();
    let view = &window.view;
    let icon = view.icon.get();
    let card = unsafe { view.prepare_card() };
    let affinity = |hwnd| unsafe {
        assert_eq!(IsWindowVisible(hwnd), 0);
        let mut value = u32::MAX;
        assert_ne!(
            GetWindowDisplayAffinity(hwnd, &mut value),
            0,
            "{}",
            io::Error::last_os_error()
        );
        value
    };
    let wait_for = |card, expected| {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(250);
        loop {
            let actual = (affinity(icon), affinity(card));
            if actual == (expected, expected) || Instant::now() >= deadline {
                assert_eq!(actual, (expected, expected), "after {:?}", start.elapsed());
                assert!(start.elapsed() <= Duration::from_millis(250));
                break;
            }
            assert!(window.pump());
        }
    };

    assert_eq!(affinity(icon), WDA_EXCLUDEFROMCAPTURE);
    assert_eq!(affinity(card), WDA_EXCLUDEFROMCAPTURE);
    save(true);
    wait_for(card, WDA_NONE);
    save(false);
    wait_for(card, WDA_EXCLUDEFROMCAPTURE);

    let select_capture_menu_item = || {
        view.toggle_capture();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !view.capture_save.borrow().as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline, "capture worker did not finish");
            assert!(window.pump());
        }
    };
    select_capture_menu_item();
    assert!(core::Config::meeting_icon_capture_allowed(root));
    wait_for(card, WDA_NONE);

    // An occupied writer lock must leave the saved value and the checkmark alone.
    let lock = store_io::lock(&root.join("local/config.lock")).unwrap();
    select_capture_menu_item();
    assert!(core::Config::meeting_icon_capture_allowed(root));
    assert!(view.capture_visible.get());
    assert_eq!(affinity(icon), WDA_NONE);
    assert_eq!(affinity(card), WDA_NONE);
    assert!(
        std::fs::read_to_string(core::log_path())
            .unwrap()
            .contains(CAPTURE_SAVE_FAILED)
    );
    drop(lock);

    select_capture_menu_item();
    assert!(!core::Config::meeting_icon_capture_allowed(root));
    wait_for(card, WDA_EXCLUDEFROMCAPTURE);

    // Create each card before pumping any timer messages for the new value.
    unsafe { view.destroy(card) };
    save(true);
    let card = unsafe { view.prepare_card() };
    assert_eq!(affinity(card), WDA_NONE);
    unsafe { view.destroy(card) };
    save(false);
    let card = unsafe { view.prepare_card() };
    assert_eq!(affinity(card), WDA_EXCLUDEFROMCAPTURE);

    save(true);
    wait_for(card, WDA_NONE);

    save(false);
    let mut locked = std::fs::OpenOptions::new()
        .write(true)
        .share_mode(0)
        .open(&config_path)
        .unwrap();
    assert_eq!(
        std::fs::read(&config_path).unwrap_err().raw_os_error(),
        Some(32)
    );
    let exclude_on_tick = |card| {
        let start = Instant::now();
        unsafe { SendMessageW(icon, WM_TIMER, CAPTURE_TIMER, 0) };
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(u64::from(CAPTURE_POLL_MS)),
            "capture timer took {elapsed:?}"
        );
        assert_eq!(affinity(icon), WDA_EXCLUDEFROMCAPTURE);
        assert_eq!(affinity(card), WDA_EXCLUDEFROMCAPTURE);
    };
    exclude_on_tick(card);
    let mut card = card;
    for visible in [false, true, false] {
        defaults.meeting_icon_in_captures = visible;
        locked.set_len(0).unwrap();
        locked.rewind().unwrap();
        serde_json::to_writer(&mut locked, &defaults).unwrap();
        unsafe { view.destroy(card) };
        card = unsafe { view.prepare_card() };
        assert_eq!(affinity(card), WDA_EXCLUDEFROMCAPTURE);
        exclude_on_tick(card);
    }
    drop(locked);
    wait_for(card, WDA_EXCLUDEFROMCAPTURE);
    save(true);
    wait_for(card, WDA_NONE);

    std::fs::write(&config_path, b"{").unwrap();
    wait_for(card, WDA_EXCLUDEFROMCAPTURE);
    store_io::write(&config_path, &config).unwrap();
    wait_for(card, WDA_NONE);
    std::fs::remove_file(config_path).unwrap();
    wait_for(card, WDA_EXCLUDEFROMCAPTURE);
}

pub(super) fn render_synthetic() -> io::Result<()> {
    unsafe {
        let view = synthetic_view(1.0)?;
        for notice in [
            Notice::Writing,
            Notice::Later,
            Notice::Failed(crate::core::meeting_job::Cause::Other),
            Notice::Skipped(SummarySkip::Off),
            Notice::Skipped(SummarySkip::Short),
            Notice::Skipped(SummarySkip::Empty),
            Notice::Unavailable,
        ] {
            view.frame.borrow_mut().notice = Some(notice);
            view.paint_icon()?;
        }
        view.frame.borrow_mut().notice = None;
        for (recording, pill, skipped) in [
            (true, false, false),
            (true, true, false),
            (false, false, true),
        ] {
            let mut frame = view.frame.borrow_mut();
            frame.recording = recording;
            frame.pill = pill;
            frame.skipped = skipped;
            frame.elapsed = 120;
            drop(frame);
            view.paint_icon()?;
        }
        Ok(())
    }
}

#[test]
fn nub_painter_renders_recording_and_quiet_at_both_scales() {
    for scale in [1.0, 1.5] {
        let view = synthetic_view(scale).unwrap();
        let start = Instant::now();
        let mut last_level = -1.0;
        for level in [None, Some(0.012), Some(0.16)] {
            view.motion.set(nub::Motion::default());
            view.frame.replace(Frame {
                recording: true,
                elapsed: 7559,
                ..Frame::default()
            });
            view.advance(start, level);
            view.advance(start + Duration::from_secs(1), level);
            let mapped = view.look.get().unwrap().level;
            assert!(mapped > last_level);
            last_level = mapped;
            let (mut surface, left) = view.draw_icon().unwrap();
            let (width, height) = (surface.width, surface.height);
            assert_eq!((left, width, height), (0, view.px(151.0), view.px(36.0)));
            let pixels = surface.pixels();
            let pixel = |x: f32, y: f32| {
                pixels[(y * scale) as usize * width as usize + (x * scale) as usize]
            };
            assert!(pixel(0.0, 18.0) >> 24 > 240);
            assert_eq!(pixel(24.0, 4.0), 0xff1d1d1b);
            let has_color = pixels.chunks_exact(width as usize).any(|row| {
                row[..view.px(92.0) as usize]
                    .iter()
                    .any(|p| (p >> 16 & 255) > (p & 255) + 30)
            });
            assert_eq!(has_color, level.is_some());
            assert!(pixels.iter().any(|p| (p & 0xff) > 0x70));
            unsafe {
                let _font = SelectedFont::new(surface.dc, view.timer_font().unwrap());
                let text = wide("125:59");
                let mut extent: SIZE = std::mem::zeroed();
                assert_ne!(
                    GetTextExtentPoint32W(surface.dc, text.as_ptr(), 6, &raw mut extent),
                    0
                );
                assert!((view.px(20.0)..=width - 2 * view.px(3.0)).contains(&extent.cx));
            }
        }
    }
}

#[test]
fn nub_pump_repaints_steps_only_during_speech() {
    let view = synthetic_view(1.0).unwrap();
    let mut frame = Frame {
        recording: true,
        elapsed: 9,
        ..Frame::default()
    };
    view.frame.replace(frame.clone());
    frame.elapsed += 1;
    let prior = view.frame.replace(frame);
    assert!(icon_changed(&prior, &view.frame.borrow()));
    let start = Instant::now();
    view.advance(start, None);
    assert_eq!(view.animation_interval(), 50);
    let (mut first, _) = view.draw_icon().unwrap();
    assert!(!view.advance(start + Duration::from_millis(50), None));
    let (mut second, _) = view.draw_icon().unwrap();
    assert_eq!(first.pixels(), second.pixels());
    assert!(view.advance(start + Duration::from_millis(66), Some(0.16)));
    assert_eq!(view.animation_interval(), 16);
    let loud = view.look.get().unwrap().level;
    assert!(loud > 0.0);
    view.frame.borrow_mut().recording = false;
    view.advance(start + Duration::from_millis(82), Some(0.16));
    assert!(view.look.get().unwrap().level < loud);
    view.advance(start + Duration::from_millis(307), None);
    assert!((view.look.get().unwrap().opacity - 0.5).abs() < 0.001);
    view.advance(start + Duration::from_millis(532), None);
    assert!(view.look.get().is_none());
    assert_eq!(view.animation_interval(), 0);
}

#[test]
fn nub_painter_reuses_timer_font_until_pixel_height_changes() {
    let mut view = synthetic_view(1.0).unwrap();
    view.replay = false;
    view.frame.borrow_mut().recording = true;
    let cached = || view.timer_font.borrow().clone().unwrap();
    let installs = Cell::new(0);
    let paint = || unsafe {
        view.paint_icon().unwrap();
        let surface = view.surface.borrow();
        let surface = surface.as_ref().unwrap();
        let key = (0, surface.width, surface.height, view.layout.get().scale);
        view.set_region(key, |region| {
            installs.set(installs.get() + 1);
            DeleteObject(region)
        })
        .unwrap();
        surface.bitmap
    };
    let bitmap = paint();
    let (height, first) = cached();
    assert_eq!((paint(), installs.get()), (bitmap, 1));
    assert!(
        Rc::ptr_eq(&first, &cached().1),
        "same height must reuse font"
    );

    view.layout.set(Layout {
        scale: 1.5,
        ..view.layout.get()
    });
    assert_ne!(paint(), bitmap);
    assert_eq!(installs.get(), 2);
    let surface = view.surface.borrow();
    let surface = surface.as_ref().unwrap();
    let (next_height, next) = cached();
    assert_eq!((height, next_height), (13, 20));
    assert_ne!(first.0, next.0);
    unsafe {
        let _font = SelectedFont::new(surface.dc, next);
        let mut face = [0u16; 32];
        assert!(GetTextFaceW(surface.dc, face.len() as i32, face.as_mut_ptr()) > 0);
        assert!(face.starts_with(&wide("Consolas")[..8]));
    }
}

#[test]
fn nub_whole_pill_admits_legacy_clicks() {
    for scale in [1.0, 1.5] {
        let view = synthetic_view(scale).unwrap();
        view.frame.borrow_mut().recording = true;
        view.advance(Instant::now(), None);
        let (mut surface, left) = view.draw_icon().unwrap();
        let (width, height) = (surface.width, surface.height);
        let pixels = surface.pixels();
        unsafe {
            let region = icon_region(left, width, height);
            assert!(!region.is_null());
            for y in 0..height {
                for x in 0..width {
                    let radius = height as f32 / 2.0;
                    let x_center = (x as f32 + 0.5).clamp(radius, width as f32 - radius);
                    let distance = (x as f32 + 0.5 - x_center).hypot(y as f32 + 0.5 - radius);
                    let alpha = pixels[(y * width + x) as usize] >> 24;
                    let hit = alpha != 0 && PtInRegion(region, x, y) != 0;
                    // GDI and the alpha mask can round the outermost pixels differently.
                    if distance <= radius - 1.0 || distance >= radius + 0.5 {
                        assert_eq!(
                            hit,
                            distance < radius,
                            "legacy click at ({x}, {y}), {scale}x"
                        );
                    }
                }
            }
            DeleteObject(region);
        }
    }
}
