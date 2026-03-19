//! Native single-key hotkey monitoring.
//!
//! Detects a "solo tap" of Right Command (macOS) or Right Control (Windows):
//!   1. Key pressed → mark pending
//!   2. If any other key pressed while held → cancel (it's a combo like Cmd+C)
//!   3. Key released within 400ms with no other keys → trigger callback

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Max duration (ms) between press and release to count as a "solo tap".
const SOLO_TAP_MAX_MS: u64 = 400;
/// Debounce interval (ms) to prevent double-fires.
const DEBOUNCE_MS: u64 = 500;

static CALLBACK: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();
static DEBOUNCE_LAST: AtomicU64 = AtomicU64::new(0);
static PAUSED: AtomicBool = AtomicBool::new(false);

fn trigger_callback() {
    if PAUSED.load(Ordering::SeqCst) {
        return;
    }
    let now = now_ms();
    let last = DEBOUNCE_LAST.load(Ordering::SeqCst);
    if now.saturating_sub(last) < DEBOUNCE_MS {
        return;
    }
    DEBOUNCE_LAST.store(now, Ordering::SeqCst);

    if let Some(cb) = CALLBACK.get() {
        cb();
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Temporarily disable the native hotkey (e.g. while capturing a custom shortcut).
pub fn pause() {
    PAUSED.store(true, Ordering::SeqCst);
}

/// Re-enable the native hotkey.
pub fn resume() {
    PAUSED.store(false, Ordering::SeqCst);
}

/// Start the native hotkey monitor on a background thread.
/// The callback is invoked (on its own thread) when a solo tap is detected.
pub fn start(callback: impl Fn() + Send + Sync + 'static) {
    let _ = CALLBACK.set(Box::new(callback));
    platform::start();
}

// ── macOS: CGEventTap ────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::AtomicIsize;

    // Opaque CF/CG types
    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type CFMachPortRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CFAllocatorRef = *const c_void;
    type CFStringRef = *const c_void;

    // CGEvent constants
    const K_CG_EVENT_KEY_DOWN: u32 = 10;
    const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
    const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
    const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9; // CGEventField
    const K_VK_RIGHT_COMMAND: i64 = 0x36;
    const K_NX_DEVICE_RCMD_KEY_MASK: u64 = 0x0000_0010;

    // CGEventTap creation params
    const K_CG_SESSION_EVENT_TAP: u32 = 1;
    const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
        fn CGEventGetFlags(event: CGEventRef) -> u64;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFMachPortCreateRunLoopSource(
            allocator: CFAllocatorRef,
            port: CFMachPortRef,
            order: i64,
        ) -> CFRunLoopSourceRef;
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
        fn CFRunLoopRun();
        static kCFRunLoopCommonModes: CFStringRef;
    }

    // Per-tap state
    static KEY_DOWN: AtomicBool = AtomicBool::new(false);
    static KEY_TIME: AtomicU64 = AtomicU64::new(0);
    static OTHER_KEY: AtomicBool = AtomicBool::new(false);
    static TAP: AtomicIsize = AtomicIsize::new(0);

    extern "C" fn event_callback(
        _proxy: CGEventTapProxy,
        event_type: u32,
        event: CGEventRef,
        _user_info: *mut c_void,
    ) -> CGEventRef {
        if matches!(
            event_type,
            K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT | K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
        ) {
            let tap = TAP.load(Ordering::SeqCst) as CFMachPortRef;
            if !tap.is_null() {
                unsafe { CGEventTapEnable(tap, true) };
            }
            return event;
        }

        if event.is_null() {
            return event;
        }

        unsafe {
            let keycode = CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE);
            let flags = CGEventGetFlags(event);

            match event_type {
                K_CG_EVENT_FLAGS_CHANGED if keycode == K_VK_RIGHT_COMMAND => {
                    let right_cmd_down = (flags & K_NX_DEVICE_RCMD_KEY_MASK) != 0;
                    if right_cmd_down {
                        if !KEY_DOWN.swap(true, Ordering::SeqCst) {
                            KEY_TIME.store(now_ms(), Ordering::SeqCst);
                            OTHER_KEY.store(false, Ordering::SeqCst);
                        }
                    } else if KEY_DOWN.swap(false, Ordering::SeqCst) {
                        let held = now_ms().saturating_sub(KEY_TIME.load(Ordering::SeqCst));
                        if !OTHER_KEY.load(Ordering::SeqCst) && held < SOLO_TAP_MAX_MS {
                            trigger_callback();
                        }
                    }
                }
                K_CG_EVENT_FLAGS_CHANGED => {
                    if KEY_DOWN.load(Ordering::SeqCst) {
                        OTHER_KEY.store(true, Ordering::SeqCst);
                    }
                }
                K_CG_EVENT_KEY_DOWN => {
                    if KEY_DOWN.load(Ordering::SeqCst) && keycode != K_VK_RIGHT_COMMAND {
                        OTHER_KEY.store(true, Ordering::SeqCst);
                    }
                }
                _ => {}
            }
        }
        event
    }

    pub fn start() {
        std::thread::spawn(|| {
            let mask: u64 = (1u64 << K_CG_EVENT_KEY_DOWN) | (1u64 << K_CG_EVENT_FLAGS_CHANGED);

            unsafe {
                if !CGPreflightListenEventAccess() {
                    let _ = CGRequestListenEventAccess();
                }
            }

            // Retry loop — event listening requires Input Monitoring permission,
            // which may not yet be granted at launch.
            loop {
                let tap = unsafe {
                    CGEventTapCreate(
                        K_CG_SESSION_EVENT_TAP,
                        K_CG_HEAD_INSERT_EVENT_TAP,
                        K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                        mask,
                        event_callback,
                        std::ptr::null_mut(),
                    )
                };

                if !tap.is_null() {
                    TAP.store(tap as isize, Ordering::SeqCst);
                    unsafe {
                        CGEventTapEnable(tap, true);
                        let source =
                            CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
                        if source.is_null() {
                            log::error!("Failed to create CFRunLoopSource");
                            return;
                        }
                        let rl = CFRunLoopGetCurrent();
                        CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
                        log::info!("Native hotkey started (Right Command)");
                        CFRunLoopRun(); // blocks
                    }
                    return;
                }

                log::info!("CGEventTap unavailable (Input Monitoring not granted?), retrying in 2s…");
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }
}

// ── Windows: Low-level keyboard hook ─────────────────────────────────────────

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    const VK_RCONTROL: u32 = 0xA3;

    static KEY_DOWN: AtomicBool = AtomicBool::new(false);
    static KEY_TIME: AtomicU64 = AtomicU64::new(0);
    static OTHER_KEY: AtomicBool = AtomicBool::new(false);
    static HOOK: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

    unsafe extern "system" fn hook_proc(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
        if code >= 0 {
            let kbd = *(l_param as *const KBDLLHOOKSTRUCT);
            let vk = kbd.vkCode;
            let is_down = w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
            let is_up = w_param == WM_KEYUP as usize || w_param == WM_SYSKEYUP as usize;

            if vk == VK_RCONTROL {
                if is_down && !KEY_DOWN.load(Ordering::SeqCst) {
                    KEY_DOWN.store(true, Ordering::SeqCst);
                    KEY_TIME.store(now_ms(), Ordering::SeqCst);
                    OTHER_KEY.store(false, Ordering::SeqCst);
                } else if is_up && KEY_DOWN.swap(false, Ordering::SeqCst) {
                    let held = now_ms().saturating_sub(KEY_TIME.load(Ordering::SeqCst));
                    if !OTHER_KEY.load(Ordering::SeqCst) && held < SOLO_TAP_MAX_MS {
                        trigger_callback();
                    }
                }
            } else if is_down && KEY_DOWN.load(Ordering::SeqCst) {
                OTHER_KEY.store(true, Ordering::SeqCst);
            }
        }

        let h = HOOK.load(Ordering::SeqCst);
        unsafe { CallNextHookEx(h, code, w_param, l_param) }
    }

    pub fn start() {
        std::thread::spawn(|| unsafe {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), 0, 0);
            if hook == 0 {
                log::error!("Failed to install keyboard hook");
                return;
            }
            HOOK.store(hook, Ordering::SeqCst);
            log::info!("Native hotkey started (Right Control)");

            // Message pump — required for low-level keyboard hook to receive events.
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {}
        });
    }
}

// ── Linux: no-op (use global_shortcut fallback) ──────────────────────────────

#[cfg(target_os = "linux")]
mod platform {
    pub fn start() {
        log::info!("Native hotkey not available on Linux; use global shortcut");
    }
}
