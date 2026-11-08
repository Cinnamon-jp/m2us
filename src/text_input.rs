/// フォーカスされているUI要素およびアプリケーションから取得した属性スナップショット
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusSnapshot<'a> {
    /// kAXRoleAttribute (例: "AXTextField", "AXTextArea", "AXWindow")
    pub role: Option<&'a str>,
    /// kAXSubroleAttribute (例: "AXSearchField", "AXSecureTextField")
    pub subrole: Option<&'a str>,
    /// kAXValueAttribute が Settable かどうか (読み取り専用 AXTextField / AXTextArea の判別用)
    pub value_settable: bool,
    /// kAXSelectedTextRangeAttribute が Settable かどうか (カスタム編集要素の判定用)
    pub selected_text_range_settable: bool,
    /// proc_pidpath から抽出した .app バンドル名または実行バイナリ名 (例: "Ghostty.app", "Slack.app")
    pub app_id: Option<&'a str>,
    /// 対象プロセスが Electron Framework.framework を内包する Electron アプリかどうか
    pub is_electron: bool,
}

/// パス文字列（例: "/Applications/Ghostty.app/Contents/MacOS/ghostty" または "/opt/homebrew/bin/wezterm-gui"）
/// から最外層の ".app" バンドル名（例: "Ghostty.app"）または末尾の実行ファイル名を抽出する純粋関数
pub fn extract_app_id(executable_path: &str) -> Option<&str> {
    if executable_path.is_empty() {
        return None;
    }
    for segment in executable_path.split('/') {
        if segment.ends_with(".app") && segment.len() > 4 {
            return Some(segment);
        }
    }
    executable_path.rsplit('/').find(|s| !s.is_empty())
}

/// 実行ファイルパス（例: "/Applications/Slack.app/Contents/MacOS/Slack"）から
/// 外側の ".app" バンドルルートパス（例: "/Applications/Slack.app"）を切り出す純粋関数。
/// Electron Helper プロセスのようなネストされたパスでも最外層の ".app" を返す。
pub fn bundle_root_from_executable(executable_path: &str) -> Option<&str> {
    let idx = executable_path.find(".app/")?;
    let root = &executable_path[..idx + 4];
    if root.ends_with("/.app") || root == ".app" {
        return None;
    }
    Some(root)
}

/// 明確にテキスト入力要素とみなす AXSubrole (Step 1)
pub fn is_text_subrole(subrole: &str) -> bool {
    matches!(subrole, "AXSearchField" | "AXSecureTextField")
}

/// AXSelectedTextRange の Settable 判定を信頼すべきでない非テキスト系コントロールの AXRole (Step 3)
pub fn is_non_text_control_role(role: &str) -> bool {
    matches!(
        role,
        "AXWindow"
            | "AXApplication"
            | "AXMenuBar"
            | "AXMenuBarItem"
            | "AXMenu"
            | "AXMenuItem"
            | "AXButton"
            | "AXCheckBox"
            | "AXRadioButton"
            | "AXPopUpButton"
            | "AXSlider"
            | "AXToolbar"
            | "AXScrollBar"
            | "AXScrollArea"
            | "AXSplitter"
            | "AXTabGroup"
            | "AXImage"
            | "AXIcon"
            | "AXList"
            | "AXOutline"
            | "AXTable"
            | "AXBrowser"
            | "AXRow"
            | "AXCell"
            | "AXGrid"
            | "AXLink"
            | "AXWebArea"
            | "AXStaticText"
    )
}

/// ウィンドウやコンテナ全体でテキスト入力を受け付けるターミナル・独自描画エディタの識別子判定 (Step 4)
pub fn is_text_centric_app(app_id: &str) -> bool {
    matches!(
        app_id,
        // GPU / サードパーティ製ターミナル
        "Ghostty.app"
            | "ghostty"
            | "WezTerm.app"
            | "wezterm-gui"
            | "Alacritty.app"
            | "alacritty"
            | "kitty.app"
            | "kitty"
            | "Warp.app"
            | "Hyper.app"
            | "Rio.app"
            | "rio"
            | "Tabby.app"
            // 標準・定番ターミナル（AXTextArea の value_settable が false の場合の救済を含む）
            | "Terminal.app"
            | "iTerm.app"
            | "iTerm2.app"
            // コードエディタ・ノートアプリ（Electron自動検出のバックアップおよび非Electron独自描画系）
            | "Visual Studio Code.app"
            | "Visual Studio Code - Insiders.app"
            | "Code.app"
            | "Cursor.app"
            | "Windsurf.app"
            | "Zed.app"
            | "Obsidian.app"
            | "Sublime Text.app"
    )
}

/// アプリ識別子または Electron 自動検出による例外許可（Step 4）を適用してよい Role かどうか
/// （ボタンやメニュー、リスト等にフォーカスがある場合は除外し、Terminal.app 等の AXTextArea は含める）
pub fn is_app_fallback_role(role: Option<&str>) -> bool {
    matches!(
        role,
        None | Some(
            "AXWindow" | "AXGroup" | "AXScrollArea" | "AXWebArea" | "AXSplitGroup" | "AXTextArea"
        )
    )
}

/// スナップショットから現在テキスト入力中かどうかを判定する純粋関数（§3.2 Step 1〜5 と完全一致）
pub fn evaluate_focus(snapshot: &FocusSnapshot<'_>) -> bool {
    // Step 1: Subrole による即時判定（パスワード欄・検索欄）
    if let Some(subrole) = snapshot.subrole
        && is_text_subrole(subrole)
    {
        return true;
    }

    // Step 2: Role によるテキスト入力判定
    //         AXTextField / AXTextArea で編集可能なら true。
    //         両方 false の場合は即 return false せず Step 4（Terminal.app等の救済）へ進む。
    if let Some(role) = snapshot.role {
        match role {
            "AXComboBox" | "AXSearchField" => return true,
            "AXTextField" | "AXTextArea"
                if snapshot.value_settable || snapshot.selected_text_range_settable =>
            {
                return true;
            }
            _ => {}
        }
    }

    // Step 3: 編集可能なテキスト選択範囲（AXSelectedTextRange Settable）による汎用判定
    //         ただし非テキスト系コントロール（AXWebArea, AXStaticText, AXList, AXButton等）は除外
    if snapshot.selected_text_range_settable {
        let excluded = snapshot.role.is_none_or(is_non_text_control_role);
        if !excluded {
            return true;
        }
    }

    // Step 4: ターミナル・独自描画エディタ・Electronアプリの例外許可（コンテナ/AXTextArea Role時のみ）
    if is_app_fallback_role(snapshot.role)
        && (snapshot.is_electron || snapshot.app_id.is_some_and(is_text_centric_app))
    {
        return true;
    }

    // Step 5: デフォルト（明確な非テキスト入力）
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_app_id() {
        assert_eq!(
            extract_app_id("/Applications/Ghostty.app/Contents/MacOS/ghostty"),
            Some("Ghostty.app")
        );
        assert_eq!(
            extract_app_id(
                "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper"
            ),
            Some("Visual Studio Code.app")
        );
        assert_eq!(
            extract_app_id("/opt/homebrew/bin/wezterm-gui"),
            Some("wezterm-gui")
        );
        assert_eq!(extract_app_id(""), None);
        assert_eq!(extract_app_id("/"), None);
    }

    #[test]
    fn test_bundle_root_from_executable() {
        assert_eq!(
            bundle_root_from_executable("/Applications/Slack.app/Contents/MacOS/Slack"),
            Some("/Applications/Slack.app")
        );
        assert_eq!(
            bundle_root_from_executable(
                "/Applications/Slack.app/Contents/Frameworks/Slack Helper.app/Contents/MacOS/Slack Helper"
            ),
            Some("/Applications/Slack.app")
        );
        assert_eq!(
            bundle_root_from_executable("/opt/homebrew/bin/wezterm-gui"),
            None
        );
        assert_eq!(bundle_root_from_executable("/.app/Contents/MacOS/x"), None);
    }

    #[test]
    fn test_editable_text_field_and_area() {
        assert!(evaluate_focus(&FocusSnapshot {
            role: Some("AXTextField"),
            value_settable: true,
            ..Default::default()
        }));
        assert!(evaluate_focus(&FocusSnapshot {
            role: Some("AXTextArea"),
            value_settable: false,
            selected_text_range_settable: true,
            ..Default::default()
        }));
    }

    #[test]
    fn test_read_only_text_area_in_normal_app_is_false() {
        // Xcodeのコンソールやログビューア等の読み取り専用 NSTextView
        assert!(!evaluate_focus(&FocusSnapshot {
            role: Some("AXTextArea"),
            value_settable: false,
            selected_text_range_settable: false,
            app_id: Some("Xcode.app"),
            is_electron: false,
            ..Default::default()
        }));
        assert!(!evaluate_focus(&FocusSnapshot {
            role: Some("AXTextField"),
            value_settable: false,
            selected_text_range_settable: false,
            app_id: Some("Finder.app"),
            is_electron: false,
            ..Default::default()
        }));
    }

    #[test]
    fn test_terminal_app_text_area_fallback_is_true() {
        // Terminal.app / iTerm2 で AXTextArea の value_settable が false でも Step 4 で救済されること
        for app in ["Terminal.app", "iTerm2.app", "iTerm.app"] {
            assert!(
                evaluate_focus(&FocusSnapshot {
                    role: Some("AXTextArea"),
                    value_settable: false,
                    selected_text_range_settable: false,
                    app_id: Some(app),
                    ..Default::default()
                }),
                "failed for {app}"
            );
        }
    }

    #[test]
    fn test_combo_box_search_field_and_subroles() {
        assert!(evaluate_focus(&FocusSnapshot {
            role: Some("AXComboBox"),
            ..Default::default()
        }));
        assert!(evaluate_focus(&FocusSnapshot {
            role: Some("AXSearchField"),
            ..Default::default()
        }));
        // NSSecureTextField は value_settable = false でも subrole で true
        assert!(evaluate_focus(&FocusSnapshot {
            role: Some("AXTextField"),
            subrole: Some("AXSecureTextField"),
            value_settable: false,
            ..Default::default()
        }));
        assert!(evaluate_focus(&FocusSnapshot {
            role: Some("AXGroup"),
            subrole: Some("AXSearchField"),
            ..Default::default()
        }));
    }

    #[test]
    fn test_custom_editable_element_by_selected_text_range() {
        assert!(evaluate_focus(&FocusSnapshot {
            role: Some("AXGroup"),
            selected_text_range_settable: true,
            ..Default::default()
        }));
    }

    #[test]
    fn test_non_text_control_roles_excluded_even_when_range_settable() {
        let non_text_roles = [
            "AXWindow",
            "AXApplication",
            "AXMenuBar",
            "AXMenuItem",
            "AXButton",
            "AXCheckBox",
            "AXList",
            "AXOutline",
            "AXTable",
            "AXBrowser",
            "AXRow",
            "AXCell",
            "AXLink",
            "AXWebArea",
            "AXStaticText",
        ];
        for role in non_text_roles {
            assert!(
                !evaluate_focus(&FocusSnapshot {
                    role: Some(role),
                    selected_text_range_settable: true,
                    app_id: Some("Safari.app"),
                    is_electron: false,
                    ..Default::default()
                }),
                "role {role} should be false in non-text-centric app"
            );
        }
    }

    #[test]
    fn test_terminals_and_electron_apps() {
        // GPUターミナル（AXWindow や None）
        for app in [
            "Ghostty.app",
            "WezTerm.app",
            "wezterm-gui",
            "Alacritty.app",
            "kitty.app",
        ] {
            assert!(evaluate_focus(&FocusSnapshot {
                role: Some("AXWindow"),
                app_id: Some(app),
                ..Default::default()
            }));
            assert!(evaluate_focus(&FocusSnapshot {
                role: None,
                app_id: Some(app),
                ..Default::default()
            }));
        }

        // 未登録の Electron アプリ（Slack.app, Notion.app 等）が is_electron = true で true になること
        for role in [None, Some("AXWindow"), Some("AXWebArea"), Some("AXGroup")] {
            assert!(evaluate_focus(&FocusSnapshot {
                role,
                app_id: Some("Slack.app"),
                is_electron: true,
                ..Default::default()
            }));
        }

        // ただしターミナルやElectronアプリ内でも、ボタン・メニュー・アウトライン操作中は false になること
        for role in [
            "AXButton",
            "AXMenuItem",
            "AXMenu",
            "AXOutline",
            "AXRow",
            "AXCheckBox",
        ] {
            assert!(!evaluate_focus(&FocusSnapshot {
                role: Some(role),
                app_id: Some("Ghostty.app"),
                is_electron: false,
                ..Default::default()
            }));
            assert!(!evaluate_focus(&FocusSnapshot {
                role: Some(role),
                app_id: Some("Slack.app"),
                is_electron: true,
                ..Default::default()
            }));
        }
    }

    #[test]
    fn test_default_empty_snapshot_is_false() {
        assert!(!evaluate_focus(&FocusSnapshot::default()));
        assert!(!evaluate_focus(&FocusSnapshot {
            role: Some("AXWindow"),
            app_id: Some("Finder.app"),
            ..Default::default()
        }));
    }
}
