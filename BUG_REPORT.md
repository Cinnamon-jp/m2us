# バグ調査レポート

調査日: 2026-10-24 / 対象: m2us v0.2.0

## 概要

- 調査範囲: `src/state.rs`、`src/macos.rs`、`src/main.rs`、CI / Release ワークフロー、README、リリースノート、PLAN。
- `state.rs` の状態機械は、手で追った範囲では論理バグなし。15個のユニットテストは Linux 上で全件パスした。
- `macos.rs` は Linux では実行できないため、静的な読解のみで確認した。実機での検証は未実施。
- **データ破壊やクラッシュにつながる致命的バグは見つかっていない。** 運用上の事故につながり得る問題は発見1〜3。

| # | 優先度 | 概要 | 場所 | 状態 |
|---|--------|------|------|------|
| 1 | 高 | 異常終了すると CapsLock が F18 のまま残り、使えなくなる | `macos.rs` L316-325, L410, L430 | 修正済(未リリース) |
| 2 | 高 | 二重起動の防止がなく、片方の終了でもう片方の CapsLock が壊れる | `macos.rs` `run()` | 修正済(未リリース) |
| 3 | 中 | ユーザーの既存 `UserKeyMapping` を上書きし、終了時に全消去する | `macos.rs` `configure_hidutil` | 修正済(未リリース) |
| 4 | 低 | README の LaunchAgent 設定が `KeepAlive=true` で、権限エラー時に再起動を繰り返す | `README.md` | 未対応 |
| 5 | 低 | `CAPS_LOCK_ACTIVE` が内部値で、外部からの状態変更とずれる | `macos.rs` `toggle_caps_lock` | 未対応 |
| 6 | 低 | シグナルハンドラ内で `CFRunLoopStop` を呼んでいる | `macos.rs` `handle_signal` | 未対応 |
| 7 | 低 | `RUN_LOOP` 保存から `CFRunLoopRun` 開始までの間のシグナルが無視される | `macos.rs` `run()` | 未対応 |

進行状況は [PATCH_PROGRESS.md](PATCH_PROGRESS.md) を参照。

## 詳細

### 1. 異常終了すると CapsLock が使えなくなる(高)

- 起動時に `hidutil` で CapsLock を F18 にリマップする。復元は `CFRunLoopRun()` が正常に戻った後の `configure_hidutil(false)` だけ。
- 次の場合は復元されない。
  - `SIGKILL` で終了したとき
  - `SIGHUP` で終了したとき(ターミナルを閉じた場合。`SIGINT` と `SIGTERM` しか処理していない)
  - クラッシュしたとき(`panic = "abort"`)
- 復元されないと、F18 を受け取るデーモンがいないのに CapsLock が F18 のままになる。再起動か再実行まで CapsLock が無反応になる。
- 修正案:
  - `SIGHUP` もハンドリングする。
  - 起動時に、前回の残りのリマップを検知して整理する。
  - LaunchAgent 運用では `KeepAlive` による自動再起動が緩和策になる。

### 2. 二重起動の防止がない(高)

- 2つ目のプロセスも tap を作り、同じ入力を処理する。
- どちらかが終了すると `hidutil` のリマップが全消去され、もう一方が動いていても CapsLock が壊れる。
- 修正案: ロックファイルなどで単一インスタンスにする。

### 3. 既存のキーマップを上書きして消去する(中)

- 起動時は `UserKeyMapping` を自前の1件で丸ごと置き換える。
- 終了時は `[]` を設定する。ユーザーが他に設定していたリマップ(Ctrl↔Caps など)はすべて消える。
- 修正案: `hidutil property --get UserKeyMapping` で既存設定を読み、マージして保存する。終了時は読み取った内容を書き戻す。

### 4. LaunchAgent 設定の再起動ループ(低)

- 入力監視の権限が未付与のとき、`run()` は `Err` を返して `exit(1)` する。
- README の設定は `KeepAlive=true` なので、そのまま再起動を繰り返す。launchd は約10秒間隔で抑制するが、エラーログが出続ける。
- 修正案: `KeepAlive` を `{SuccessfulExit=false}` にし、`ThrottleInterval` を長めに設定する。

### 5. CapsLock 状態の内部管理(低)

- `CAPS_LOCK_ACTIVE` は起動時に1回読むだけで、以後は内部で反転させている。ロック状態を外部から変更されるとずれる。
- 修正案: トグルのたびに `IOHIDGetModifierLockState` で実状態を読む。

### 6. シグナルハンドラの安全性(低)

- `CFRunLoopStop` は async-signal-safe ではない。
- 修正案: self-pipe などを使うか、`CFRunLoopSource` 経由で終了を通知する。

### 7. 起動直後のシグナル(低)

- `RUN_LOOP` を保存してから `CFRunLoopRun` に入るまでの間にシグナルが来ると、終了要求が無視される。窓はごく狭い。

## 問題なしと確認できた点

- `hidutil` のキーコードは正しい(CapsLock `0x700000039`、F18 `0x70000006D`、仮想キーコード `0x4F`)。
- `CFRunLoopTimer` の retain/release、`CGEvent` の解放、tap 無効化時の復旧処理。
- Release ワークフローのリリースノート存在チェックと `Cargo.toml` の version の整合。

## 未調査(今後の候補)

1. `IOHIDSetModifierLockState` が実機で `FlagsChanged` を発生させるか。それによって状態が意図せずリセットされないか。
2. Esc などを自分が post したときの tap への再入。
3. `Cmd` 押下中のトラックパッドジェスチャで、空打ち判定が誤発火しないか。
