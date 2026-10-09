#!/bin/bash
# daq_up.sh — 本番 DAQ 機でスタックを起動する(docs/NEW_DAQ_SETUP_ja.md §8、TODO/107)。
#
#   tools/deploy/daq_up.sh            # 既定の環境ファイル /opt/tpcdaq/daq.env
#   TPCDAQ_ENV=/path/daq.env tools/deploy/daq_up.sh
#
# 起動順は reference/_spike/demo/start_demo.sh と同じ(下流から):
#   root_sink → monitor → graw_writer → decoder → receiver0 → getEccServer → ecc_bridge → controller
# 各プロセスは setsid で切り離す(ssh から叩いても返ってくる)。pid は $LOGS/<name>.pid。
# 停止は tools/deploy/daq_down.sh。**起動しただけでは ECC は zCoBo に繋がない**(describe は run/start 時)。
#
# ホスト固有の値(パス)は環境ファイル、DAQ の設定は TOML(SPEC §3)。どちらもリポ外に置く
# (雛形 = tools/deploy/daq.env.example / config/daq_example.toml)。
set -uo pipefail

ENV_FILE=${TPCDAQ_ENV:-/opt/tpcdaq/daq.env}
[ -f "$ENV_FILE" ] || { echo "FAIL: no env file $ENV_FILE" >&2; exit 1; }
# shellcheck disable=SC1090
source "$ENV_FILE"

BIN=$TPCDAQ_REPO/target/release
ROOT_SINK=$TPCDAQ_REPO/tools/root_sink/root_sink
LOGS=$TPCDAQ_LOGS
CONF=$TPCDAQ_CONF

fail() { echo "FAIL: $*" >&2; exit 1; }

# TOML から 1 キーを拾う(最初に出た `key = "value"` / `key = value`)。
# 拾うのは [system] の output_root / geometry と [root_sink] の数値・真偽だけ
# (どれも TOML 内で一意なキー)。
toml_get() {
    sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\"\{0,1\}\([^\"#]*\)\"\{0,1\}.*/\1/p" "$CONF" \
        | head -1 | sed 's/[[:space:]]*$//'
}

wait_for_line() {  # wait_for_line <file> <pattern> <timeout_s>
    local file=$1 pattern=$2 timeout=${3:-20} waited=0
    while [ "$waited" -lt "$((timeout * 10))" ]; do
        grep -q "$pattern" "$file" 2>/dev/null && return 0
        sleep 0.1
        waited=$((waited + 1))
    done
    echo "TIMEOUT waiting for '$pattern' in $file" >&2
    [ -f "$file" ] && tail -5 "$file" >&2
    return 1
}

start() {  # start <name> <cmd...>
    local name=$1; shift
    if [ -f "$LOGS/$name.pid" ] && kill -0 "$(cat "$LOGS/$name.pid")" 2>/dev/null; then
        fail "$name already running (pid $(cat "$LOGS/$name.pid")) — run daq_down.sh first"
    fi
    # 前回のログは 1 世代だけ残す
    [ -f "$LOGS/$name.log" ] && mv "$LOGS/$name.log" "$LOGS/$name.log.prev"
    setsid "$@" > "$LOGS/$name.log" 2>&1 < /dev/null &
    echo $! > "$LOGS/$name.pid"
    echo "  $name pid $!"
}

[ -f "$CONF" ] || fail "missing config $CONF"
OUTPUT_ROOT=$(toml_get output_root)
GEOMETRY=$(toml_get geometry)
BUILD_TIMEOUT_MS=$(toml_get build_timeout_ms)
RAW_TREE=$(toml_get raw_tree)
[ -n "$OUTPUT_ROOT" ] && [ -d "$OUTPUT_ROOT" ] || fail "output_root '$OUTPUT_ROOT' is not a directory"
[ -f "$GEOMETRY" ] || fail "missing geometry '$GEOMETRY'"
for f in "$ROOT_SINK" "$BIN/monitor" "$BIN/graw_writer" "$BIN/decoder" "$BIN/receiver" \
         "$BIN/ecc_bridge" "$BIN/controller" "$GET_BIN/bin/getEccServer"; do
    [ -x "$f" ] || fail "missing binary: $f"
done
[ -d "$ECC_CONFIG_DIR" ] || fail "missing ECC config repo $ECC_CONFIG_DIR"
# root_sink の PEventTPC 並列書き出し本数(daq.env の ROOT_SINK_RECORDER_WORKERS、既定 1 — TODO/109)。
# > 1 で --recorder-workers N(064: mini 100 events/s)。その場合ファイル名は run{N}_w{k}.root になる。
# raw ツリーは常に 1 本(run{N}_raw.root)。
RECORDER_WORKERS=${ROOT_SINK_RECORDER_WORKERS:-1}
[[ "$RECORDER_WORKERS" =~ ^[1-9][0-9]*$ ]] || fail "ROOT_SINK_RECORDER_WORKERS='$RECORDER_WORKERS' is not a positive integer"
mkdir -p "$LOGS"

# 統合テストは 1024 で "Too many open files" になった(107)。ZMQ ソケット数に余裕を持たせる。
ulimit -n 65536 || fail "ulimit -n 65536 (hard limit $(ulimit -Hn))"
# shellcheck disable=SC1090
source "$ROOT_SETUP" || fail "cannot source $ROOT_SETUP"

echo "config: $CONF  output_root: $OUTPUT_ROOT  logs: $LOGS  root_sink workers: $RECORDER_WORKERS"
echo "== starting the stack =="

# 1. root-sink(下流から。monitor PUB を先に上げる — SUB は slow-joiner)
start root_sink "$ROOT_SINK" --bind "tcp://*:47003" --pub "tcp://*:47004" \
    --geometry "$GEOMETRY" --output-root "$OUTPUT_ROOT" --expect "${ROOT_SINK_EXPECT:-0:0}" \
    ${BUILD_TIMEOUT_MS:+--build-timeout-ms "$BUILD_TIMEOUT_MS"} \
    $( [ "$RECORDER_WORKERS" -gt 1 ] && echo --recorder-workers "$RECORDER_WORKERS" ) \
    $( [ "$RAW_TREE" = "true" ] && echo --raw-tree )
wait_for_line "$LOGS/root_sink.log" "monitor PUB bind" 20 || fail "root_sink did not bind"

# 2. monitor(WS)
start monitor "$BIN/monitor" --config "$CONF"
wait_for_line "$LOGS/monitor.log" "monitor WS listening" 20 || fail "monitor did not listen"
sleep 0.4   # SUB の slow-joiner マージン

# 3. graw-writer / decoder / receiver
start graw_writer "$BIN/graw_writer" --config "$CONF"
wait_for_line "$LOGS/graw_writer.log" "command socket listening" 20 || fail "graw_writer"
start decoder "$BIN/decoder" --config "$CONF"
wait_for_line "$LOGS/decoder.log" "command socket listening" 20 || fail "decoder"
start receiver0 "$BIN/receiver" --config "$CONF" --cobo-id 0
wait_for_line "$LOGS/receiver0.log" "command socket listening" 20 || fail "receiver0"

# 4. 実 ECC(16.04 ビルド + xenial libs + Ice 3.7.2 — NEW_DAQ_SETUP §5 A)。
#    LD_LIBRARY_PATH はこのプロセスにだけ渡す(ROOT 等に古い libs を混ぜない)。
#    Ice の Ecc アダプタはループバックのみ(ecc_bridge しか話さない)。cwd は設定リポジトリ(GDED と同じ)。
(cd "$ECC_CONFIG_DIR" && start ecc_server env LD_LIBRARY_PATH="$GET_LD_PATH" \
    "$GET_BIN/bin/getEccServer" --config-repo-url "$ECC_CONFIG_DIR" -a 127.0.0.1:46002) || exit 1
wait_for_line "$LOGS/ecc_server.log" "STARTED server on 127.0.0.1:46002" 20 || fail "getEccServer"

# 5. ecc-bridge(REP 47200 → Ice 46002)
start ecc_bridge "$BIN/ecc_bridge" --bind "tcp://127.0.0.1:47200" --ecc-proxy "Ecc:tcp -h 127.0.0.1 -p 46002"
wait_for_line "$LOGS/ecc_bridge.log" "BIND " 20 || fail "ecc_bridge"

# 6. controller(REST)
start controller "$BIN/controller" --config "$CONF"
wait_for_line "$LOGS/controller.log" "controller REST listening" 20 || fail "controller"

echo "== up =="
REST=$(toml_get rest_listen)
curl -s "http://${REST/0.0.0.0/127.0.0.1}/api/status" | python3 -m json.tool 2>/dev/null | head -40
