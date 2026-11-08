use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU8, AtomicU32, Ordering};
use std::process::Command;
use std::sync::OnceLock;

use crate::hidmap::{self, Mapping};
use crate::lock;
use crate::state::{Action, CAPS_HOLD_THRESHOLD_SECS, InputEvent, KEY_CAPS_PROXY, State};
use crate::text_input::{self, FocusSnapshot};

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

type AXUIElementRef = *mut c_void;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFTypeID = usize;
type CFStringEncoding = u32;
type AXError = i32;
type AXCopyMultipleAttributeOptions = u32;
type Boolean = u8;
type PidT = i32;

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

// POSIX Signals & access(2)
const SIGHUP: c_int = 1;
const SIGINT: c_int = 2;
const SIGTERM: c_int = 15;
const F_OK: c_int = 0;

// Accessibility & CoreFoundation constants
const K_AX_ERROR_SUCCESS: AXError = 0;
const K_AX_ERROR_CANNOT_COMPLETE: AXError = -25204;
const K_AX_ERROR_ATTRIBUTE_UNSUPPORTED: AXError = -25205;
const K_AX_ERROR_API_DISABLED: AXError = -25211;
const K_AX_ERROR_NO_VALUE: AXError = -25212;

const K_CF_STRING_ENCODING_UTF8: CFStringEncoding = 0x0800_0100;
/// 個別 Accessibility IPC タイムアウト（10ms）
const AX_MESSAGING_TIMEOUT_SECS: f32 = 0.01;
/// 新規 IPC 開始の打ち切り期限（30ms。最後のIPC最大10msと合わせて最悪所要時間は最大40ms）
const AX_START_DEADLINE_SECS: f64 = 0.03;
/// proc_pidpath 用バッファサイズ
/// ※ Darwin の PROC_PIDPATHINFO_SIZE (MAXPATHLEN = 1024) が下限（これ未満は ENOMEM で失敗する）。
const PATH_BUF_SIZE: usize = 1024;
/// CFString 読み取り用スタックバッファサイズ（128バイト以上の文字列は None 扱い）
const ATTR_STR_BUF_SIZE: usize = 128;
/// Electron Framework 相対パス（NUL終端付き）
const ELECTRON_FRAMEWORK_SUFFIX: &[u8] = b"/Contents/Frameworks/Electron Framework.framework\0";

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

    fn AXIsProcessTrusted() -> Boolean;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout_in_seconds: f32) -> AXError;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementCopyMultipleAttributeValues(
        element: AXUIElementRef,
        attributes: CFArrayRef,
        options: AXCopyMultipleAttributeOptions,
        values: *mut CFArrayRef,
    ) -> AXError;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: CFStringRef,
        settable: *mut Boolean,
    ) -> AXError;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut PidT) -> AXError;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFAllocatorDefault: CFAllocatorRef;
    static kCFAllocatorNull: CFAllocatorRef;
    static kCFRunLoopCommonModes: CFRunLoopMode;
    static kCFTypeArrayCallBacks: c_void;

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

    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFStringCreateWithBytesNoCopy(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        num_bytes: CFIndex,
        encoding: CFStringEncoding,
        is_external_representation: Boolean,
        contents_deallocator: CFAllocatorRef,
    ) -> CFStringRef;
    fn CFStringGetCString(
        the_string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: CFStringEncoding,
    ) -> Boolean;
    fn CFArrayCreate(
        allocator: CFAllocatorRef,
        values: *const *const c_void,
        num_values: CFIndex,
        call_backs: *const c_void,
    ) -> CFArrayRef;
    fn CFArrayGetCount(the_array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(the_array: CFArrayRef, idx: CFIndex) -> *const c_void;
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
    fn proc_pidpath(pid: PidT, buffer: *mut c_void, buffersize: u32) -> c_int;
    fn access(path: *const c_char, amode: c_int) -> c_int;
}

static STATE: AtomicU8 = AtomicU8::new(State::Idle as u8);
static EVENT_TAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static RUN_LOOP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static CAPS_TIMER: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static HID_CONNECT: AtomicU32 = AtomicU32::new(0);
static CAPS_LOCK_ACTIVE: AtomicBool = AtomicBool::new(false);

// Accessibility 用のグローバルリソース（起動時に一括初期化し、終了時に解放）
static AX_SYSTEM_WIDE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static AX_ATTR_FOCUSED_UI_ELEMENT: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static AX_ATTR_FOCUSED_APPLICATION: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static AX_ATTR_ROLE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static AX_ATTR_SUBROLE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static AX_ATTR_VALUE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static AX_ATTR_SELECTED_TEXT_RANGE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static AX_ROLE_SUBROLE_ARRAY: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

// 直近の PID に対する Electron 判定キャッシュ
static LAST_ELECTRON_PID: AtomicI32 = AtomicI32::new(0);
static LAST_ELECTRON_RESULT: AtomicBool = AtomicBool::new(false);

/// 終了時（正常終了・panic）に `hidutil` へ書き戻す元の `UserKeyMapping`（JSON）。
/// リマップを有効化した時点でのみ設定される。
static RESTORE_PAYLOAD: OnceLock<String> = OnceLock::new();

unsafe fn create_static_cfstring(bytes: &'static [u8]) -> CFStringRef {
    unsafe {
        CFStringCreateWithBytesNoCopy(
            kCFAllocatorDefault,
            bytes.as_ptr(),
            bytes.len() as CFIndex,
            K_CF_STRING_ENCODING_UTF8,
            0,
            kCFAllocatorNull,
        )
    }
}

unsafe fn init_ax_resources() {
    if unsafe { AXIsProcessTrusted() } == 0 {
        eprintln!(
            "m2us warning: Accessibility permission is not granted; falling back to always sending EISU/KANA on Command tap."
        );
    }

    let sys_wide = unsafe { AXUIElementCreateSystemWide() };
    if !sys_wide.is_null() {
        let _ = unsafe { AXUIElementSetMessagingTimeout(sys_wide, AX_MESSAGING_TIMEOUT_SECS) };
        AX_SYSTEM_WIDE.store(sys_wide, Ordering::Relaxed);
    }

    let focused_ui = unsafe { create_static_cfstring(b"AXFocusedUIElement") };
    let focused_app = unsafe { create_static_cfstring(b"AXFocusedApplication") };
    let role = unsafe { create_static_cfstring(b"AXRole") };
    let subrole = unsafe { create_static_cfstring(b"AXSubrole") };
    let value = unsafe { create_static_cfstring(b"AXValue") };
    let selected_range = unsafe { create_static_cfstring(b"AXSelectedTextRange") };

    AX_ATTR_FOCUSED_UI_ELEMENT.store(focused_ui.cast_mut(), Ordering::Relaxed);
    AX_ATTR_FOCUSED_APPLICATION.store(focused_app.cast_mut(), Ordering::Relaxed);
    AX_ATTR_ROLE.store(role.cast_mut(), Ordering::Relaxed);
    AX_ATTR_SUBROLE.store(subrole.cast_mut(), Ordering::Relaxed);
    AX_ATTR_VALUE.store(value.cast_mut(), Ordering::Relaxed);
    AX_ATTR_SELECTED_TEXT_RANGE.store(selected_range.cast_mut(), Ordering::Relaxed);

    let attrs: [*const c_void; 2] = [role, subrole];
    let arr = unsafe {
        CFArrayCreate(
            kCFAllocatorDefault,
            attrs.as_ptr(),
            attrs.len() as CFIndex,
            &raw const kCFTypeArrayCallBacks,
        )
    };
    AX_ROLE_SUBROLE_ARRAY.store(arr.cast_mut(), Ordering::Relaxed);
}

unsafe fn release_atomic_cf(slot: &AtomicPtr<c_void>) {
    let ptr = slot.swap(ptr::null_mut(), Ordering::Relaxed);
    if !ptr.is_null() {
        unsafe { CFRelease(ptr) };
    }
}

unsafe fn cleanup_ax_resources() {
    unsafe {
        release_atomic_cf(&AX_ROLE_SUBROLE_ARRAY);
        release_atomic_cf(&AX_ATTR_SELECTED_TEXT_RANGE);
        release_atomic_cf(&AX_ATTR_VALUE);
        release_atomic_cf(&AX_ATTR_SUBROLE);
        release_atomic_cf(&AX_ATTR_ROLE);
        release_atomic_cf(&AX_ATTR_FOCUSED_APPLICATION);
        release_atomic_cf(&AX_ATTR_FOCUSED_UI_ELEMENT);
        release_atomic_cf(&AX_SYSTEM_WIDE);
    }
}

unsafe fn cfstring_to_stack_str(cf: CFTypeRef, buf: &mut [u8; ATTR_STR_BUF_SIZE]) -> Option<&str> {
    if cf.is_null() {
        return None;
    }
    if unsafe { CFGetTypeID(cf) != CFStringGetTypeID() } {
        return None;
    }
    let ok = unsafe {
        CFStringGetCString(
            cf,
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len() as CFIndex,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    core::str::from_utf8(&buf[..len]).ok()
}

unsafe fn check_is_electron(pid: PidT, executable_path: &str) -> bool {
    if pid <= 0 {
        return false;
    }
    if LAST_ELECTRON_PID.load(Ordering::Relaxed) == pid {
        return LAST_ELECTRON_RESULT.load(Ordering::Relaxed);
    }

    let is_electron = if let Some(bundle_root) =
        text_input::bundle_root_from_executable(executable_path)
    {
        let root_bytes = bundle_root.as_bytes();
        let total_len = root_bytes.len() + ELECTRON_FRAMEWORK_SUFFIX.len();
        if total_len <= PATH_BUF_SIZE {
            let mut framework_buf = [0u8; PATH_BUF_SIZE];
            framework_buf[..root_bytes.len()].copy_from_slice(root_bytes);
            framework_buf[root_bytes.len()..total_len].copy_from_slice(ELECTRON_FRAMEWORK_SUFFIX);
            unsafe { access(framework_buf.as_ptr().cast::<c_char>(), F_OK) == 0 }
        } else {
            false
        }
    } else {
        false
    };

    LAST_ELECTRON_PID.store(pid, Ordering::Relaxed);
    LAST_ELECTRON_RESULT.store(is_electron, Ordering::Relaxed);
    is_electron
}

unsafe fn resolve_app_info(
    elem: AXUIElementRef,
    path_buf: &mut [u8; PATH_BUF_SIZE],
) -> (Option<&str>, bool) {
    let mut pid: PidT = 0;
    if unsafe { AXUIElementGetPid(elem, &mut pid) } != K_AX_ERROR_SUCCESS || pid <= 0 {
        return (None, false);
    }
    let ret = unsafe {
        proc_pidpath(
            pid,
            path_buf.as_mut_ptr().cast::<c_void>(),
            path_buf.len() as u32,
        )
    };
    if ret <= 0 {
        return (None, false);
    }
    let len = (ret as usize).min(path_buf.len());
    let Ok(exec_path) = core::str::from_utf8(&path_buf[..len]) else {
        return (None, false);
    };
    let is_electron = unsafe { check_is_electron(pid, exec_path) };
    let app_id = text_input::extract_app_id(exec_path);
    (app_id, is_electron)
}

/// 属性が Settable かどうかを確認する。
/// タイムアウト (`kAXErrorCannotComplete`) または権限無効 (`kAXErrorAPIDisabled`) の場合は `Err(())` を返し、
/// 属性未定義 (`kAXErrorAttributeUnsupported` 等) の場合は `Ok(false)` を返す。
unsafe fn check_attribute_settable(elem: AXUIElementRef, attr: CFStringRef) -> Result<bool, ()> {
    let mut settable: Boolean = 0;
    let err = unsafe { AXUIElementIsAttributeSettable(elem, attr, &mut settable) };
    if err == K_AX_ERROR_SUCCESS {
        Ok(settable != 0)
    } else if err == K_AX_ERROR_CANNOT_COMPLETE || err == K_AX_ERROR_API_DISABLED {
        Err(())
    } else {
        Ok(false)
    }
}

/// 現在フォーカスされているUI要素がテキスト入力対象かどうかを判定する。
/// フェイルセーフ方針に基づき、タイムアウト・権限未付与・未知のAXエラー時はすべて `true`（送出）を返す。
unsafe fn is_text_input_focused() -> bool {
    let sys_wide = AX_SYSTEM_WIDE.load(Ordering::Relaxed);
    let focused_ui_attr = AX_ATTR_FOCUSED_UI_ELEMENT.load(Ordering::Relaxed);
    let role_subrole_array = AX_ROLE_SUBROLE_ARRAY.load(Ordering::Relaxed);
    if sys_wide.is_null() || focused_ui_attr.is_null() || role_subrole_array.is_null() {
        return true;
    }

    let start_time = unsafe { CFAbsoluteTimeGetCurrent() };

    // IPC #1: AXFocusedUIElement の取得
    let mut focused_elem: CFTypeRef = ptr::null();
    let err =
        unsafe { AXUIElementCopyAttributeValue(sys_wide, focused_ui_attr, &mut focused_elem) };

    if err == K_AX_ERROR_NO_VALUE || err == K_AX_ERROR_ATTRIBUTE_UNSUPPORTED {
        // フォーカスUI要素が存在しない場合（デスクトップや一部のGPUターミナル等）
        if unsafe { CFAbsoluteTimeGetCurrent() } - start_time >= AX_START_DEADLINE_SECS {
            return true;
        }
        let focused_app_attr = AX_ATTR_FOCUSED_APPLICATION.load(Ordering::Relaxed);
        if focused_app_attr.is_null() {
            return true;
        }
        // IPC #2 (代替): AXFocusedApplication の取得
        let mut app_elem: CFTypeRef = ptr::null();
        let app_err =
            unsafe { AXUIElementCopyAttributeValue(sys_wide, focused_app_attr, &mut app_elem) };
        if app_err == K_AX_ERROR_SUCCESS && !app_elem.is_null() {
            let mut path_buf = [0u8; PATH_BUF_SIZE];
            let (app_id, is_electron) =
                unsafe { resolve_app_info(app_elem.cast_mut(), &mut path_buf) };
            unsafe { CFRelease(app_elem) };
            let snapshot = FocusSnapshot {
                app_id,
                is_electron,
                ..Default::default()
            };
            return text_input::evaluate_focus(&snapshot);
        } else if app_err == K_AX_ERROR_NO_VALUE || app_err == K_AX_ERROR_ATTRIBUTE_UNSUPPORTED {
            return text_input::evaluate_focus(&FocusSnapshot::default());
        } else {
            // タイムアウト・権限エラー・その他未知のエラー時はフェイルセーフで true
            return true;
        }
    }

    if err != K_AX_ERROR_SUCCESS || focused_elem.is_null() {
        // kAXErrorCannotComplete (タイムアウト) / kAXErrorAPIDisabled (権限なし) / 未知のエラー時はフェイルセーフで true
        return true;
    }

    let elem = focused_elem.cast_mut();
    let mut path_buf = [0u8; PATH_BUF_SIZE];
    let (app_id, is_electron) = unsafe { resolve_app_info(elem, &mut path_buf) };

    if unsafe { CFAbsoluteTimeGetCurrent() } - start_time >= AX_START_DEADLINE_SECS {
        unsafe { CFRelease(focused_elem) };
        return true;
    }

    // IPC #2: AXRole と AXSubrole を 1 回の IPC で一括取得
    let mut values_array: CFArrayRef = ptr::null();
    let multi_err = unsafe {
        AXUIElementCopyMultipleAttributeValues(elem, role_subrole_array, 0, &mut values_array)
    };
    if multi_err != K_AX_ERROR_SUCCESS || values_array.is_null() {
        unsafe { CFRelease(focused_elem) };
        return true;
    }

    let mut role_buf = [0u8; ATTR_STR_BUF_SIZE];
    let mut subrole_buf = [0u8; ATTR_STR_BUF_SIZE];
    let count = unsafe { CFArrayGetCount(values_array) };
    let role = if count > 0 {
        let val = unsafe { CFArrayGetValueAtIndex(values_array, 0) };
        unsafe { cfstring_to_stack_str(val, &mut role_buf) }
    } else {
        None
    };
    let subrole = if count > 1 {
        let val = unsafe { CFArrayGetValueAtIndex(values_array, 1) };
        unsafe { cfstring_to_stack_str(val, &mut subrole_buf) }
    } else {
        None
    };
    unsafe { CFRelease(values_array) };

    // Subrole または Role で即座に true が確定する場合は後続の IPC (#3, #4) を省略
    if subrole.is_some_and(text_input::is_text_subrole)
        || matches!(role, Some("AXComboBox" | "AXSearchField"))
    {
        unsafe { CFRelease(focused_elem) };
        return true;
    }

    let mut value_settable = false;
    let mut selected_text_range_settable = false;

    if matches!(role, Some("AXTextField" | "AXTextArea")) {
        if unsafe { CFAbsoluteTimeGetCurrent() } - start_time >= AX_START_DEADLINE_SECS {
            unsafe { CFRelease(focused_elem) };
            return true;
        }
        // IPC #3: AXValue が Settable か確認
        match unsafe { check_attribute_settable(elem, AX_ATTR_VALUE.load(Ordering::Relaxed)) } {
            Ok(settable) => value_settable = settable,
            Err(()) => {
                unsafe { CFRelease(focused_elem) };
                return true;
            }
        }

        // value_settable が false の場合のみ IPC #4: AXSelectedTextRange の Settable を確認
        if !value_settable {
            if unsafe { CFAbsoluteTimeGetCurrent() } - start_time >= AX_START_DEADLINE_SECS {
                unsafe { CFRelease(focused_elem) };
                return true;
            }
            match unsafe {
                check_attribute_settable(elem, AX_ATTR_SELECTED_TEXT_RANGE.load(Ordering::Relaxed))
            } {
                Ok(settable) => selected_text_range_settable = settable,
                Err(()) => {
                    unsafe { CFRelease(focused_elem) };
                    return true;
                }
            }
        }
    } else if role.is_some_and(|r| !text_input::is_non_text_control_role(r)) {
        if unsafe { CFAbsoluteTimeGetCurrent() } - start_time >= AX_START_DEADLINE_SECS {
            unsafe { CFRelease(focused_elem) };
            return true;
        }
        // IPC #3 (最大): カスタムロール要素の AXSelectedTextRange が Settable か確認
        match unsafe {
            check_attribute_settable(elem, AX_ATTR_SELECTED_TEXT_RANGE.load(Ordering::Relaxed))
        } {
            Ok(settable) => selected_text_range_settable = settable,
            Err(()) => {
                unsafe { CFRelease(focused_elem) };
                return true;
            }
        }
    }

    unsafe { CFRelease(focused_elem) };

    let snapshot = FocusSnapshot {
        role,
        subrole,
        value_settable,
        selected_text_range_settable,
        app_id,
        is_electron,
    };
    text_input::evaluate_focus(&snapshot)
}

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
        Action::SendImeKey(target_key) => unsafe {
            cancel_caps_timer();
            if is_text_input_focused() {
                send_key(target_key);
            }
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

const HIDUTIL_PATH: &str = "/usr/bin/hidutil";

fn hidutil_set(payload: &str) {
    let _ = Command::new(HIDUTIL_PATH)
        .args(["property", "--set", payload])
        .status();
}

/// 現在の `UserKeyMapping` を取得する。取得・解釈に失敗した場合は `None`。
fn read_user_key_mapping() -> Option<Vec<Mapping>> {
    let out = Command::new(HIDUTIL_PATH)
        .args(["property", "--get", "UserKeyMapping"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    hidmap::parse_user_key_mapping(&String::from_utf8_lossy(&out.stdout))
}

/// 既存のリマップを保持したまま CapsLock→F18 を追加する。
/// 前回異常終了して残った自前エントリは「元の設定」から除外されるため、
/// 再起動するだけで正常な状態に戻る。
fn enable_hidutil() {
    let original = match read_user_key_mapping() {
        Some(current) => hidmap::without_own(&current),
        None => {
            eprintln!(
                "m2us warning: could not read the current UserKeyMapping; existing key remaps may be lost."
            );
            Vec::new()
        }
    };
    let _ = RESTORE_PAYLOAD.set(hidmap::set_payload(&original, false));
    hidutil_set(&hidmap::set_payload(&hidmap::with_own(&original), true));
}

/// 起動前の `UserKeyMapping` を書き戻す（有効化していない場合は何もしない）。
fn restore_hidutil() {
    if let Some(payload) = RESTORE_PAYLOAD.get() {
        hidutil_set(payload);
    }
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
    // 他のインスタンスが動作中の場合は、何も変更せずに終了する。
    let _instance_lock = lock::acquire(&lock::default_path())?;

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

    unsafe { init_ax_resources() };

    enable_hidutil();

    // panic = "abort" でもリマップが残らないよう、abort前に元の設定へ戻す。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_hidutil();
        default_hook(info);
    }));

    unsafe {
        let run_loop = CFRunLoopGetCurrent();
        RUN_LOOP.store(run_loop, Ordering::Relaxed);
        signal(SIGHUP, handle_signal);
        signal(SIGINT, handle_signal);
        signal(SIGTERM, handle_signal);

        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRunLoopRun();

        cancel_caps_timer();
        cleanup_ax_resources();
        CFRelease(source);
        CFRelease(tap);
        if hid_connect != 0 {
            IOServiceClose(hid_connect);
        }
    }

    restore_hidutil();

    Ok(())
}
