#!/bin/bash
# run_vcobo_ci.sh — vcobo_daq の CI 可能な統合テスト(実 ECC も実 .graw も要らない)。
#
#   make -C tools/vcobo ci TPCDAQ_ICE_DIR=<repo>/reference/20190315_patched
#
# フェーズ 1(full): 合成 .graw を 2 本作って vcobo_daq を起動し、**encoding 1.0** の
#   Ice クライアントで 46001(create → 魔法値 → powerON/alarm)と 46004
#   (connect/start/stop/冪等/再 connect/disconnect)を機械照合したうえで、
#   リプレイ出力が**ソース .graw とバイト一致**すること・stop→start でリンクが
#   維持されること・受け側が読まない間ブロックするだけでドロップしないことを見る。
#   = TODO/040 受け入れ ②③④⑤
# フェーズ 2(heartbeat): flush_bytes を巨大に・rate_hz を低くした別インスタンスで、
#   **3 秒端数 flush**(§4.3-⑥)が効いていることを時間で見る。
set -uo pipefail
cd "$(dirname "$0")"

RUN=ci_run
rm -rf "$RUN"
mkdir -p "$RUN"

CTRL_PORT=${VCOBO_CI_CTRL_PORT:-47001}
DAQ_PORT=${VCOBO_CI_DAQ_PORT:-47004}
CTRL_PORT2=${VCOBO_CI_CTRL_PORT2:-47011}
DAQ_PORT2=${VCOBO_CI_DAQ_PORT2:-47014}

# 非対称なフレーム数・フレーム長にして、連結順の取り違えを見つける。
#   a.graw: 2000 frames x 1024 B = 2,048,000 B  (ソケットバッファより十分大きい
#                                                = 背圧テストが実際にブロックする)
#   b.graw:   24 frames x  512 B =    12,288 B
#   合計                            2,060,288 B
./make_graw_fixture "$RUN/a.graw" 2000 1024 || exit 1
./make_graw_fixture "$RUN/b.graw" 24 512 || exit 1
cat "$RUN/a.graw" "$RUN/b.graw" > "$RUN/expected.graw"

start_daq() {  # start_daq <conf> <log>
  ./vcobo_daq --config "$1" > "$2" 2>&1 &
  echo $!
}

wait_ready() {  # wait_ready <log>
  for _ in $(seq 50); do
    if grep -q "vcobo_daq ready" "$1" 2>/dev/null; then return 0; fi
    sleep 0.1
  done
  echo "vcobo_daq did not come up:"
  cat "$1"
  return 1
}

# --- フェーズ 1 -------------------------------------------------------------
cat > "$RUN/vcobo.conf" <<EOF
# CI 用: mini プロファイル(1 CoBo x 1 AsAd)、全速リプレイ、ループ無し。
cobo_id = 0
asad_count = 1
listen_host = "127.0.0.1"
ctrl_port = $CTRL_PORT
daq_port = $DAQ_PORT
rate_hz = 0.0
loop = false
flush_bytes = 4096
heartbeat_flush_ms = 3000
graw_files = ["$RUN/a.graw", "$RUN/b.graw"]
EOF

PID1=$(start_daq "$RUN/vcobo.conf" "$RUN/vcobo_daq.log")
PID2=""
PID3=""
cleanup() {
  [ -n "$PID1" ] && kill "$PID1" 2>/dev/null
  [ -n "$PID2" ] && kill "$PID2" 2>/dev/null
  [ -n "$PID3" ] && kill "$PID3" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

wait_ready "$RUN/vcobo_daq.log" || exit 1

./vcobo_ice_client --host 127.0.0.1 --ctrl-port "$CTRL_PORT" --daq-port "$DAQ_PORT" \
    --graw "$RUN/expected.graw"
STATUS=$?

# --- フェーズ 2(3 s 端数 flush)--------------------------------------------
# flush_bytes = 100 MB なので溜まりきることはない。rate 5 Hz x 512 B なので
# 3 s で ~7.5 KB。**ハートビートが無ければ 1 バイトも出てこない**構成。
cat > "$RUN/vcobo_hb.conf" <<EOF
cobo_id = 0
asad_count = 1
listen_host = "127.0.0.1"
ctrl_port = $CTRL_PORT2
daq_port = $DAQ_PORT2
rate_hz = 5.0
loop = true
flush_bytes = 100000000
heartbeat_flush_ms = 3000
graw_files = ["$RUN/b.graw"]
EOF

PID2=$(start_daq "$RUN/vcobo_hb.conf" "$RUN/vcobo_hb.log")
if wait_ready "$RUN/vcobo_hb.log"; then
  ./vcobo_ice_client --host 127.0.0.1 --daq-port "$DAQ_PORT2" --mode heartbeat --heartbeat-ms 3000
  HB_STATUS=$?
else
  HB_STATUS=1
fi

# --- フェーズ 3(任意): 実 .graw のバイト一致リプレイ -----------------------
# `TPCDAQ_REAL_GRAW_DIR` が指すディレクトリの最初の .graw を 1 本だけ 1 パス送る。
# 実 .graw はローカルのみ(リポには合成フィクスチャしか置かない)。
REAL_STATUS=0

if [ -n "${TPCDAQ_REAL_GRAW_DIR:-}" ]; then
  REAL_GRAW=$(ls "$TPCDAQ_REAL_GRAW_DIR"/*.graw 2>/dev/null | head -1)
  if [ -n "$REAL_GRAW" ]; then
    CTRL_PORT3=${VCOBO_CI_CTRL_PORT3:-47021}
    DAQ_PORT3=${VCOBO_CI_DAQ_PORT3:-47024}
    cat > "$RUN/vcobo_real.conf" <<EOF
cobo_id = 0
asad_count = 1
listen_host = "127.0.0.1"
ctrl_port = $CTRL_PORT3
daq_port = $DAQ_PORT3
rate_hz = 0.0
loop = false
flush_bytes = 65536
heartbeat_flush_ms = 3000
graw_files = ["$REAL_GRAW"]
EOF
    PID3=$(start_daq "$RUN/vcobo_real.conf" "$RUN/vcobo_real.log")
    if wait_ready "$RUN/vcobo_real.log"; then
      ./vcobo_ice_client --host 127.0.0.1 --daq-port "$DAQ_PORT3" --mode replay --graw "$REAL_GRAW"
      REAL_STATUS=$?
    else
      REAL_STATUS=1
    fi
  else
    echo "TPCDAQ_REAL_GRAW_DIR='$TPCDAQ_REAL_GRAW_DIR' has no .graw: skipping the real-data regression"
  fi
else
  echo "TPCDAQ_REAL_GRAW_DIR not set: skipping the real-data regression (optional)"
fi

echo "--- vcobo_daq log (phase 1) ---"
cat "$RUN/vcobo_daq.log"
echo "--- vcobo_daq log (phase 2: heartbeat) ---"
cat "$RUN/vcobo_hb.log"

[ "$STATUS" -eq 0 ] && [ "$HB_STATUS" -eq 0 ] && [ "$REAL_STATUS" -eq 0 ]
