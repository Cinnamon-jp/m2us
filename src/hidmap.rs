//! `hidutil` の `UserKeyMapping` を扱う純粋ロジック（OS非依存・テスト可能）。
//!
//! 既存のユーザー設定リマップを壊さないために、`hidutil property --get UserKeyMapping`
//! の出力をパースし、自前のCapsLock→F18エントリだけを追加/除去する。

/// HID Usage: Keyboard CapsLock (page 0x07, usage 0x39)
pub const HID_CAPS_LOCK: u64 = 0x7_0000_0039;
/// HID Usage: Keyboard F18 (page 0x07, usage 0x6D)
pub const HID_F18: u64 = 0x7_0000_006D;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub src: u64,
    pub dst: u64,
}

/// m2usが追加する CapsLock → F18 のリマップ
pub const OWN_MAPPING: Mapping = Mapping {
    src: HID_CAPS_LOCK,
    dst: HID_F18,
};

fn parse_number(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// `key = value;` 形式の行から value を取り出す
fn field_value(block: &str, key: &str) -> Option<u64> {
    let idx = block.find(key)?;
    let rest = &block[idx + key.len()..];
    let rest = rest.trim_start().strip_prefix('=')?;
    let end = rest.find(';')?;
    parse_number(&rest[..end])
}

/// `hidutil property --get UserKeyMapping` の出力をパースする。
///
/// - 未設定（`(null)` や空の `( )`）は空のリストとして扱う。
/// - 形式が想定外（`{}` ブロックの数と取得できたエントリ数が一致しない等）の場合は `None`。
pub fn parse_user_key_mapping(output: &str) -> Option<Vec<Mapping>> {
    let mut result = Vec::new();
    let mut rest = output;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        let block = &after[..close];
        let src = field_value(block, "HIDKeyboardModifierMappingSrc")?;
        let dst = field_value(block, "HIDKeyboardModifierMappingDst")?;
        result.push(Mapping { src, dst });
        rest = &after[close + 1..];
    }
    // `{` を含まない出力は「未設定」とみなすが、エントリらしき記述があるのに
    // 取れていない場合は異常とする。
    if result.is_empty() && output.contains("HIDKeyboardModifierMapping") {
        return None;
    }
    Some(result)
}

/// 元の設定から、自前のエントリ（前回異常終了した残骸を含む）を取り除く。
pub fn without_own(mappings: &[Mapping]) -> Vec<Mapping> {
    mappings
        .iter()
        .copied()
        .filter(|m| *m != OWN_MAPPING)
        .collect()
}

/// 有効化時のマッピング一覧。CapsLockを別キーにリマップ済みの既存設定は置き換える。
pub fn with_own(original: &[Mapping]) -> Vec<Mapping> {
    let mut v: Vec<Mapping> = original
        .iter()
        .copied()
        .filter(|m| m.src != HID_CAPS_LOCK)
        .collect();
    v.push(OWN_MAPPING);
    v
}

/// `hidutil property --set` に渡すJSONを生成する。
pub fn set_payload(mappings: &[Mapping], caps_delay_override: bool) -> String {
    let entries: Vec<String> = mappings
        .iter()
        .map(|m| {
            format!(
                r#"{{"HIDKeyboardModifierMappingSrc":{},"HIDKeyboardModifierMappingDst":{}}}"#,
                m.src, m.dst
            )
        })
        .collect();
    let delay = if caps_delay_override {
        r#","CapsLockDelayOverride":0"#
    } else {
        ""
    };
    format!(r#"{{"UserKeyMapping":[{}]{}}}"#, entries.join(","), delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OWN: &str = "(\n        {\n        HIDKeyboardModifierMappingDst = 30064771181;\n        HIDKeyboardModifierMappingSrc = 30064771129;\n    }\n)\n";

    #[test]
    fn test_constants_match_decimal() {
        assert_eq!(HID_CAPS_LOCK, 30064771129);
        assert_eq!(HID_F18, 30064771181);
    }

    #[test]
    fn test_parse_null_and_empty() {
        assert_eq!(parse_user_key_mapping("(null)\n"), Some(vec![]));
        assert_eq!(parse_user_key_mapping("(\n)\n"), Some(vec![]));
        assert_eq!(parse_user_key_mapping(""), Some(vec![]));
    }

    #[test]
    fn test_parse_single() {
        assert_eq!(parse_user_key_mapping(SAMPLE_OWN), Some(vec![OWN_MAPPING]));
    }

    #[test]
    fn test_parse_multiple_and_hex() {
        let out = "(\n    {\n        HIDKeyboardModifierMappingDst = 0x7000000E0;\n        HIDKeyboardModifierMappingSrc = 30064771129;\n    },\n    {\n        HIDKeyboardModifierMappingDst = 30064771129;\n        HIDKeyboardModifierMappingSrc = 30064771296;\n    }\n)\n";
        assert_eq!(
            parse_user_key_mapping(out),
            Some(vec![
                Mapping {
                    src: HID_CAPS_LOCK,
                    dst: 0x7_0000_00E0
                },
                Mapping {
                    src: 30064771296,
                    dst: HID_CAPS_LOCK
                },
            ])
        );
    }

    #[test]
    fn test_parse_malformed_returns_none() {
        // Dstが欠けている
        let out = "( { HIDKeyboardModifierMappingSrc = 1; } )";
        assert_eq!(parse_user_key_mapping(out), None);
        // 閉じ括弧がない
        let out = "( { HIDKeyboardModifierMappingSrc = 1; HIDKeyboardModifierMappingDst = 2;";
        assert_eq!(parse_user_key_mapping(out), None);
        // 値が数値でない
        let out = "( { HIDKeyboardModifierMappingSrc = abc; HIDKeyboardModifierMappingDst = 2; } )";
        assert_eq!(parse_user_key_mapping(out), None);
        // エントリの記述はあるが {} がない
        assert_eq!(
            parse_user_key_mapping("HIDKeyboardModifierMappingSrc = 1;"),
            None
        );
    }

    #[test]
    fn test_without_own_removes_stale_entry() {
        let other = Mapping { src: 10, dst: 20 };
        assert_eq!(without_own(&[other, OWN_MAPPING]), vec![other]);
        assert_eq!(without_own(&[OWN_MAPPING]), vec![]);
        // CapsLockを別キーにしているユーザー設定は残す
        let user_caps = Mapping {
            src: HID_CAPS_LOCK,
            dst: 0x7_0000_00E0,
        };
        assert_eq!(without_own(&[user_caps]), vec![user_caps]);
    }

    #[test]
    fn test_with_own_keeps_others_and_replaces_caps() {
        let other = Mapping { src: 10, dst: 20 };
        let user_caps = Mapping {
            src: HID_CAPS_LOCK,
            dst: 0x7_0000_00E0,
        };
        assert_eq!(with_own(&[]), vec![OWN_MAPPING]);
        assert_eq!(with_own(&[other]), vec![other, OWN_MAPPING]);
        assert_eq!(with_own(&[other, user_caps]), vec![other, OWN_MAPPING]);
    }

    #[test]
    fn test_set_payload() {
        assert_eq!(set_payload(&[], false), r#"{"UserKeyMapping":[]}"#);
        assert_eq!(
            set_payload(&[OWN_MAPPING], true),
            r#"{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":30064771129,"HIDKeyboardModifierMappingDst":30064771181}],"CapsLockDelayOverride":0}"#
        );
        let other = Mapping { src: 1, dst: 2 };
        assert_eq!(
            set_payload(&[other, OWN_MAPPING], false),
            r#"{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":1,"HIDKeyboardModifierMappingDst":2},{"HIDKeyboardModifierMappingSrc":30064771129,"HIDKeyboardModifierMappingDst":30064771181}]}"#
        );
    }

    #[test]
    fn test_roundtrip_restore_after_crash() {
        // 前回異常終了: 自前エントリ + ユーザー設定が残っている
        let user = Mapping { src: 10, dst: 20 };
        let parsed = parse_user_key_mapping(SAMPLE_OWN).unwrap();
        let mut stale = vec![user];
        stale.extend(parsed);
        let original = without_own(&stale);
        assert_eq!(original, vec![user]);
        assert_eq!(with_own(&original), vec![user, OWN_MAPPING]);
    }
}
