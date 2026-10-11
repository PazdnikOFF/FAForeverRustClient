//! `fa-window.exe`: a borderless, cursor-confining window for Forged Alliance
//! under Wine on macOS.
//!
//! Started by the client beside the game, in the same Wine prefix, so it
//! shares the game's window station. It finds the game's main window, and:
//!
//! - when the game runs windowed, takes the frame away and fills the monitor
//!   the window is on. A window that covers a screen is shown by the Wine Mac
//!   driver as full screen (menu bar and Dock hidden), and unlike Direct3D's
//!   exclusive full screen it is not minimised when another application gets
//!   the focus, which is what made a second display unusable;
//! - while the game is the foreground window, keeps the cursor inside it, so
//!   pushing against the edge scrolls the map instead of sliding onto the
//!   other display. The game's own `ClipCursor` cannot do this under the Mac
//!   driver from another process, and the driver's confinement only covers
//!   exclusive full screen; `SetCursorPos` is global, so a poll can.
//!
//! It exits when the game does, or after three minutes without finding it.
//!
//! No standard library and no C runtime: built with `rustc` alone for
//! `x86_64-pc-windows-msvc`, importing straight from user32 and kernel32
//! (`raw-dylib`), linked by `rust-lld`. See `scripts/build-fa-window.mjs`.

#![no_std]
#![no_main]

use core::ffi::c_void;

type Hwnd = *mut c_void;
type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MonitorInfo {
    cb_size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumWindows(callback: extern "system" fn(Hwnd, isize) -> i32, param: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    fn IsWindow(hwnd: Hwnd) -> i32;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn IsIconic(hwnd: Hwnd) -> i32;
    fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
    fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
    fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn MonitorFromWindow(hwnd: Hwnd, flags: u32) -> *mut c_void;
    fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
    fn GetForegroundWindow() -> Hwnd;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
    fn SetForegroundWindow(hwnd: Hwnd) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
    fn Sleep(millis: u32);
    fn GetTickCount() -> u32;
    fn ExitProcess(code: u32) -> !;
}

const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const WS_CAPTION: isize = 0x00C0_0000;
const WS_THICKFRAME: isize = 0x0004_0000;
const WS_SYSMENU: isize = 0x0008_0000;
const WS_MINIMIZEBOX: isize = 0x0002_0000;
const WS_MAXIMIZEBOX: isize = 0x0001_0000;
const WS_POPUP: isize = 0x8000_0000;
const FRAME: isize = WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
const EX_FRAME: isize = 0x1 | 0x100 | 0x200 | 0x2_0000;
const SWP_NOZORDER: u32 = 0x4;
const SWP_NOACTIVATE: u32 = 0x10;
const SWP_FRAMECHANGED: u32 = 0x20;
const SWP_NOOWNERZORDER: u32 = 0x200;
const MONITOR_DEFAULTTONEAREST: u32 = 2;
const SW_RESTORE: i32 = 9;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const WAIT_OBJECT_0: u32 = 0;

/// How long to wait for the game's window before giving up: a cold start
/// that compiles shaders takes a while, a game that never starts does not.
const FIND_TIMEOUT_MS: u32 = 180_000;
/// 125 checks a second: below what a hand on a mouse can outrun at the edge
/// of a screen, and still nothing in CPU terms.
const POLL_MS: u32 = 8;
/// How long after the window appears it is brought up, in polls: a minute.
/// Long enough for a slow first start, short enough never to pull the game
/// forward over something the player went to on purpose.
const BRING_UP_POLLS: u32 = 60_000 / POLL_MS;

/// Whether `pid` runs `ForgedAlliance.exe`, compared without case.
fn is_game(pid: u32) -> bool {
    const NAME: &[u8] = b"forgedalliance.exe";
    // Left uninitialised on purpose: zeroing it would make the compiler call
    // `memset`, which nothing here provides. Only what the call wrote is read.
    let mut buffer = core::mem::MaybeUninit::<[u16; 1024]>::uninit();
    let mut size = 1024u32;
    let found = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr().cast(), &mut size) != 0;
        CloseHandle(process);
        ok
    };
    if !found || (size as usize) < NAME.len() || size > 1024 {
        return false;
    }
    let written = unsafe { core::slice::from_raw_parts(buffer.as_ptr().cast::<u16>(), size as usize) };
    let tail = &written[written.len() - NAME.len()..];
    tail.iter().zip(NAME).all(|(&have, &want)| {
        let lower = if (b'A' as u16..=b'Z' as u16).contains(&have) { have + 32 } else { have };
        lower == want as u16
    })
}

struct Search {
    best: Hwnd,
    area: i64,
    pid: u32,
}

extern "system" fn consider(hwnd: Hwnd, param: isize) -> i32 {
    let search = unsafe { &mut *(param as *mut Search) };
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if !is_game(pid) {
            return 1;
        }
        let mut rect = Rect::default();
        GetWindowRect(hwnd, &mut rect);
        let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
        if area > search.area {
            search.best = hwnd;
            search.area = area;
            search.pid = pid;
        }
    }
    1
}

/// The game's largest visible top-level window, and the game's process id.
fn find_game() -> Option<(Hwnd, u32)> {
    let mut search = Search {
        best: core::ptr::null_mut(),
        area: 0,
        pid: 0,
    };
    unsafe { EnumWindows(consider, &mut search as *mut Search as isize) };
    (!search.best.is_null()).then_some((search.best, search.pid))
}

/// Take the frame off a windowed game and fill its monitor. Does nothing to
/// a window that has no frame, which is the exclusive full-screen mode.
fn make_borderless(hwnd: Hwnd) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if style & WS_CAPTION == 0 || IsIconic(hwnd) != 0 {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !FRAME) | WS_POPUP);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !EX_FRAME);
        let mut info = MonitorInfo {
            cb_size: core::mem::size_of::<MonitorInfo>() as u32,
            monitor: Rect::default(),
            work: Rect::default(),
            flags: 0,
        };
        if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) == 0 {
            return;
        }
        let m = info.monitor;
        SetWindowPos(
            hwnd,
            core::ptr::null_mut(),
            m.left,
            m.top,
            m.right - m.left,
            m.bottom - m.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
        );
    }
}

/// Bring the game up the first time, and only then.
///
/// The game starts while the client is the active application, and macOS no
/// longer lets an application take the focus by itself; the game's window
/// comes up minimised to the Dock as a result. Until it has once been the
/// foreground window, it is restored and brought forward. After that it is
/// the player's: minimising it on purpose is left alone. Returns whether the
/// game has reached the front.
fn bring_up(hwnd: Hwnd) -> bool {
    unsafe {
        if IsIconic(hwnd) == 0 && GetForegroundWindow() == hwnd {
            return true;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
    false
}

/// Keep the cursor inside the window while it is the foreground one.
fn confine(hwnd: Hwnd) {
    unsafe {
        if GetForegroundWindow() != hwnd || IsIconic(hwnd) != 0 {
            return;
        }
        let mut rect = Rect::default();
        let mut at = Point::default();
        if GetWindowRect(hwnd, &mut rect) == 0 || GetCursorPos(&mut at) == 0 {
            return;
        }
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return;
        }
        let x = at.x.clamp(rect.left, rect.right - 1);
        let y = at.y.clamp(rect.top, rect.bottom - 1);
        if x != at.x || y != at.y {
            SetCursorPos(x, y);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn mainCRTStartup() -> ! {
    let started = unsafe { GetTickCount() };
    let (mut hwnd, pid) = loop {
        if let Some(found) = find_game() {
            break found;
        }
        if unsafe { GetTickCount() }.wrapping_sub(started) > FIND_TIMEOUT_MS {
            unsafe { ExitProcess(1) }
        }
        unsafe { Sleep(500) };
    };
    let process = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    let mut tick: u32 = 0;
    let mut up = false;
    let mut since_found: u32 = 0;
    loop {
        if !process.is_null() && unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0 {
            unsafe { ExitProcess(0) }
        }
        // Twice a second, look again: the game replaces its window when it
        // switches between windowed and full screen, and a mode change can put
        // the frame back.
        if tick % 64 == 0 {
            if unsafe { IsWindow(hwnd) } == 0 {
                match find_game() {
                    Some((found, _)) => {
                        hwnd = found;
                        up = false;
                        since_found = 0;
                    }
                    None if process.is_null() => unsafe { ExitProcess(0) },
                    None => {}
                }
            }
            if !up && since_found < BRING_UP_POLLS {
                // Restored before the frame comes off: a minimised window has
                // no monitor-sized rectangle to give it.
                up = bring_up(hwnd);
            }
            make_borderless(hwnd);
        }
        confine(hwnd);
        tick = tick.wrapping_add(1);
        since_found = since_found.saturating_add(1);
        unsafe { Sleep(POLL_MS) };
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { ExitProcess(2) }
}

