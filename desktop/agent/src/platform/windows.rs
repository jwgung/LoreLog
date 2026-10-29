//! Windows: the foreground window, its process, and the time of the last input (user32/kernel32 through windows-sys).
use life_agent_core::agent::Sampler;
use life_agent_core::segments::Sample;
use windows_sys::Win32::Foundation::{CloseHandle, HWND};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
};

/// The lock screen and the secure desktop are not "work": treat them like having no window.
const NOT_WORK: [&str; 2] = ["lockapp.exe", "logonui.exe"];

pub struct WindowsSampler;

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

fn process_name(hwnd: HWND) -> String {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer for the call.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    // SAFETY: plain query access to a process id; a null handle (access denied) is handled below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units and `handle` is the open process handle from above.
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) };
    // SAFETY: closing the handle opened above, once.
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().unwrap_or("").to_string()
}

fn window_title(hwnd: HWND) -> String {
    // SAFETY: `hwnd` came from GetForegroundWindow; the call only reads the window's title length.
    let n = unsafe { GetWindowTextLengthW(hwnd) };
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    // SAFETY: `buf` has room for `n + 1` UTF-16 units including the terminator.
    let got = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

impl Sampler for WindowsSampler {
    fn sample(&mut self) -> Result<Sample, String> {
        let idle_s = idle_seconds();
        // SAFETY: no arguments; returns null when no window is in the foreground.
        let hwnd = unsafe { GetForegroundWindow() };
        let none = Sample {
            app: String::new(),
            title: String::new(),
            idle_s,
        };
        if hwnd.is_null() {
            return Ok(none);
        }
        let app = process_name(hwnd);
        if app.is_empty() || NOT_WORK.contains(&app.to_lowercase().as_str()) {
            return Ok(none);
        }
        Ok(Sample {
            title: window_title(hwnd),
            app,
            idle_s,
        })
    }
}
