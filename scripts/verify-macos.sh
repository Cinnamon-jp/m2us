#!/bin/bash
# m2us の macOS 実環境依存の挙動を検証するスクリプト。
# CI（macos-verify.yml、手動実行）と、実機のどちらでも実行できる。
#
# 使い方: scripts/verify-macos.sh [m2us バイナリのパス]
#
# 注意: 実行中は hidutil の UserKeyMapping を一時的に書き換え、終了時に空へ戻す。
#       実機で実行する場合、既存のリマップは失われる。必要なら事前に控えておくこと。
#
# 結果: PASS / FAIL / SKIP を表示する。1件でも FAIL があれば終了コード 1。
#       環境の制約（権限不足など）で検証できない項目は SKIP とする。

set -u

BIN="${1:-target/release/m2us}"
HIDUTIL=/usr/bin/hidutil

# HID Usage（10進）: CapsLock=0x700000039, F18=0x70000006D, F13=0x700000068, F14=0x700000069
OWN_SRC=30064771129
OWN_DST=30064771181
USER_SRC=30064771176
USER_DST=30064771177

FAIL=0
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; FAIL=1; }
skip() { echo "SKIP: $*"; }

TMP_DIR="$(mktemp -d)"
DAEMON_PID=""
HOLDER_PID=""

cleanup() {
    [ -n "$DAEMON_PID" ] && kill -9 "$DAEMON_PID" 2>/dev/null
    [ -n "$HOLDER_PID" ] && kill -9 "$HOLDER_PID" 2>/dev/null
    $HIDUTIL property --set '{"UserKeyMapping":[]}' >/dev/null 2>&1
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

get_mapping() { $HIDUTIL property --get UserKeyMapping 2>&1; }
set_mapping() { $HIDUTIL property --set "$1" >/dev/null 2>&1; }
user_only_json() {
    echo "{\"UserKeyMapping\":[{\"HIDKeyboardModifierMappingSrc\":$USER_SRC,\"HIDKeyboardModifierMappingDst\":$USER_DST}]}"
}
user_and_own_json() {
    echo "{\"UserKeyMapping\":[{\"HIDKeyboardModifierMappingSrc\":$USER_SRC,\"HIDKeyboardModifierMappingDst\":$USER_DST},{\"HIDKeyboardModifierMappingSrc\":$OWN_SRC,\"HIDKeyboardModifierMappingDst\":$OWN_DST}]}"
}
has_user() { echo "$1" | grep -q "$USER_SRC"; }
has_own() { echo "$1" | grep -q "$OWN_DST"; }

# デーモンをバックグラウンドで起動し、生存を確認する。
# 生存していれば DAEMON_PID を設定して 0、権限不足で終了していれば 2、それ以外の失敗は 1 を返す。
start_daemon() {
    "$BIN" >"$TMP_DIR/daemon.log" 2>&1 &
    DAEMON_PID=$!
    sleep 3
    if kill -0 "$DAEMON_PID" 2>/dev/null; then
        return 0
    fi
    wait "$DAEMON_PID" 2>/dev/null
    DAEMON_PID=""
    if grep -q "Failed to create CGEventTap" "$TMP_DIR/daemon.log"; then
        return 2
    fi
    return 1
}

wait_exit() {
    local pid="$1" i=0
    while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 50 ]; do
        sleep 0.1
        i=$((i + 1))
    done
    ! kill -0 "$pid" 2>/dev/null
}

[ -x "$BIN" ] || { echo "binary not found: $BIN"; exit 1; }

echo "== 1. hidutil --get の出力形式 =="
set_mapping '{"UserKeyMapping":[]}'
echo "--- 未設定時の出力:"
get_mapping
echo "--- ここまで"
set_mapping "$(user_and_own_json)"
OUT="$(get_mapping)"
echo "--- 2件設定時の出力:"
echo "$OUT"
echo "--- ここまで"
if has_user "$OUT" && has_own "$OUT"; then
    pass "hidutil --set/--get が機能し、10進数の Src/Dst が出力される"
    HIDUTIL_OK=1
else
    skip "この環境では hidutil --set が反映されない。以降の hidutil 依存テストをスキップする"
    HIDUTIL_OK=0
fi

echo "== 2. 二重起動の防止（権限に依存しない） =="
export TMPDIR="$TMP_DIR/"
python3 - "$TMP_DIR/m2us.lock" <<'PY' &
import fcntl, sys, time
f = open(sys.argv[1], "a")
fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
print("locked", flush=True)
time.sleep(30)
PY
HOLDER_PID=$!
sleep 1
if [ "$HIDUTIL_OK" = 1 ]; then
    set_mapping "$(user_only_json)"
fi
"$BIN" >"$TMP_DIR/second.log" 2>&1 &
SECOND_PID=$!
if wait_exit "$SECOND_PID"; then
    wait "$SECOND_PID"
    CODE=$?
else
    # 二重起動が防止されず、デーモンとして動き続けている
    kill -9 "$SECOND_PID" 2>/dev/null
    wait "$SECOND_PID" 2>/dev/null
    CODE=-1
fi
if [ "$CODE" = 1 ] && grep -q "already running" "$TMP_DIR/second.log"; then
    pass "ロック保持中の起動は 'already running' で終了コード1になる"
else
    fail "二重起動が防止されなかった (exit=$CODE): $(cat "$TMP_DIR/second.log")"
fi
if [ "$HIDUTIL_OK" = 1 ]; then
    OUT="$(get_mapping)"
    if has_user "$OUT" && ! has_own "$OUT"; then
        pass "二重起動の拒否時に hidutil の設定を変更しない"
    else
        fail "二重起動の拒否時に hidutil の設定が変更された: $OUT"
    fi
fi
kill -9 "$HOLDER_PID" 2>/dev/null
wait "$HOLDER_PID" 2>/dev/null
HOLDER_PID=""
sleep 1

if [ "$HIDUTIL_OK" != 1 ]; then
    skip "hidutil が使えないため、デーモン起動を伴うテストをスキップ"
    [ "$FAIL" = 0 ] && echo "ALL OK (with skips)" || echo "FAILED"
    exit "$FAIL"
fi

echo "== 3. 既存リマップの保持と復元（デーモン起動が必要） =="
set_mapping "$(user_only_json)"
start_daemon
RC=$?
if [ "$RC" = 2 ]; then
    skip "入力監視/アクセシビリティ権限がなく CGEventTap を作成できないため、デーモン起動を伴うテストをスキップ"
    [ "$FAIL" = 0 ] && echo "ALL OK (with skips)" || echo "FAILED"
    exit "$FAIL"
elif [ "$RC" != 0 ]; then
    fail "デーモンが起動直後に終了した: $(cat "$TMP_DIR/daemon.log")"
    exit 1
fi

OUT="$(get_mapping)"
if has_user "$OUT" && has_own "$OUT"; then
    pass "起動中: 既存リマップと CapsLock→F18 が共存している"
else
    fail "起動中のリマップが想定と異なる: $OUT"
fi
kill -TERM "$DAEMON_PID"
wait_exit "$DAEMON_PID" || kill -9 "$DAEMON_PID" 2>/dev/null
DAEMON_PID=""

echo "== 4. SIGHUP / SIGTERM / SIGINT で元の設定に復元される =="
for SIG in HUP TERM INT; do
    set_mapping "$(user_only_json)"
    start_daemon || { fail "SIG$SIG テスト用のデーモン起動に失敗"; continue; }
    kill -"$SIG" "$DAEMON_PID"
    if wait_exit "$DAEMON_PID"; then
        pass "SIG$SIG でデーモンが終了した"
    else
        fail "SIG$SIG でデーモンが終了しなかった"
        kill -9 "$DAEMON_PID" 2>/dev/null
    fi
    DAEMON_PID=""
    OUT="$(get_mapping)"
    if has_user "$OUT" && ! has_own "$OUT"; then
        pass "SIG$SIG 後: 既存リマップが残り、CapsLock→F18 は除去された"
    else
        fail "SIG$SIG 後のリマップが想定と異なる: $OUT"
    fi
done

echo "== 5. 前回異常終了の残骸からの復旧 =="
set_mapping "$(user_and_own_json)"
start_daemon || fail "残骸復旧テスト用のデーモン起動に失敗"
if [ -n "$DAEMON_PID" ]; then
    kill -TERM "$DAEMON_PID"
    wait_exit "$DAEMON_PID"
    DAEMON_PID=""
    OUT="$(get_mapping)"
    if has_user "$OUT" && ! has_own "$OUT"; then
        pass "残骸があっても、終了後は既存リマップのみになる"
    else
        fail "残骸復旧後のリマップが想定と異なる: $OUT"
    fi
fi

echo "== 6. SIGKILL 後の再起動で復旧する（SIGKILL 自体は復元不可） =="
set_mapping "$(user_only_json)"
start_daemon || fail "SIGKILL テスト用のデーモン起動に失敗"
if [ -n "$DAEMON_PID" ]; then
    kill -KILL "$DAEMON_PID"
    wait "$DAEMON_PID" 2>/dev/null
    DAEMON_PID=""
    OUT="$(get_mapping)"
    if has_own "$OUT"; then
        pass "SIGKILL 後は CapsLock→F18 が残る（既知の制約。READMEに記載の対処が必要）"
    else
        echo "NOTE: SIGKILL 後に CapsLock→F18 が残らなかった: $OUT"
    fi
    start_daemon || fail "SIGKILL 後の再起動に失敗（ロックが解放されていない可能性）"
    if [ -n "$DAEMON_PID" ]; then
        pass "SIGKILL 後にロックが自動解放され、再起動できた"
        kill -TERM "$DAEMON_PID"
        wait_exit "$DAEMON_PID"
        DAEMON_PID=""
        OUT="$(get_mapping)"
        if has_user "$OUT" && ! has_own "$OUT"; then
            pass "SIGKILL 後の再起動と終了で、既存リマップのみに復旧した"
        else
            fail "SIGKILL 後の復旧が想定と異なる: $OUT"
        fi
    fi
fi

if [ "$FAIL" = 0 ]; then
    echo "ALL OK"
else
    echo "FAILED"
fi
exit "$FAIL"
