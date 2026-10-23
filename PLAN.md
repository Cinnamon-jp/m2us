# m2us 詳細設計書・実装手順書（実装担当LLM向け指示書）

このドキュメントは、`SPEC.md` に定義された macOS (Apple Silicon) 向け超軽量キーリマップデーモン **m2us** を実装するための完全な設計書および作業手順書です。実装を担当するエージェント（LLM）は、本ドキュメントの仕様と手順に厳密に従って実装を進めてください。

> [!IMPORTANT]
> **バージョン管理に関する必須ルール（Jujutsu / `jj` の使用）**
> - **このプロジェクトは `jj` (Jujutsu) で作成・管理されています。**
> - **作業の区切り（本手順書の各 Step 完了ごと）に、必ず `jj` を用いてコミットを行ってください。**
> - `git commit` ではなく、必ず `jj commit -m "<コミットメッセージ>"`（または `jj describe -m "..."` 後に `jj new`）を使用してください。
> - 既存の履歴（`Initialize cargo project`, `Add README`, `Add SPEC.md`）に合わせ、コミットメッセージは簡潔な英語の命令形で記述してください。

---

## 1. プロジェクト要件とアーキテクチャ方針

### 1.1 目的と機能要件
- US配列のMacBookにおいて、以下のキーバインドを提供する常駐デーモンを作成する。
  1. **左右Commandキーの「空打ち（単体プレス＆リリース）」**:
     - **左Command単体タップ** → **英数** (`kVK_JIS_Eisu`: `0x66`) を送出
     - **右Command単体タップ** → **かな** (`kVK_JIS_Kana`: `0x68`) を送出
     - Commandキーを押しながら他のキーを押した場合（例: `Cmd + C`）や、マウスクリック・スクロールを行った場合（例: `Cmd + Click`）、他の修飾キーと組み合わせた場合は、通常のCommand修飾キーとしてのみ動作させ、「英数/かな」は送出しない。
  2. **CapsLockキーの「単押し＝Esc / 長押し＝CapsLock」**:
     - **CapsLock単押し（250ms未満でリリース）** → **Esc** (`kVK_Escape`: `0x35`) を送出
     - **CapsLock長押し（250ms以上ホールド）** → **250ms経過した瞬間にCapsLock状態をトグル**（以降のキー解放時は何もしない）
     - CapsLockを押下してから250ms経過するまでの間に、他のキー・修飾キー・マウスクリック・スクロールが発生した場合はキャンセル（dirty扱い）し、Esc・CapsLockともに発火しない。
- 設定ファイルやCLI引数は持たず、単一バイナリとしてハードコードする。

### 1.2 非機能要件（極限までの軽量化・低遅延）
1. **外部クレート依存ゼロ (`[dependencies]` は空)**
   - `Cargo.toml` に外部クレートを追加しないこと。
   - macOSの `ApplicationServices` (`CoreGraphics`)、`CoreFoundation`、`IOKit`、および `pthread` QoS API は、必要な関数・定数のみを標準機能 (`unsafe extern "C"`) で直接FFI定義する。
2. **完全イベント駆動 + HIDリマップ連携 (`CGEventTap` + `CFRunLoopTimer` + `hidutil` + `IOKit`)**
   - ポーリングは一切行わず、`CGEventTapCreate` + `CFRunLoopRun()` でカーネルからのイベント通知を待機する（アイドル時CPU使用率 0%）。
   - macOSの仕様上、物理CapsLockキー (`0x700000039`) はHIDドライバ層で即座にロック状態が切り替わるため、起動時に `hidutil` を用いて物理CapsLockを未使用キー **`F18` (`0x70000006D`, Keycode `0x4F`)** にリマップし（同時に `CapsLockDelayOverride: 0` でハードウェア遅延を解除）、終了時（SIGINT / SIGTERM）に元へ復元する。
   - タップオプションには `kCGEventTapOptionDefault` (`0`) を使用し、リマップされた `F18` (`0x4F`) の `KeyDown` / `KeyUp` イベントのみを消費（`null` 返却）してインターセプトし、それ以外の全イベントは一切加工せずそのまま通過させる。
   - CapsLock長押し判定（250ms）には `CFRunLoopTimer` のワンショットタイマーを使用し、満了時に `IOKit` の `IOHIDSetModifierLockState` でOSのCapsLock状態を直接トグルする。
3. **スレッド優先度の最大化 (`QOS_CLASS_USER_INTERACTIVE`)**
   - バックグラウンド常駐時にmacOSの省電力制御でウェイクアップが遅延しないよう、起動時に `pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0)` を呼び出す。
4. **イベントループ内のヒープアロケーションゼロ**
   - コールバック関数内では `Box`, `Vec`, `String`, 標準出力へのI/O等を一切行わず、`AtomicU8` による状態遷移とC関数呼び出しのみで完結させる。
5. **ターゲット環境**
   - **Apple Silicon (`aarch64-apple-darwin`) のみ対応**（Intel Mac対応は不要）。
   - Linux開発環境でもコアロジックの単体テスト（`cargo test`）が実行できるよう、OS非依存の状態遷移モジュールとmacOS固有のFFIモジュールを分離する。

---

## 2. ディレクトリ・ファイル構成

最終的に以下のファイル構成とすること。

```text
m2us/
├── Cargo.toml                  # プロジェクト定義・リリース最適化設定
├── SPEC.md                     # 仕様定義（変更しない）
├── PLAN.md                     # 本設計・実装手順書（変更しない）
├── README.md                   # 概要・ビルド方法・権限設定・実行手順
├── .github/
│   └── workflows/
│       └── ci.yml              # GitHub Actions (Linuxテスト & macOS aarch64ビルド)
└── src/
    ├── main.rs                 # エントリポイント
    ├── state.rs                # OS非依存の状態マシンと単体テスト
    └── macos.rs                # #[cfg(target_os = "macos")] FFI定義・イベントループ実装
```

---

## 3. モジュール別詳細設計

### 3.1 `Cargo.toml`

`[dependencies]` は空のままにし、リリースビルドのサイズ・速度最適化プロファイルを追加する。

```toml
[package]
name = "m2us"
version = "0.1.0"
edition = "2024"

[dependencies]

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

---

### 3.2 コアロジック・状態マシン (`src/state.rs`)

OSに依存しない純粋なRustコードとして実装し、Linux上でも `cargo test` で全ロジックを検証できるようにする。

#### 定数定義
```rust
/// macOS Virtual KeyCodes
pub const KEY_LEFT_COMMAND: u16 = 0x37;  // 55: kVK_Command
pub const KEY_RIGHT_COMMAND: u16 = 0x36; // 54: kVK_RightCommand
pub const KEY_JIS_EISU: u16 = 0x66;      // 102: kVK_JIS_Eisu (英数)
pub const KEY_JIS_KANA: u16 = 0x68;      // 104: kVK_JIS_Kana (かな)
pub const KEY_ESCAPE: u16 = 0x35;        // 53: kVK_Escape (Esc)
pub const KEY_CAPS_PROXY: u16 = 0x4F;    // 79: kVK_F18 (hidutilにより物理CapsLockからリマップされたキー)

/// CapsLock長押し判定しきい値（秒）
pub const CAPS_HOLD_THRESHOLD_SECS: f64 = 0.25; // 250ms

/// CGEventFlags bitmasks
pub const FLAG_MASK_ALPHA_SHIFT: u64 = 0x0001_0000; // kCGEventFlagMaskAlphaShift (CapsLock ON状態)
pub const FLAG_MASK_COMMAND: u64 = 0x0010_0000;     // kCGEventFlagMaskCommand
/// Command以外の押下修飾キー（Shift, Control, Option）のマスク
/// ※ AlphaShift(0x0001_0000)はCapsLockロック中もCommandタップを有効にするため除外
pub const FLAG_MASK_OTHER_MODIFIERS: u64 = 0x0002_0000 | 0x0004_0000 | 0x0008_0000;

/// Device-dependent Command key flags (IOKit IOLLEvent.h NX_DEVICELCMDKEYMASK / NX_DEVICERCMDKEYMASK)
/// 左右Commandの同時押しを厳密に区別するために利用可能（下位ビット）
pub const FLAG_DEV_LEFT_CMD: u64 = 0x0000_0008;
pub const FLAG_DEV_RIGHT_CMD: u64 = 0x0000_0010;
```

#### 型定義と状態遷移ロジック
- **内部状態 (`State` - `#[repr(u8)]`)**:
  - `Idle = 0`: 待機状態（またはキャンセル済み状態）
  - `PendingLeftCmd = 1`: 左Commandが単体で押下され、まだ他の操作が行われていない状態
  - `PendingRightCmd = 2`: 右Commandが単体で押下され、まだ他の操作が行われていない状態
  - `PendingCaps = 3`: CapsLock（`KEY_CAPS_PROXY`）が単体で押下され、250ms未満かつ他の操作が行われていない状態

- **入力イベント (`InputEvent`)**:
  ```rust
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
      /// 通常キー押下、マウスクリック、スクロールなど単押しをキャンセルすべきイベント
      OtherInput,
      /// タップタイムアウト等によるリセット
      Reset,
  }
  ```

- **出力アクション (`Action`)**:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Action {
      None,
      SendKey(u16),     // KEY_JIS_EISU, KEY_JIS_KANA, または KEY_ESCAPE
      StartCapsTimer,   // 250msワンショットタイマーを開始
      CancelCapsTimer,  // 動作中のタイマーをキャンセル（他入力やReset時）
      ToggleCapsLock,   // IOKit経由でOSのCapsLockロック状態をトグル
  }
  ```

- **遷移アルゴリズム (`State::process_event(self, event: InputEvent) -> (State, Action)`)**:
  1. `InputEvent::OtherInput` または `InputEvent::Reset` の場合:
     - `self == State::PendingCaps` の場合は `(State::Idle, Action::CancelCapsTimer)` を返す。
     - それ以外は `(State::Idle, Action::None)` を返す。
  2. `InputEvent::CapsDown { is_repeat }` の場合:
     - `is_repeat == true` の場合は状態を維持し `(self, Action::None)` を返す。
     - `self == State::Idle` の場合は `(State::PendingCaps, Action::StartCapsTimer)` を返す。
     - それ以外（Command押下中など）の場合は `(State::Idle, Action::CancelCapsTimer)` を返す。
  3. `InputEvent::CapsUp` の場合:
     - `self == State::PendingCaps` の場合は単押し成立として `(State::Idle, Action::SendKey(KEY_ESCAPE))` を返す（呼び出し側でタイマーも停止）。
     - それ以外（既に長押し発火済み、またはdirtyキャンセル済み）の場合は `(State::Idle, Action::None)` を返す。
  4. `InputEvent::CapsHoldTimeout` の場合:
     - `self == State::PendingCaps` の場合は長押し成立として `(State::Idle, Action::ToggleCapsLock)` を返す。
     - それ以外の場合は `(self, Action::None)` を返す。
  5. `InputEvent::FlagsChanged { keycode, flags }` の場合:
     - `self == State::PendingCaps` の場合は他修飾キーが操作されたとみなし `(State::Idle, Action::CancelCapsTimer)` を返す。
     - 他の修飾キーが押されている場合 (`(flags & FLAG_MASK_OTHER_MODIFIERS) != 0`):
       - `(State::Idle, Action::None)` を返す。
     - 左右両方のデバイス依存Commandフラグが同時に立っている場合 (`(flags & (FLAG_DEV_LEFT_CMD | FLAG_DEV_RIGHT_CMD)) == (FLAG_DEV_LEFT_CMD | FLAG_DEV_RIGHT_CMD)`):
       - 左右Command同時押しとみなし、`(State::Idle, Action::None)` を返す。
     - `keycode == KEY_LEFT_COMMAND` の場合:
       - Commandフラグが立っているか判定: `let is_pressed = (flags & FLAG_MASK_COMMAND) != 0 && (flags & FLAG_DEV_LEFT_CMD != 0 || (flags & FLAG_DEV_RIGHT_CMD) == 0);`
       - **押下 (`is_pressed == true`)**:
         - 現在の状態が `State::Idle` かつ右Commandが押されていなければ `(State::PendingLeftCmd, Action::None)`。
         - それ以外の状態（既に `PendingRightCmd` 等）なら `(State::Idle, Action::None)`。
       - **解放 (`is_pressed == false`)**:
         - 現在の状態が `State::PendingLeftCmd` かつ `(flags & FLAG_MASK_COMMAND) == 0` であれば、空打ち成立として `(State::Idle, Action::SendKey(KEY_JIS_EISU))` を返す。
         - それ以外は `(State::Idle, Action::None)`。
     - `keycode == KEY_RIGHT_COMMAND` の場合:
       - Commandフラグが立っているか判定: `let is_pressed = (flags & FLAG_MASK_COMMAND) != 0 && (flags & FLAG_DEV_RIGHT_CMD != 0 || (flags & FLAG_DEV_LEFT_CMD) == 0);`
       - **押下 (`is_pressed == true`)**:
         - 現在の状態が `State::Idle` かつ左Commandが押されていなければ `(State::PendingRightCmd, Action::None)`。
         - それ以外の状態なら `(State::Idle, Action::None)`。
       - **解放 (`is_pressed == false`)**:
         - 現在の状態が `State::PendingRightCmd` かつ `(flags & FLAG_MASK_COMMAND) == 0` であれば、空打ち成立として `(State::Idle, Action::SendKey(KEY_JIS_KANA))` を返す。
         - それ以外は `(State::Idle, Action::None)`。
     - 上記以外の `keycode`（Shift, Option, Control, Fn 等）による `FlagsChanged` の場合:
       - `(State::Idle, Action::None)` を返す。

#### 単体テスト要件 (`src/state.rs` 内 `#[cfg(test)]`)
以下のテストケースをすべて作成し、パスさせること：
1. 左Commandの単体プレス＆リリースで `Action::SendKey(KEY_JIS_EISU)` が返ること。
2. 右Commandの単体プレス＆リリースで `Action::SendKey(KEY_JIS_KANA)` が返ること。
3. 左Command押下中に `InputEvent::OtherInput`（例: `C` キー押下やマウスクリック）が挟まった場合、左Commandを離しても `Action::None` になること。
4. 右Command押下中に `InputEvent::OtherInput` が挟まった場合、右Commandを離しても `Action::None` になること。
5. 左Command押下中に Shift などの別修飾キーが押された場合、キャンセルされ `Action::None` になること。
6. 左Command押下中に右Commandが押された場合（左右Command同時押し）、どちらを離しても `Action::None` になること。
7. 連続して左Command → 右Commandと空打ちした場合、順番通りに `KEY_JIS_EISU` → `KEY_JIS_KANA` が発火すること。
8. `InputEvent::Reset` 受信時に `Pending*` 状態が `Idle` にリセットされること。
9. CapsLockの単押し（`CapsDown { is_repeat: false }` → `CapsUp`）で `Action::StartCapsTimer` → `Action::SendKey(KEY_ESCAPE)` が返ること。
10. CapsLockの長押し（`CapsDown` → `CapsHoldTimeout` → `CapsUp`）で `CapsHoldTimeout` 時に `Action::ToggleCapsLock` が返り、その後の `CapsUp` では `Action::None` になること。
11. CapsLock押下中のキーリピート（`CapsDown { is_repeat: true }`）で `PendingCaps` 状態が維持され、`Action::None` になること。
12. CapsLock押下中に `OtherInput` や `FlagsChanged` が発生した場合、`Action::CancelCapsTimer` が返り、その後の `CapsUp` や `CapsHoldTimeout` で何も発火しないこと。
13. CapsLockロック中（`FLAG_MASK_ALPHA_SHIFT` が立っている状態）でも左右Commandの単体タップが正常に動作すること。

---

### 3.3 macOS FFI・イベントループ層 (`src/macos.rs`)

- **HIDリマップ (`hidutil`)**:
  - 起動時に `/usr/bin/hidutil property --set '{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}],"CapsLockDelayOverride":0}'` を実行し、物理CapsLock (`0x39`) を `F18` (`0x6D`, macOS keycode `0x4F`) に割り当てる。
  - シグナル（`SIGINT`, `SIGTERM`）受信時および終了時に `{"UserKeyMapping":[]}` を設定してリマップを解除する。
- **イベントタップ (`CGEventTap`)**:
  - `K_CG_EVENT_TAP_OPTION_DEFAULT` (`0`) で作成し、`K_CG_EVENT_KEY_UP` (`11`) もマスクに追加する。
  - `event_type == K_CG_EVENT_KEY_DOWN` または `K_CG_EVENT_KEY_UP` で `keycode == KEY_CAPS_PROXY (0x4F)` の場合:
    - `K_CG_KEYBOARD_EVENT_AUTOREPEAT` (`8`) フィールドを確認して `InputEvent::CapsDown { is_repeat }` または `InputEvent::CapsUp` を状態マシンに渡す。
    - アクションに応じて `start_caps_timer()` / `cancel_caps_timer()` / `send_key(KEY_ESCAPE)` を実行し、**戻り値として `ptr::null_mut()` を返して `F18` イベントを消費（破棄）する**。
  - それ以外のイベントは一切変更せず `event` をそのまま返す。
- **長押しタイマー (`CFRunLoopTimer`) & CapsLockトグル (`IOKit`)**:
  - `Action::StartCapsTimer`: `CFRunLoopTimerCreate` で 250ms (`0.25`秒) 後のワンショットタイマーを作成し、`CFRunLoopAddTimer` で現在のRunLoopに登録する。
  - `Action::CancelCapsTimer`（および `SendKey(KEY_ESCAPE)` 時）: `CFRunLoopTimerInvalidate` でタイマーを無効化・解放する。
  - タイマー満了コールバック: `InputEvent::CapsHoldTimeout` を状態マシンに渡し、`Action::ToggleCapsLock` が返った場合は `IOKit` の `IOHIDGetModifierLockState` / `IOHIDSetModifierLockState` (`kIOHIDCapsLockState = 1`) を呼び出してOSのCapsLock状態を反転させる。

---

### 3.4 エントリポイント (`src/main.rs`)

```rust
pub mod state;

#[cfg(target_os = "macos")]
mod macos;

fn main() {
    #[cfg(target_os = "macos")]
    {
        if let Err(err) = macos::run() {
            eprintln!("m2us error: {err}");
            std::process::exit(1);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("m2us is only supported on macOS (Apple Silicon).");
        std::process::exit(1);
    }
}
```
※ 非macOS環境で `cargo clippy --all-targets -- -D warnings` を実行した際に `state.rs` の関数や定数が `dead_code` 警告にならないよう、`pub mod state;` として公開するか適切に構成し、警告が一切出ないようにすること。

---

### 3.5 GitHub Actions ワークフロー (`.github/workflows/ci.yml`)

Apple Silicon (`macos-latest`, M1/ARM64ランナー) を使用してテストとリリースビルドを実行する。

```yaml
name: CI

on:
  push:
    branches: ["main", "master"]
  pull_request:

jobs:
  test-and-build:
    runs-on: macos-latest
    steps:
      - name: Checkout repository
        uses: actions/checkout@v4

      - name: Install Rust toolchain (aarch64-apple-darwin)
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-darwin
          components: rustfmt, clippy

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Run Clippy
        run: cargo clippy --all-targets --target aarch64-apple-darwin -- -D warnings

      - name: Run unit tests
        run: cargo test --target aarch64-apple-darwin

      - name: Build release binary (Apple Silicon)
        run: cargo build --release --target aarch64-apple-darwin

      - name: Check binary size
        run: ls -lh target/aarch64-apple-darwin/release/m2us

      - name: Upload release binary artifact
        uses: actions/upload-artifact@v4
        with:
          name: m2us-aarch64-apple-darwin
          path: target/aarch64-apple-darwin/release/m2us
```

---

## 4. 実装作業手順（Step-by-Step 実行ガイド）

実装担当LLMは、以下の **Step 1 〜 Step 3** を順番に実行し、**各ステップの最後に必ず `jj` でコミット**してください。

### Step 1: コアロジック (`src/state.rs`) と `Cargo.toml` の実装

1. `Cargo.toml` に `[profile.release]` の最適化設定を追記する。
2. `src/state.rs` を新規作成し、状態マシン（`State`, `InputEvent`, `Action`）および全テストケース（`#[cfg(test)]`）を実装する。
3. `src/main.rs` から `pub mod state;` を宣言し、以下の検証コマンドを実行してエラーや警告がゼロであることを確認する：
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
4. **【必須】`jj` でコミットを作成する**：
   ```bash
   jj status
   jj commit -m "Implement state machine for Command key tap detection"
   ```

### Step 2: macOS FFI層 (`src/macos.rs`) とエントリポイント (`src/main.rs`) の実装

1. `src/macos.rs` を新規作成し、`CoreGraphics` / `CoreFoundation` / `pthread` QoS のFFI宣言とイベントループ（`run` 関数、コールバック、キー送出処理）を実装する。
2. `src/main.rs` を完成させ、macOS環境では `macos::run()` を呼び出し、非macOS環境ではエラーメッセージを出して終了するように実装する。
3. 以下の検証コマンドを実行し、フォーマット・Clippy・テストがすべて通ることを確認する：
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
4. **【必須】`jj` でコミットを作成する**：
   ```bash
   jj status
   jj commit -m "Implement macOS CGEventTap daemon loop"
   ```

### Step 3: GitHub Actions CI (`.github/workflows/ci.yml`) と `README.md` の整備

1. `.github/workflows/ci.yml` を作成し、Apple Silicon (`aarch64-apple-darwin`) 向けのフォーマットチェック・Clippy・テスト・リリースビルド・アーティファクト保存を設定する。
2. `README.md` を更新し、以下の項目を簡潔に記載する：
   - 機能概要（US配列Macで左右Command空打ちを「英数/かな」に変換する超軽量デーモン）
   - 動作要件（macOS / Apple Silicon `aarch64-apple-darwin`）
   - ビルド方法（`cargo build --release`）
   - 実行に必要な権限設定（「システム設定 > プライバシーとセキュリティ > 入力監視（またはアクセシビリティ）」で `m2us` バイナリを許可する手順）
   - バックグラウンド常駐（`launchd` LaunchAgent での自動起動設定例）
3. 最終確認コマンドを実行する：
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
4. **【必須】`jj` でコミットを作成し、履歴を確認する**：
   ```bash
   jj status
   jj commit -m "Add GitHub Actions CI for Apple Silicon and update README"
   jj log -n 6
   ```

---

## 5. 完了条件チェックリスト

実装担当LLMは作業完了前に以下をすべて満たしているか確認すること：
- [ ] `Cargo.toml` の `[dependencies]` が空（外部クレート依存ゼロ）である。
- [ ] `cargo test` で全単体テストがパスしている。
- [ ] `cargo clippy --all-targets -- -D warnings` および `cargo fmt --all -- --check` が警告・エラーなしでパスしている。
- [ ] `.github/workflows/ci.yml` が `aarch64-apple-darwin` のみをターゲットとしている。
- [ ] 各作業ステップごとに `jj commit` が実行されており、`jj status` で `The working copy has no changes.` となっている。
