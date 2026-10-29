//! Windows: the foreground window, its process, and the time of the last input (user32/kernel32 through windows-sys).
//!
//! Three things here exist because the obvious call goes wrong:
//! - the title is read with `InternalGetWindowText`, not `GetWindowTextW`: the latter sends `WM_GETTEXT` and blocks
//!   for as long as the other process is hung, which would stall the whole collector;
//! - a window whose process cannot be opened (an elevated app) is reported as *unreadable*, not as a locked screen;
//! - a Store (UWP) app is in front as `ApplicationFrameHost.exe`; the real app is a child window of another process.
use life_agent_core::agent::Sampler;
use life_agent_core::segments::Sample;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, GetLastError, HANDLE, HWND, LPARAM,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetForegroundWindow, GetWindowThreadProcessId, InternalGetWindowText,
};

/// The lock screen and the secure desktop are not "work": treat them like having no window.
const NOT_WORK: [&str; 2] = ["lockapp.exe", "logonui.exe"];
const FRAME_HOST: &str = "applicationframehost.exe";
/// A title longer than this is dropped whole: it is a page-made string, and keeping part of it only risks a cut
/// through the middle of something private. Real document and site titles never get near it.
const TITLE_CHARS: usize = 2048;
const CHILD_PIDS_MAX: usize = 64;

pub struct WindowsSampler;

enum Process {
    Name(String),
    Denied,
    Gone,
}

fn idle_seconds() -> f64 {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a valid LASTINPUTINFO with cbSize set, as the API requires.
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return 0.0;
    }
    // SAFETY: GetTickCount takes no arguments. Tick counts wrap every 49 days, so subtract with wrapping.
    let now = unsafe { GetTickCount() };
    f64::from(now.wrapping_sub(info.dwTime)) / 1000.0
}

/// Closes the process handle on every way out.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: a non-null handle from OpenProcess, closed exactly once, by its only owner.
        unsafe { CloseHandle(self.0) };
    }
}

fn process_name(pid: u32) -> Process {
    // SAFETY: plain query access to a process id; a null handle is handled below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        // SAFETY: reads the calling thread's last error, right after the failed call.
        return if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
            Process::Denied
        } else {
            Process::Gone
        };
    }
    let handle = OwnedHandle(handle);
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units and `handle.0` is the open process handle from above.
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, buf.as_mut_ptr(), &mut len) } == 0 {
        return Process::Gone;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Process::Name(path.rsplit('\\').next().unwrap_or("").to_string())
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = vec![0u16; TITLE_CHARS * 2 + 2];
    // SAFETY: `buf` has room for `buf.len()` UTF-16 units including the terminator; this call does not message the
    // window's owner, so a hung process cannot block it.
    let got = unsafe { InternalGetWindowText(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    let title = String::from_utf16_lossy(&buf[..got.max(0) as usize]);
    if title.chars().count() > TITLE_CHARS {
        String::new()
    } else {
        title
    }
}

unsafe extern "system" fn collect_child_pid(child: HWND, out: LPARAM) -> i32 {
    // SAFETY: `out` is the address of the Vec<u32> that `child_pids` keeps alive for the whole enumeration.
    let pids = unsafe { &mut *(out as *mut Vec<u32>) };
    let mut pid = 0u32;
    // SAFETY: `pid` is a writable u32.
    unsafe { GetWindowThreadProcessId(child, &mut pid) };
    if pid != 0 && !pids.contains(&pid) && pids.len() < CHILD_PIDS_MAX {
        pids.push(pid);
    }
    1
}

fn child_pids(frame: HWND) -> Vec<u32> {
    let mut pids: Vec<u32> = Vec::new();
    // SAFETY: the callback only touches the Vec passed through the pointer, which outlives the call.
    unsafe {
        EnumChildWindows(
            frame,
            Some(collect_child_pid),
            &mut pids as *mut Vec<u32> as LPARAM,
        )
    };
    pids
}

/// The real app behind an `ApplicationFrameHost` window: the first child window owned by another readable process.
fn hosted_app(frame: HWND, frame_pid: u32) -> Option<String> {
    child_pids(frame)
        .into_iter()
        .filter(|p| *p != frame_pid)
        .find_map(|pid| match process_name(pid) {
            Process::Name(n) if !n.is_empty() => Some(n),
            _ => None,
        })
}

impl Sampler for WindowsSampler {
    fn sample(&mut self) -> Result<Sample, String> {
        let idle_s = idle_seconds();
        let none = Sample {
            idle_s,
            ..Sample::default()
        };
        // SAFETY: no arguments; returns null when no window is in the foreground.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return Ok(none);
        }
        let mut pid = 0u32;
        // SAFETY: `pid` is a writable u32.
        if unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } == 0 || pid == 0 {
            return Ok(none); // the window vanished while we looked
        }
        let mut app = match process_name(pid) {
            Process::Name(n) if !n.is_empty() => n,
            Process::Denied => {
                return Ok(Sample {
                    unreadable: true,
                    ..none
                });
            }
            _ => return Ok(none),
        };
        if NOT_WORK.contains(&app.to_lowercase().as_str()) {
            return Ok(none);
        }
        if app.to_lowercase() == FRAME_HOST
            && let Some(real) = hosted_app(hwnd, pid)
        {
            app = real;
        }
        Ok(Sample {
            title: window_title(hwnd),
            app,
            idle_s,
            unreadable: false,
        })
    }
}
