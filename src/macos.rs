use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

use crate::state::{Action, InputEvent, State};

type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFAllocatorRef = *const c_void;
type CFRunLoopMode = *const c_void;
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
const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 0x0000_0001;

// CGEventType
const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const K_CG_EVENT_KEY_DOWN: u32 = 10;
const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
const K_CG_EVENT_SCROLL_WHEEL: u32 = 22;
const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

// CGEventField
const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;

// QoS Class (sys/qos.h)
const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

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
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFRunLoopMode);
    fn CFRunLoopRun();
    fn CFRelease(cf: *const c_void);
}

unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

static STATE: AtomicU8 = AtomicU8::new(State::Idle as u8);
static EVENT_TAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

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

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    if event_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || event_type == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        STATE.store(State::Idle as u8, Ordering::Relaxed);
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
        if let Action::SendKey(target_key) = action {
            unsafe { send_key(target_key) };
        }
    } else {
        // 通常キー押下、マウスクリック、スクロール等
        let current_state = State::from_u8(STATE.load(Ordering::Relaxed));
        let (next_state, _) = current_state.process_event(InputEvent::OtherInput);
        STATE.store(next_state as u8, Ordering::Relaxed);
    }

    event
}

pub fn run() -> Result<(), &'static str> {
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
    }

    let mask: CGEventMask = (1 << K_CG_EVENT_FLAGS_CHANGED)
        | (1 << K_CG_EVENT_KEY_DOWN)
        | (1 << K_CG_EVENT_LEFT_MOUSE_DOWN)
        | (1 << K_CG_EVENT_RIGHT_MOUSE_DOWN)
        | (1 << K_CG_EVENT_OTHER_MOUSE_DOWN)
        | (1 << K_CG_EVENT_SCROLL_WHEEL);

    let tap = unsafe {
        CGEventTapCreate(
            K_CG_HID_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            mask,
            event_tap_callback,
            ptr::null_mut(),
        )
    };

    if tap.is_null() {
        return Err(
            "Failed to create CGEventTap. Please grant Input Monitoring / Accessibility permissions.",
        );
    }

    EVENT_TAP.store(tap, Ordering::Relaxed);

    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
    if source.is_null() {
        unsafe { CFRelease(tap) };
        return Err("Failed to create CFRunLoopSource.");
    }

    unsafe {
        let run_loop = CFRunLoopGetCurrent();
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRunLoopRun();
    }

    Ok(())
}
