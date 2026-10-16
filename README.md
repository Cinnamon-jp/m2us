# m2us (more mac US)

**m2us** は、US配列のMacBookにおいて左右Commandキーの空打ち（単体タップ）を「英数 / かな」に変換する超軽量な常駐デーモンです。

- **左Command単体タップ**: 「英数」(`kVK_JIS_Eisu`: `0x66`)
- **右Command単体タップ**: 「かな」(`kVK_JIS_Kana`: `0x68`)
- **同時押し（ショートカット操作）**: 他のキーやマウスクリック等と組み合わせた場合は通常のCommandキーとして動作し、「英数/かな」は発火しません。

外部クレートへの依存を一切持たず（ゼロ依存）、macOSのイベント駆動API（`CGEventTap` + `CFRunLoop`）および最高優先度QoS（`USER_INTERACTIVE`）を採用することで、アイドル時のCPU使用率0%、メモリ消費最小、かつ遅延のないキー切り替えを実現しています。

---

## 動作要件

- macOS (Apple Silicon / `aarch64-apple-darwin`)
  - ※ Intel Mac には対応していません。

---

## ビルド方法

Rustツールチェーンがインストールされた環境で以下を実行します：

```bash
cargo build --release
```

バイナリは `target/release/m2us` に生成されます。

---

## 権限設定

キーイベントの監視およびキーイベントの送出を行うため、macOSの権限が必要です。

1. `m2us` を実行します（初回実行時に権限要求ダイアログが表示されるか、権限エラーメッセージが出力されます）。
2. **システム設定** > **プライバシーとセキュリティ** を開きます。
3. **入力監視**（または **アクセシビリティ**）を選択します。
4. `+` ボタンを押してビルドした `m2us` バイナリを追加し、トグルをオンにして許可します。
5. ターミナルを再起動するか、バイナリを再実行します。

---

## バックグラウンド常駐（LaunchAgent設定）

ログイン時に自動起動させる場合は、LaunchAgentを設定します。

1. バイナリを任意のパス（例: `/usr/local/bin/m2us` または `~/.local/bin/m2us`）に配置します。

```bash
mkdir -p ~/.local/bin
cp target/release/m2us ~/.local/bin/
```

2. `~/Library/LaunchAgents/com.user.m2us.plist` を作成します：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.user.m2us</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Users/YOUR_USERNAME/.local/bin/m2us</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
```
※ `/Users/YOUR_USERNAME` は実際のユーザー名に置き換えてください。

3. サービスを登録・起動します：

```bash
launchctl load ~/Library/LaunchAgents/com.user.m2us.plist
```

停止・無効化する場合：
```bash
launchctl unload ~/Library/LaunchAgents/com.user.m2us.plist
```
