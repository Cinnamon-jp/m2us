**m2us** (more mac US) は、US配列のMacbookのキー配列を変更する超軽量なデーモンです。

キーマップ
- 左右のCommandキーに「英数/かな」を割り当てます。(JIS配列と同じ)
- CapsLockキーの単押しに「Esc」、長押し（250ms以上）に「CapsLock」を割り当てます。

条件
- できる限り極限まで軽量にすること(メモリ・CPUの仕様率は最小にすること)
- 言語にはRustを用いる
- macOSが必要なビルドやテストはGitHub Actionsを用いて実行する