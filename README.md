# m2us (more mac US)

**m2us** は、US配列のMacBookにおいて左右Commandキーの空打ち（単体タップ）を「英数 / かな」に、CapsLockキーを「単押し＝Esc / 長押し＝CapsLock」に変換する超軽量な常駐デーモンです。

- **左Command単体タップ**: 「英数」(`kVK_JIS_Eisu`: `0x66`) ※テキスト入力フォーカス時のみ発火（非テキスト入力時は通常のCommand単体押しとして通過）
- **右Command単体タップ**: 「かな」(`kVK_JIS_Kana`: `0x68`) ※テキスト入力フォーカス時のみ発火（非テキスト入力時は通常のCommand単体押しとして通過）
- **CapsLock単押し（250ms未満）**: 「Esc」(`kVK_Escape`: `0x35`)
- **CapsLock長押し（250ms以上）**: 「CapsLock」トグル（250ms経過した瞬間に発火）
- **同時押し（ショートカット操作）**: 他のキーやマウスクリック等と組み合わせた場合は通常の修飾キーとして動作し、単押しアクションは発火しません。

外部クレートへの依存を一切持たず（ゼロ依存）、macOSのイベント駆動API（`CGEventTap` + `CFRunLoop` + `IOKit`）および最高優先度QoS（`USER_INTERACTIVE`）を採用することで、アイドル時のCPU使用率0%、メモリ消費最小、かつ遅延のないキー切り替えを実現しています。

---

## 動作要件

- macOS (Apple Silicon / `aarch64-apple-darwin`)
  - ※ Intel Mac には対応していません。

---

## インストール手順

### 1. バイナリの配置

#### A. リリースバイナリをダウンロードする場合（推奨）
GitHub Releases から `m2us.tar.gz` をダウンロードし、以下を実行します：

```bash
cd ~/Downloads

# 1. アーカイブを展開
tar -xzf m2us.tar.gz

# 2. macOSの検疫属性（Quarantineフラグ）を解除して実行権限を付与
xattr -d com.apple.quarantine m2us 2>/dev/null || true
chmod +x m2us

# 3. インストール先ディレクトリへ配置
mkdir -p ~/.local/bin
mv m2us ~/.local/bin/
```

#### B. ソースコードからビルドする場合
Rustツールチェーンがインストールされた環境で以下を実行します：

```bash
cargo build --release
mkdir -p ~/.local/bin
cp target/release/m2us ~/.local/bin/
```

---

## 権限設定（重要）

キーイベントの監視（`CGEventTap`）およびキーイベントの送出（`CGEventPost`）を行うため、macOS のセキュリティ権限が必要です。

> [!IMPORTANT]
> ターミナルから直接実行するとターミナルアプリの権限で動いてしまうことがありますが、後述の常駐（LaunchAgent）で動かすには **`m2us` バイナリ実体そのもの** への権限付与が必須です。

以下の2つの項目で権限を許可してください：

1. **システム設定** > **プライバシーとセキュリティ** を開きます。
2. **「入力監視」** を選択します。
   - `+` ボタンを押します（認証が求められたら解除）。
   - ファイル選択ダイアログで **`Cmd + Shift + G`** を押し、`/Users/YOUR_USERNAME/.local/bin/m2us`（または `~/.local/bin/m2us`）を入力して追加します。
   - リストに追加された `m2us` のトグルを **オン** にします。
3. 同様に **「アクセシビリティ」** を選択します。
   - `+` ボタンを押し、同様に `~/.local/bin/m2us` を追加して **オン** にします。

---

## バックグラウンド常駐（LaunchAgent設定）

ログイン時に自動起動させ、バックグラウンドで常駐させます。

### 1. 設定ファイル（.plist）を作成

以下のコマンドをターミナルでそのまま実行すると、絶対パスが展開された設定ファイルが作成されます：

```bash
mkdir -p ~/Library/LaunchAgents

cat <<EOF > ~/Library/LaunchAgents/com.user.m2us.plist
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.user.m2us</string>
    <key>ProgramArguments</key>
    <array>
        <string>$HOME/.local/bin/m2us</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
EOF
```

> [!NOTE]
> `.plist` 内では `~` や `$HOME` の自動展開が機能しないため、必ず上記のようにフルパス（例: `/Users/username/.local/bin/m2us`）で書き込まれる必要があります。

### 2. サービスを登録・起動

```bash
launchctl load ~/Library/LaunchAgents/com.user.m2us.plist
```

### 3. 起動確認

```bash
launchctl list | grep m2us
```

出力の左端に **プロセスID（PID: 数字）** が表示されていれば、正常に常駐して動作しています：
```text
12345   0   com.user.m2us
```

### 停止・アンインストールする場合

```bash
launchctl unload ~/Library/LaunchAgents/com.user.m2us.plist
rm -f ~/Library/LaunchAgents/com.user.m2us.plist
```

---

## アップデート（再インストール）手順

新しいバージョンのバイナリを適用する際は、以下の手順で更新します。

### 1. デーモンを一時停止
```bash
launchctl unload ~/Library/LaunchAgents/com.user.m2us.plist
```

### 2. 新しいバイナリを展開・配置
```bash
cd ~/Downloads
tar -xzf m2us.tar.gz
xattr -d com.apple.quarantine m2us 2>/dev/null || true
chmod +x m2us
mv -f m2us ~/.local/bin/m2us
```

### 3. 権限（TCC）の再登録【必須】
macOS のセキュリティ仕様上、バイナリが更新されるとコード署名の識別子（Identifier）が変化するため、**システム設定上は ON に見えても内部的にブロックされます（exit code 1 で即時終了します）**。

1. **システム設定 > プライバシーとセキュリティ > 入力監視** を開きます。
2. 既存の `m2us` を選択し、**`-`（マイナス）ボタンを押して一度削除** します。
3. **`+`（プラス）ボタン** を押し、`Cmd + Shift + G` で `~/.local/bin/m2us` を **再度追加して ON** にします。
4. **「アクセシビリティ」** でも同様に、既存の `m2us` を **`-` で削除してから `+` で再追加** します。

### 4. デーモンを再起動
```bash
launchctl load ~/Library/LaunchAgents/com.user.m2us.plist
```
`launchctl list | grep m2us` でプロセスID（PID）が表示されることを確認します。

---

## トラブルシューティング

`launchctl list | grep m2us` を実行した際、左端に PID が表示されない（`-` になっている）場合の対処法：

| ステータス表示例 | 主な原因 | 対処法 |
| :--- | :--- | :--- |
| `-    78    com.user.m2us` | **設定エラー（EX_CONFIG）**<br>plist 内のプログラムパスに `$HOME` や `~` が残っている、またはファイルが存在しない。 | plist 内のパスが `/Users/<username>/.local/bin/m2us` の絶対パスになっているか確認し、`unload` 後に再度 `load` してください。 |
| `-    1    com.user.m2us` | **権限エラー（Denied）**<br>「入力監視」または「アクセシビリティ」の権限が未許可、または更新前の古い識別子が残っている。 | システム設定の「入力監視」および「アクセシビリティ」から既存の `m2us` を **`-` で削除し、`+` で再追加** してください。 |
| 起動しているがキーが効かない | ターミナルアプリ側の権限で動いている、または常駐プロセスに権限が反映されていない。 | デーモンを再起動（`launchctl kill SIGTERM gui/$(id -u)/com.user.m2us`）するか、Mac を再ログインしてください。 |
