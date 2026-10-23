use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, Ordering};
use std::process::Command;

use crate::state::{Action, CAPS_HOLD_THRESHOLD_SECS, InputEvent, KEY_CAPS_PROXY, State};

type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopTimerRef = *mut c_void;
type CFAllocatorRef = *const c_void;
type CFRunLoopMode = *const c_void;
type CFAbsoluteTime = f64;
type CFTimeInterval = f64;
type CFIndex = isize;
type CFOptionFlags = usize;
type CGEventTapProxy = *mut c_void;
type CGEventRef = *mut c_void;
type CGEventSourceRef = *mut c_void;
type CGKeyCode = u16;
type CGEventFlags = u64;
type CGEventMask = u64;

// CGEventTapLocation
const K_CG_HID_EVENT_TAP: u32 = 0;

// CGEventTapPlacement
const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;

// CGEventTapOptions
const K_CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0x0000_0000;

// CGEventType
const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const K_CG_EVENT_KEY_DOWN: u32 = 10;
const K_CG_EVENT_KEY_UP: u32 = 11;
const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
const K_CG_EVENT_SCROLL_WHEEL: u32 = 22;
const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

// CGEventField
const K_CG_KEYBOARD_EVENT_AUTOREPEAT: u32 = 8;
const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;

// QoS Class (sys/qos.h)
const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;

// IOKit constants
const K_IO_HID_PARAM_CONNECT_TYPE: u32 = 1;
const K_IO_HID_CAPS_LOCK_STATE: i32 = 1;
const K_IO_HID_SYSTEM_CLASS: &core::ffi::CStr = c"IOHIDSystem";

// POSIX Signals
const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

type CFRunLoopTimerCallBack = unsafe extern "C" fn(timer: CFRunLoopTimerRef, info: *mut c_void);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;

    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: CGKeyCode,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
    fn CGEventPost(tap: u32, event: CGEventRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CFRunLoopMode;

    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: CFIndex,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFRunLoopMode);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRelease(cf: *const c_void);

    fn CFAbsoluteTimeGetCurrent() -> CFAbsoluteTime;
    fn CFRunLoopTimerCreate(
        allocator: CFAllocatorRef,
        fire_date: CFAbsoluteTime,
        interval: CFTimeInterval,
        flags: CFOptionFlags,
        order: CFIndex,
        callout: CFRunLoopTimerCallBack,
        context: *mut c_void,
    ) -> CFRunLoopTimerRef;
    fn CFRunLoopAddTimer(rl: CFRunLoopRef, timer: CFRunLoopTimerRef, mode: CFRunLoopMode);
    fn CFRunLoopTimerInvalidate(timer: CFRunLoopTimerRef);
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
    fn IOServiceOpen(service: u32, owning_task: u32, connect_type: u32, connect: *mut u32) -> i32;
    fn IOServiceClose(connect: u32) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOHIDGetModifierLockState(handle: u32, selector: i32, state: *mut bool) -> i32;
    fn IOHIDSetModifierLockState(handle: u32, selector: i32, state: bool) -> i32;
}

unsafe extern "C" {
    static mach_task_self_: u32;
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    fn signal(sig: c_int, handler: unsafe extern "C" fn(c_int)) -> usize;
}

static STATE: AtomicU8 = AtomicU8::new(State::Idle as u8);
static EVENT_TAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static RUN_LOOP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static CAPS_TIMER: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static HID_CONNECT: AtomicU32 = AtomicU32::new(0);
static CAPS_LOCK_ACTIVE: AtomicBool = AtomicBool::new(false);

unsafe fn send_key(keycode: CGKeyCode) {
    let down = unsafe { CGEventCreateKeyboardEvent(ptr::null_mut(), keycode, true) };
    if !down.is_null() {
        unsafe {
            CGEventSetFlags(down, 0);
            CGEventPost(K_CG_HID_EVENT_TAP, down);
            CFRelease(down);
        }
    }
    let up = unsafe { CGEventCreateKeyboardEvent(ptr::null_mut(), keycode, false) };
    if !up.is_null() {
        unsafe {
            CGEventSetFlags(up, 0);
            CGEventPost(K_CG_HID_EVENT_TAP, up);
            CFRelease(up);
        }
    }
}

unsafe fn cancel_caps_timer() {
    let old = CAPS_TIMER.swap(ptr::null_mut(), Ordering::Relaxed);
    if !old.is_null() {
        unsafe {
            CFRunLoopTimerInvalidate(old);
            CFRelease(old);
        }
    }
}

unsafe fn start_caps_timer() {
    unsafe { cancel_caps_timer() };
    let rl = RUN_LOOP.load(Ordering::Relaxed);
    if rl.is_null() {
        return;
    }
    let fire_date = unsafe { CFAbsoluteTimeGetCurrent() } + CAPS_HOLD_THRESHOLD_SECS;
    let timer = unsafe {
        CFRunLoopTimerCreate(
            ptr::null(),
            fire_date,
            0.0,
            0,
            0,
            caps_timer_callback,
            ptr::null_mut(),
        )
    };
    if !timer.is_null() {
        CAPS_TIMER.store(timer, Ordering::Relaxed);
        unsafe {
            CFRunLoopAddTimer(rl, timer, kCFRunLoopCommonModes);
        }
    }
}

unsafe fn toggle_caps_lock() {
    let connect = HID_CONNECT.load(Ordering::Relaxed);
    if connect == 0 {
        return;
    }
    let next = !CAPS_LOCK_ACTIVE.load(Ordering::Relaxed);
    CAPS_LOCK_ACTIVE.store(next, Ordering::Relaxed);
    unsafe {
        IOHIDSetModifierLockState(connect, K_IO_HID_CAPS_LOCK_STATE, next);
    }
}

unsafe fn execute_action(action: Action) {
    match action {
        Action::None => {}
        Action::SendKey(target_key) => unsafe {
            cancel_caps_timer();
            send_key(target_key);
        },
        Action::StartCapsTimer => unsafe {
            start_caps_timer();
        },
        Action::CancelCapsTimer => unsafe {
            cancel_caps_timer();
        },
        Action::ToggleCapsLock => unsafe {
            toggle_caps_lock();
        },
    }
}

unsafe extern "C" fn caps_timer_callback(_timer: CFRunLoopTimerRef, _info: *mut c_void) {
    let old = CAPS_TIMER.swap(ptr::null_mut(), Ordering::Relaxed);
    if !old.is_null() {
        unsafe {
            CFRunLoopTimerInvalidate(old);
            CFRelease(old);
        }
    }

    let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
    let (next_state, action) = current_state.process_event(InputEvent::CapsHoldTimeout);
    STATE.store(next_state as u8, Ordering::Relaxed);
    unsafe { execute_action(action) };
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    if event_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || event_type == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
        let (next_state, action) = current_state.process_event(InputEvent::Reset);
        STATE.store(next_state as u8, Ordering::Relaxed);
        unsafe { execute_action(action) };

        let tap = EVENT_TAP.load(Ordering::Relaxed);
        if !tap.is_null() {
            unsafe { CGEventTapEnable(tap, true) };
        }
        return event;
    }

    if event_type == K_CG_EVENT_FLAGS_CHANGED {
        let keycode =
            unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) } as u16;
        let flags = unsafe { CGEventGetFlags(event) };
        let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
        let (next_state, action) =
            current_state.process_event(InputEvent::FlagsChanged { keycode, flags });
        STATE.store(next_state as u8, Ordering::Relaxed);
        unsafe { execute_action(action) };
    } else if event_type == K_CG_EVENT_KEY_DOWN {
        let keycode =
            unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) } as u16;
        if keycode == KEY_CAPS_PROXY {
            let is_repeat =
                unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_AUTOREPEAT) } != 0;
            let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
            let (next_state, action) =
                current_state.process_event(InputEvent::CapsDown { is_repeat });
            STATE.store(next_state as u8, Ordering::Relaxed);
            unsafe { execute_action(action) };
            return ptr::null_mut();
        }

        let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
        let (next_state, action) = current_state.process_event(InputEvent::OtherInput);
        STATE.store(next_state as u8, Ordering::Relaxed);
        unsafe { execute_action(action) };
    } else if event_type == K_CG_EVENT_KEY_UP {
        let keycode =
            unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) } as u16;
        if keycode == KEY_CAPS_PROXY {
            let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
            let (next_state, action) = current_state.process_event(InputEvent::CapsUp);
            STATE.store(next_state as u8, Ordering::Relaxed);
            unsafe { execute_action(action) };
            return ptr::null_mut();
        }
    } else {
        // マウスクリック、スクロール等
        let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
        let (next_state, action) = current_state.process_event(InputEvent::OtherInput);
        STATE.store(next_state as u8, Ordering::Relaxed);
        unsafe { execute_action(action) };
    }

    event
}

fn configure_hidutil(enable: bool) {
    let payload = if enable {
        r#"{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}],"CapsLockDelayOverride":0}"#
    } else {
        r#"{"UserKeyMapping":[]}"#
    };
    let _ = Command::new("/usr/bin/hidutil")
        .args(["property", "--set", payload])
        .status();
}

unsafe fn open_iohid_connect() -> u32 {
    let matching = unsafe { IOServiceMatching(K_IO_HID_SYSTEM_CLASS.as_ptr()) };
    if matching.is_null() {
        return 0;
    }
    let service = unsafe { IOServiceGetMatchingService(0, matching) };
    if service == 0 {
        return 0;
    }
    let mut connect: u32 = 0;
    let kr = unsafe {
        IOServiceOpen(
            service,
            mach_task_self_,
            K_IO_HID_PARAM_CONNECT_TYPE,
            &mut connect,
        )
    };
    unsafe { IOObjectRelease(service) };
    if kr == 0 { connect } else { 0 }
}

unsafe extern "C" fn handle_signal(_sig: c_int) {
    let rl = RUN_LOOP.load(Ordering::Relaxed);
    if !rl.is_null() {
        unsafe { CFRunLoopStop(rl) };
    }
}

pub fn run() -> Result<(), &'static str> {
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
    }

    let hid_connect = unsafe { open_iohid_connect() };
    HID_CONNECT.store(hid_connect, Ordering::Relaxed);
    if hid_connect != 0 {
        let mut init_state = false;
        let _ = unsafe {
            IOHIDGetModifierLockState(hid_connect, K_IO_HID_CAPS_LOCK_STATE, &mut init_state)
        };
        CAPS_LOCK_ACTIVE.store(init_state, Ordering::Relaxed);
    }

    let mask: CGEventMask = (1 << K_CG_EVENT_FLAGS_CHANGED)
        | (1 << K_CG_EVENT_KEY_DOWN)
        | (1 << K_CG_EVENT_KEY_UP)
        | (1 << K_CG_EVENT_LEFT_MOUSE_DOWN)
        | (1 << K_CG_EVENT_RIGHT_MOUSE_DOWN)
        | (1 << K_CG_EVENT_OTHER_MOUSE_DOWN)
        | (1 << K_CG_EVENT_SCROLL_WHEEL);

    let tap = unsafe {
        CGEventTapCreate(
            K_CG_HID_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_DEFAULT,
            mask,
            event_tap_callback,
            ptr::null_mut(),
        )
    };

    if tap.is_null() {
        if hid_connect != 0 {
            unsafe { IOServiceClose(hid_connect) };
        }
        return Err(
            "Failed to create CGEventTap. Please grant Input Monitoring / Accessibility permissions.",
        );
    }

    EVENT_TAP.store(tap, Ordering::Relaxed);

    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
    if source.is_null() {
        unsafe { CFRelease(tap) };
        if hid_connect != 0 {
            unsafe { IOServiceClose(hid_connect) };
        }
        return Err("Failed to create CFRunLoopSource.");
    }

    configure_hidutil(true);

    unsafe {
        let run_loop = CFRunLoopGetCurrent();
        RUN_LOOP.store(run_loop, Ordering::Relaxed);
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);

        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRunLoopRun();

        cancel_caps_timer();
        CFRelease(source);
        CFRelease(tap);
        if hid_connect != 0 {
            IOServiceClose(hid_connect);
        }
    }

    configure_hidutil(false);

    Ok(())
}
