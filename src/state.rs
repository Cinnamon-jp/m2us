/// macOS Virtual KeyCodes
pub const KEY_LEFT_COMMAND: u16 = 0x37; // 55: kVK_Command
pub const KEY_RIGHT_COMMAND: u16 = 0x36; // 54: kVK_RightCommand
pub const KEY_JIS_EISU: u16 = 0x66; // 102: kVK_JIS_Eisu (英数)
pub const KEY_JIS_KANA: u16 = 0x68; // 104: kVK_JIS_Kana (かな)
pub const KEY_ESCAPE: u16 = 0x35; // 53: kVK_Escape (Esc)
pub const KEY_CAPS_PROXY: u16 = 0x4F; // 79: kVK_F18 (hidutilにより物理CapsLockからリマップされたキー)

/// CapsLock長押し判定しきい値（秒）
pub const CAPS_HOLD_THRESHOLD_SECS: f64 = 0.25; // 250ms

/// CGEventFlags bitmasks
pub const FLAG_MASK_ALPHA_SHIFT: u64 = 0x0001_0000; // kCGEventFlagMaskAlphaShift (CapsLock ON状態)
pub const FLAG_MASK_COMMAND: u64 = 0x0010_0000; // kCGEventFlagMaskCommand
/// Command以外の押下修飾キー（Shift, Control, Option）のマスク
/// ※ AlphaShift(0x0001_0000)はCapsLockロック中もCommandタップを有効にするため除外
/// Shift(0x0002_0000) | Control(0x0004_0000) | Alternate/Option(0x0008_0000)
pub const FLAG_MASK_OTHER_MODIFIERS: u64 = 0x0002_0000 | 0x0004_0000 | 0x0008_0000;

/// Device-dependent Command key flags (IOKit IOLLEvent.h NX_DEVICELCMDKEYMASK / NX_DEVICERCMDKEYMASK)
/// 左右Commandの同時押しを厳密に区別するために利用可能（下位ビット）
pub const FLAG_DEV_LEFT_CMD: u64 = 0x0000_0008;
pub const FLAG_DEV_RIGHT_CMD: u64 = 0x0000_0010;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum State {
    #[default]
    Idle = 0,
    PendingLeftCmd = 1,
    PendingRightCmd = 2,
    PendingCaps = 3,
}

impl State {
    #[inline]
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => State::PendingLeftCmd,
            2 => State::PendingRightCmd,
            3 => State::PendingCaps,
            _ => State::Idle,
        }
    }

    pub fn process_event(self, event: InputEvent) -> (Self, Action) {
        match event {
            InputEvent::OtherInput | InputEvent::Reset => {
                if self == State::PendingCaps {
                    (State::Idle, Action::CancelCapsTimer)
                } else {
                    (State::Idle, Action::None)
                }
            }
            InputEvent::CapsDown { is_repeat } => {
                if is_repeat {
                    (self, Action::None)
                } else if self == State::Idle {
                    (State::PendingCaps, Action::StartCapsTimer)
                } else {
                    (State::Idle, Action::CancelCapsTimer)
                }
            }
            InputEvent::CapsUp => {
                if self == State::PendingCaps {
                    (State::Idle, Action::SendKey(KEY_ESCAPE))
                } else {
                    (State::Idle, Action::None)
                }
            }
            InputEvent::CapsHoldTimeout => {
                if self == State::PendingCaps {
                    (State::Idle, Action::ToggleCapsLock)
                } else {
                    (self, Action::None)
                }
            }
            InputEvent::FlagsChanged { keycode, flags } => {
                if self == State::PendingCaps {
                    return (State::Idle, Action::CancelCapsTimer);
                }

                // 他の修飾キー（Shift, Control, Option）が押されている場合はキャンセル
                if (flags & FLAG_MASK_OTHER_MODIFIERS) != 0 {
                    return (State::Idle, Action::None);
                }

                // 左右両方のCommandキーが同時に押されている場合はキャンセル
                if (flags & (FLAG_DEV_LEFT_CMD | FLAG_DEV_RIGHT_CMD))
                    == (FLAG_DEV_LEFT_CMD | FLAG_DEV_RIGHT_CMD)
                {
                    return (State::Idle, Action::None);
                }

                match keycode {
                    KEY_LEFT_COMMAND => {
                        let is_pressed = (flags & FLAG_MASK_COMMAND) != 0
                            && (flags & FLAG_DEV_LEFT_CMD != 0
                                || (flags & FLAG_DEV_RIGHT_CMD) == 0);

                        if is_pressed {
                            if self == State::Idle && (flags & FLAG_DEV_RIGHT_CMD) == 0 {
                                (State::PendingLeftCmd, Action::None)
                            } else {
                                (State::Idle, Action::None)
                            }
                        } else if self == State::PendingLeftCmd && (flags & FLAG_MASK_COMMAND) == 0
                        {
                            (State::Idle, Action::SendKey(KEY_JIS_EISU))
                        } else {
                            (State::Idle, Action::None)
                        }
                    }
                    KEY_RIGHT_COMMAND => {
                        let is_pressed = (flags & FLAG_MASK_COMMAND) != 0
                            && (flags & FLAG_DEV_RIGHT_CMD != 0
                                || (flags & FLAG_DEV_LEFT_CMD) == 0);

                        if is_pressed {
                            if self == State::Idle && (flags & FLAG_DEV_LEFT_CMD) == 0 {
                                (State::PendingRightCmd, Action::None)
                            } else {
                                (State::Idle, Action::None)
                            }
                        } else if self == State::PendingRightCmd && (flags & FLAG_MASK_COMMAND) == 0
                        {
                            (State::Idle, Action::SendKey(KEY_JIS_KANA))
                        } else {
                            (State::Idle, Action::None)
                        }
                    }
                    _ => (State::Idle, Action::None),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// kCGEventFlagsChanged: keycode と イベント発生後の flags
    FlagsChanged { keycode: u16, flags: u64 },
    /// CapsLock (KEY_CAPS_PROXY) 押下（is_repeat: キーリピートかどうか）
    CapsDown { is_repeat: bool },
    /// CapsLock (KEY_CAPS_PROXY) 解放
    CapsUp,
    /// CapsLock 長押しタイマー（250ms）満了
    CapsHoldTimeout,
    /// 通常キー押下、マウスクリック、スクロールなど空打ちをキャンセルすべきイベント
    OtherInput,
    /// タップタイムアウト等によるリセット
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    SendKey(u16), // KEY_JIS_EISU, KEY_JIS_KANA, または KEY_ESCAPE
    StartCapsTimer,
    CancelCapsTimer,
    ToggleCapsLock,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_u8() {
        assert_eq!(State::from_u8(0), State::Idle);
        assert_eq!(State::from_u8(1), State::PendingLeftCmd);
        assert_eq!(State::from_u8(2), State::PendingRightCmd);
        assert_eq!(State::from_u8(3), State::PendingCaps);
        assert_eq!(State::from_u8(99), State::Idle);
    }

    #[test]
    fn test_left_command_tap() {
        let mut state = State::Idle;

        // 左Command押下
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_COMMAND | FLAG_DEV_LEFT_CMD,
        });
        assert_eq!(s, State::PendingLeftCmd);
        assert_eq!(act, Action::None);
        state = s;

        // 左Command解放
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: 0,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::SendKey(KEY_JIS_EISU));
    }

    #[test]
    fn test_right_command_tap() {
        let mut state = State::Idle;

        // 右Command押下
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: FLAG_MASK_COMMAND | FLAG_DEV_RIGHT_CMD,
        });
        assert_eq!(s, State::PendingRightCmd);
        assert_eq!(act, Action::None);
        state = s;

        // 右Command解放
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: 0,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::SendKey(KEY_JIS_KANA));
    }

    #[test]
    fn test_left_command_cancelled_by_other_input() {
        let mut state = State::Idle;

        // 左Command押下
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_COMMAND,
        });
        assert_eq!(s, State::PendingLeftCmd);
        assert_eq!(act, Action::None);
        state = s;

        // 他キー押下（Cmd + C等）またはマウスクリック
        let (s, act) = state.process_event(InputEvent::OtherInput);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
        state = s;

        // 左Command解放
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: 0,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_right_command_cancelled_by_other_input() {
        let mut state = State::Idle;

        // 右Command押下
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: FLAG_MASK_COMMAND,
        });
        assert_eq!(s, State::PendingRightCmd);
        assert_eq!(act, Action::None);
        state = s;

        // マウスホイールや通常キー押下
        let (s, act) = state.process_event(InputEvent::OtherInput);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
        state = s;

        // 右Command解放
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: 0,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_command_cancelled_by_other_modifier() {
        let mut state = State::Idle;

        // 左Command押下
        let (s, _) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_COMMAND,
        });
        assert_eq!(s, State::PendingLeftCmd);
        state = s;

        // Shiftキーが押下された
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: 0x38, // kVK_Shift
            flags: FLAG_MASK_COMMAND | 0x0002_0000,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
        state = s;

        // 左Command解放
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: 0x0002_0000,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_simultaneous_commands_cancelled() {
        let mut state = State::Idle;

        // 左Command押下
        let (s, _) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_COMMAND | FLAG_DEV_LEFT_CMD,
        });
        assert_eq!(s, State::PendingLeftCmd);
        state = s;

        // 続いて右Commandも押下（両方押された状態）
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: FLAG_MASK_COMMAND | FLAG_DEV_LEFT_CMD | FLAG_DEV_RIGHT_CMD,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
        state = s;

        // 右Command解放
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: FLAG_MASK_COMMAND | FLAG_DEV_LEFT_CMD,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
        state = s;

        // 左Command解放
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: 0,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_sequential_taps() {
        let mut state = State::Idle;

        // 左Commandタップ
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_COMMAND,
        });
        assert_eq!(s, State::PendingLeftCmd);
        assert_eq!(act, Action::None);

        let (s, act) = s.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: 0,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::SendKey(KEY_JIS_EISU));
        state = s;

        // 右Commandタップ
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: FLAG_MASK_COMMAND,
        });
        assert_eq!(s, State::PendingRightCmd);
        assert_eq!(act, Action::None);

        let (s, act) = s.process_event(InputEvent::FlagsChanged {
            keycode: KEY_RIGHT_COMMAND,
            flags: 0,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::SendKey(KEY_JIS_KANA));
    }

    #[test]
    fn test_reset_event() {
        let mut state = State::Idle;

        let (s, _) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_COMMAND,
        });
        assert_eq!(s, State::PendingLeftCmd);
        state = s;

        let (s, act) = state.process_event(InputEvent::Reset);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_caps_tap_emits_escape() {
        assert_eq!(KEY_CAPS_PROXY, 0x4F);
        assert!((CAPS_HOLD_THRESHOLD_SECS - 0.25).abs() < f64::EPSILON);

        let state = State::Idle;

        // CapsLock押下（初回）
        let (s, act) = state.process_event(InputEvent::CapsDown { is_repeat: false });
        assert_eq!(s, State::PendingCaps);
        assert_eq!(act, Action::StartCapsTimer);

        // 250ms未満でCapsLock解放 -> Esc送出
        let (s, act) = s.process_event(InputEvent::CapsUp);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::SendKey(KEY_ESCAPE));
    }

    #[test]
    fn test_caps_hold_timeout_toggles_capslock() {
        let state = State::Idle;

        // CapsLock押下
        let (s, act) = state.process_event(InputEvent::CapsDown { is_repeat: false });
        assert_eq!(s, State::PendingCaps);
        assert_eq!(act, Action::StartCapsTimer);

        // 250ms経過（タイマー満了） -> CapsLockトグル発火
        let (s, act) = s.process_event(InputEvent::CapsHoldTimeout);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::ToggleCapsLock);

        // その後キーリピートが来ても無視
        let (s, act) = s.process_event(InputEvent::CapsDown { is_repeat: true });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);

        // その後キーを離しても何も発火しない
        let (s, act) = s.process_event(InputEvent::CapsUp);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_caps_autorepeat_ignored_while_pending() {
        let state = State::Idle;

        let (s, act) = state.process_event(InputEvent::CapsDown { is_repeat: false });
        assert_eq!(s, State::PendingCaps);
        assert_eq!(act, Action::StartCapsTimer);

        // 押下中のキーリピートはPendingCapsを維持し、タイマーを再起動しない
        let (s, act) = s.process_event(InputEvent::CapsDown { is_repeat: true });
        assert_eq!(s, State::PendingCaps);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_caps_cancelled_by_other_input() {
        let state = State::Idle;

        let (s, act) = state.process_event(InputEvent::CapsDown { is_repeat: false });
        assert_eq!(s, State::PendingCaps);
        assert_eq!(act, Action::StartCapsTimer);

        // 押下中に他のキーやマウス操作が発生 -> タイマーキャンセル & Idleへ
        let (s, act) = s.process_event(InputEvent::OtherInput);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::CancelCapsTimer);

        // その後CapsLockを離してもEscは発火しない
        let (s, act) = s.process_event(InputEvent::CapsUp);
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::None);
    }

    #[test]
    fn test_caps_cancelled_by_modifier_flags_changed() {
        let state = State::Idle;

        let (s, _) = state.process_event(InputEvent::CapsDown { is_repeat: false });
        assert_eq!(s, State::PendingCaps);

        // CapsLock押下中にCommandキーが押された場合
        let (s, act) = s.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_COMMAND | FLAG_DEV_LEFT_CMD,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::CancelCapsTimer);
    }

    #[test]
    fn test_command_tap_works_when_capslock_is_locked_on() {
        let state = State::Idle;

        // CapsLockがON（FLAG_MASK_ALPHA_SHIFT）の状態で左Command押下
        let (s, act) = state.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_ALPHA_SHIFT | FLAG_MASK_COMMAND | FLAG_DEV_LEFT_CMD,
        });
        assert_eq!(s, State::PendingLeftCmd);
        assert_eq!(act, Action::None);

        // 左Command解放（FLAG_MASK_ALPHA_SHIFTは維持） -> 英数が発火すること
        let (s, act) = s.process_event(InputEvent::FlagsChanged {
            keycode: KEY_LEFT_COMMAND,
            flags: FLAG_MASK_ALPHA_SHIFT,
        });
        assert_eq!(s, State::Idle);
        assert_eq!(act, Action::SendKey(KEY_JIS_EISU));
    }
}
