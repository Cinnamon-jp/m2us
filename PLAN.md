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
- US配列のMacBookにおいて、**左右のCommandキーの「空打ち（単体プレス＆リリース）」**にそれぞれ「英数」「かな」キーを割り当てる常駐デーモンを作成する。
  - **左Command単体タップ** → **英数** (`kVK_JIS_Eisu`: `0x66`) を送出
  - **右Command単体タップ** → **かな** (`kVK_JIS_Kana`: `0x68`) を送出
- Commandキーを押しながら他のキーを押した場合（例: `Cmd + C`）や、マウスクリック・スクロールを行った場合（例: `Cmd + Click`）、他の修飾キーと組み合わせた場合は、通常のCommand修飾キーとしてのみ動作させ、「英数/かな」は送出しない。
- 設定ファイルやCLI引数は持たず、単一バイナリとしてハードコードする。

### 1.2 非機能要件（極限までの軽量化・低遅延）
1. **外部クレート依存ゼロ (`[dependencies]` は空)**
   - `Cargo.toml` に外部クレートを追加しないこと。
   - macOSの `ApplicationServices` (`CoreGraphics`)、`CoreFoundation`、および `pthread` QoS API は、必要な関数・定数のみを標準機能 (`unsafe extern "C"`) で直接FFI定義する。
2. **完全イベント駆動 + 監視専用タップ (`kCGEventTapOptionListenOnly`)**
   - ポーリングは一切行わず、`CGEventTapCreate` + `CFRunLoopRun()` でカーネルからのイベント通知を待機する（アイドル時CPU使用率 0%）。
   - タップオプションには `kCGEventTapOptionListenOnly` (`1`) を使用する。元のキー入力をフックして書き換えるのではなく、監視のみを行い、空打ち成立時のみ新規イベントを `CGEventPost` で追加送出する。これにより通常のタイピングやショートカットへの遅延が構造上ゼロになる。
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

/// CGEventFlags bitmasks
pub const FLAG_MASK_COMMAND: u64 = 0x0010_0000; // kCGEventFlagMaskCommand
/// Command以外の修飾キー（Shift, Control, Option, CapsLock）のマスク
/// AlphaShift/Caps(0x0001_0000) | Shift(0x0002_0000) | Control(0x0004_0000) | Alternate/Option(0x0008_0000)
pub const FLAG_MASK_OTHER_MODIFIERS: u64 = 0x0001_0000 | 0x0002_0000 | 0x0004_0000 | 0x0008_0000;

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

- **入力イベント (`InputEvent`)**:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum InputEvent {
      /// kCGEventFlagsChanged: keycode と イベント発生後の flags
      FlagsChanged { keycode: u16, flags: u64 },
      /// 通常キー押下、マウスクリック、スクロールなど空打ちをキャンセルすべきイベント
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
      SendKey(u16), // KEY_JIS_EISU または KEY_JIS_KANA
  }
  ```

- **遷移アルゴリズム (`State::process_event(self, event: InputEvent) -> (State, Action)`)**:
  1. `InputEvent::OtherInput` または `InputEvent::Reset` の場合:
     - 無条件に `(State::Idle, Action::None)` を返す。
  2. `InputEvent::FlagsChanged { keycode, flags }` の場合:
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

---

### 3.3 macOS FFI・イベントループ層 (`src/macos.rs`)

ファイル先頭または `main.rs` 側のモジュール宣言で `#[cfg(target_os = "macos")]` を付与する。

#### FFI 型・定数定義
```rust
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
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFFFFFE;
const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFFFFFF;

// CGEventField
const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;

// QoS Class (sys/qos.h)
const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
```

#### FFI 関数宣言
```rust
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
```

#### グローバル状態とコールバック実装のポイント
- `static STATE: AtomicU8 = AtomicU8::new(State::Idle as u8);`
- `static EVENT_TAP: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());`
- **監視イベントマスク (`events_of_interest`)**:
  ```rust
  let mask: CGEventMask = (1 << K_CG_EVENT_FLAGS_CHANGED)
      | (1 << K_CG_EVENT_KEY_DOWN)
      | (1 << K_CG_EVENT_LEFT_MOUSE_DOWN)
      | (1 << K_CG_EVENT_RIGHT_MOUSE_DOWN)
      | (1 << K_CG_EVENT_OTHER_MOUSE_DOWN)
      | (1 << K_CG_EVENT_SCROLL_WHEEL);
  ```
- **コールバック処理 (`event_tap_callback`)**:
  1. `event_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT || event_type == K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT` の場合:
     - `STATE` を `State::Idle` にリセット。
     - `EVENT_TAP.load(Ordering::Relaxed)` が非nullなら `CGEventTapEnable(tap, true)` を呼んで再有効化し、`event` をそのまま返す。
  2. `event_type == K_CG_EVENT_FLAGS_CHANGED` の場合:
     - `let keycode = CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) as u16;`
     - `let flags = CGEventGetFlags(event);`
     - `InputEvent::FlagsChanged { keycode, flags }` を `State::from_u8(STATE.load(Ordering::Relaxed)).process_event(...)` に渡す。
     - 新しい状態を `STATE.store(next_state as u8, Ordering::Relaxed)` で保存。
     - `Action::SendKey(target_key)` が返った場合、`send_key(target_key)` を呼び出す。
  3. その他の監視イベント（`KEY_DOWN`, マウスクリック, スクロール）の場合:
     - 自身が送出した「英数 (`0x66`)」「かな (`0x68`)」イベントであっても既に `Idle` なので副作用はないが、`InputEvent::OtherInput` として処理し `STATE` を `State::Idle` にする。
  4. 最後に `event` をそのまま返す。

- **キー送出関数 (`send_key(keycode: u16)`)**:
  ```rust
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
  ```
  ※ `CGEventSetFlags(..., 0)` により、万が一OS側にCommandフラグの残像があっても「Cmd+英数/かな」にならないよう明示的にフラグを0クリアして送出する。

- **初期化・起動 (`pub fn run() -> Result<(), &'static str>`)**:
  1. `unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0); }` を実行。
  2. `CGEventTapCreate` を呼び出す。戻り値が `null` の場合は `"Failed to create CGEventTap. Please grant Input Monitoring / Accessibility permissions."` を `Err` で返す。
  3. `EVENT_TAP.store(tap, Ordering::Relaxed);`
  4. `CFMachPortCreateRunLoopSource(ptr::null(), tap, 0)` でソースを作成（`null` なら `CFRelease(tap)` して `Err` を返す）。
  5. `CFRunLoopGetCurrent()` に `CFRunLoopAddSource(..., kCFRunLoopCommonModes)` で登録し、`CGEventTapEnable(tap, true)` を呼んだ上で `CFRunLoopRun()` を実行する。

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
