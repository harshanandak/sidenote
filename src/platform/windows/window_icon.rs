use std::{cell::Cell, ptr, rc::Rc};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            DestroyIcon, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, LoadImageW, SM_CXICON,
            SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_DPICHANGED, WM_NCDESTROY,
            WM_SETICON,
        },
    },
};

const SUBCLASS_ID: usize = 2;

enum Source {
    Executable,
    #[cfg(test)]
    File(Vec<u16>),
}

impl Source {
    fn load(&self, width: i32, height: i32) -> Option<Icon> {
        let (module, name, flags) = match self {
            // Resource group 1, embedded by build.rs.
            Self::Executable => (
                unsafe { GetModuleHandleW(ptr::null()) },
                ptr::without_provenance::<u16>(1),
                0,
            ),
            #[cfg(test)]
            Self::File(name) => (
                ptr::null_mut(),
                name.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::LR_LOADFROMFILE,
            ),
        };
        let handle = unsafe { LoadImageW(module, name, IMAGE_ICON, width, height, flags) };
        if handle.is_null() {
            None
        } else {
            Some(Icon(handle))
        }
    }
}

struct Icon(HICON);

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.0) };
    }
}

struct WindowIcons {
    source: Source,
    small: Cell<Option<Icon>>,
    large: Cell<Option<Icon>>,
}

impl WindowIcons {
    fn refresh(&self, hwnd: HWND, dpi: u32) {
        for (current, kind, cx, cy) in [
            (&self.small, ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (&self.large, ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            let (width, height) = unsafe {
                (
                    GetSystemMetricsForDpi(cx, dpi),
                    GetSystemMetricsForDpi(cy, dpi),
                )
            };
            if let Some(icon) = self.source.load(width, height) {
                let handle = icon.0;
                let previous = current.replace(Some(icon));
                unsafe { SendMessageW(hwnd, WM_SETICON, kind as usize, handle as isize) };
                drop(previous);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    let icons = data as *const WindowIcons;
    unsafe {
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(window_proc), id);
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            drop(Rc::from_raw(icons));
            return result;
        }
        if msg == WM_DPICHANGED {
            // WM_SETICON dispatches synchronously and can re-enter the window procedure.
            Rc::increment_strong_count(icons);
            let icons = Rc::from_raw(icons);
            icons.refresh(hwnd, (wp & 0xffff) as u32);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

fn install(hwnd: HWND, source: Source) -> bool {
    let icons = Rc::new(WindowIcons {
        source,
        small: Cell::new(None),
        large: Cell::new(None),
    });
    let subclass = Rc::into_raw(Rc::clone(&icons));
    if unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, subclass as usize) } == 0 {
        drop(unsafe { Rc::from_raw(subclass) });
        return false;
    }
    let dpi = match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        dpi => dpi,
    };
    icons.refresh(hwnd, dpi);
    true
}

pub(super) fn set_window_icon(hwnd: isize) {
    install(hwnd as HWND, Source::Executable);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{mem, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::{
        Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetIconInfo, HWND_MESSAGE, ICONINFO, WM_GETICON,
        },
    };

    struct TestWindow(HWND);

    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.0) };
        }
    }

    fn icon_size(hwnd: HWND, kind: u32) -> (i32, i32) {
        let handle = unsafe { SendMessageW(hwnd, WM_GETICON, kind as usize, 0) } as HICON;
        assert!(!handle.is_null());
        let mut info: ICONINFO = unsafe { mem::zeroed() };
        assert_ne!(unsafe { GetIconInfo(handle, &mut info) }, 0);
        let mut bitmap: BITMAP = unsafe { mem::zeroed() };
        let bytes = unsafe {
            let bytes = GetObjectW(
                info.hbmColor,
                mem::size_of::<BITMAP>() as i32,
                (&raw mut bitmap).cast(),
            );
            DeleteObject(info.hbmColor);
            DeleteObject(info.hbmMask);
            bytes
        };
        assert_eq!(bytes as usize, mem::size_of::<BITMAP>());
        assert_eq!(bitmap.bmBitsPixel, 32);
        (bitmap.bmWidth, bitmap.bmHeight)
    }

    #[test]
    fn window_reloads_both_icons_at_each_dpi_change() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon/meetings.ico");
        let file = path.as_os_str().encode_wide().chain([0]).collect();
        let window = TestWindow(unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        });
        let hwnd = window.0;
        assert!(!hwnd.is_null());
        assert!(install(hwnd, Source::File(file)));
        for (dpi, small, large) in [
            (96, 16, 32),
            (120, 20, 40),
            (144, 24, 48),
            (192, 32, 64),
            (96, 16, 32),
        ] {
            unsafe {
                let suggested: windows_sys::Win32::Foundation::RECT = mem::zeroed();
                SendMessageW(
                    hwnd,
                    WM_DPICHANGED,
                    dpi | (dpi << 16),
                    (&raw const suggested) as LPARAM,
                );
            }
            assert_eq!(icon_size(hwnd, ICON_SMALL), (small, small));
            assert_eq!(icon_size(hwnd, ICON_BIG), (large, large));
        }
    }
}
