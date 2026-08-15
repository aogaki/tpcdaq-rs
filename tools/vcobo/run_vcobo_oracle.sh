#!/bin/bash
# run_vcobo_oracle.sh — **実 ECC(20190315 版)相手のフルウォーク**。ローカル限定・env ゲート。
#
#   make -C tools/vcobo oracle TPCDAQ_ICE_DIR=<repo>/reference/20190315_patched
#   TPCDAQ_SPIKE_PREFIX=<repo>/reference/_spike/prefix ./run_vcobo_oracle.sh
#
# 039(実 getHwServer + CLI シード)と**同じウォーク**を、vcobo_daq(46001 + 46004)相手に
# 走らせて全遷移 `result: 0` を確認する:
#   describe -> prepare -> configure -> start -> stop -> breakup -> reset -> reset
#
# 039 との違いは 2 つだけ:
#   - getHwServer もシーダ CLI も**動かさない**(vcobo_daq が 46001 も名乗る)
#   - configure xcfg は **checkPowerSupply=true の無改変版**(reference/_spike/run040/configs)
#     = alarm ビット i = !powerON{i} の意味論が実 ECC のチェックを通ることの実証
#
# データ面: 46005 で簡易 TCP リスナ(python3)が受け、daqStart 後にバイトが届くことを見る。
set -uo pipefail
cd "$(dirname "$0")"
REPO=$(cd ../.. && pwd)

SPIKE_PREFIX=${TPCDAQ_SPIKE_PREFIX:-$REPO/reference/_spike/prefix}
CONFIGS=${TPCDAQ_ORACLE_CONFIGS:-$REPO/reference/_spike/run040/configs}
ICE37=${ICE_HOME:-/opt/homebrew/opt/ice@3.7}
RUN=$REPO/reference/_spike/run040/logs

if [ ! -x "$SPIKE_PREFIX/bin/getEccServer" ]; then
  echo "SKIP: no getEccServer at $SPIKE_PREFIX/bin (set TPCDAQ_SPIKE_PREFIX)"
  exit 0
fi
if [ ! -f "$CONFIGS/describe-mini.xcfg" ]; then
  echo "SKIP: no xcfg set at $CONFIGS"
  exit 0
fi

export DYLD_LIBRARY_PATH="$SPIKE_PREFIX/lib:$ICE37/lib:/opt/homebrew/lib"
CLI="$SPIKE_PREFIX/bin/getEccClient"
ECC="$SPIKE_PREFIX/bin/getEccServer"

rm -rf "$RUN"
mkdir -p "$RUN"

# 合成 .graw(実 .graw はローカルのみ・リポに入れない)。
./make_graw_fixture "$RUN/oracle.graw" 200 1024 > "$RUN/fixture.txt" || exit 1

pkill -f "$SPIKE_PREFIX/bin/getEccServer" 2>/dev/null
pkill -f "$PWD/vcobo_daq" 2>/dev/null
sleep 1

# --- 1. 46005 の簡易 TCP リスナ(receiver の代わり)-------------------------
python3 - "$RUN/received.bin" > "$RUN/listener.log" 2>&1 <<'PY' &
import socket, sys
out = sys.argv[1]
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 46005))
s.listen(1)
print("listening on 127.0.0.1:46005", flush=True)
c, a = s.accept()
print("accepted from", a, flush=True)
n = 0
with open(out, "wb") as f:
    while True:
        b = c.recv(65536)
        if not b:
            break
        f.write(b)
        n += len(b)
print("closed after", n, "bytes", flush=True)
PY
LISTENER_PID=$!

# --- 2. vcobo_daq(46001 + 46004)-------------------------------------------
cat > "$RUN/vcobo.conf" <<EOF
cobo_id = 0
asad_count = 1
listen_host = "127.0.0.1"
ctrl_port = 46001
daq_port = 46004
rate_hz = 100.0
loop = false
flush_bytes = 8192
heartbeat_flush_ms = 3000
graw_files = ["$RUN/oracle.graw"]
EOF
./vcobo_daq --config "$RUN/vcobo.conf" > "$RUN/vcobo_daq.log" 2>&1 &
VCOBO_PID=$!

# --- 3. 実 ECC ---------------------------------------------------------------
"$ECC" --config-repo-url "$CONFIGS" > "$RUN/ecc.log" 2>&1 &
ECC_PID=$!

cleanup() {
  kill "$ECC_PID" "$VCOBO_PID" "$LISTENER_PID" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

sleep 2
if ! grep -q "vcobo_daq ready" "$RUN/vcobo_daq.log"; then
  echo "vcobo_daq did not come up:"; cat "$RUN/vcobo_daq.log"; exit 1
fi

# --- 4. フルウォーク --------------------------------------------------------
WALK=$RUN/walk.txt
: > "$WALK"
step() {  # step <label> <command>
  local label=$1; shift
  printf '%s\n' "$@" | { cat; echo q; } | "$CLI" 2>&1 \
    | sed -n '/^Get-Ecc > /,$p' | grep -v '^Get-Ecc > q$' >> "$WALK"
  echo "### $label" >> "$WALK"
}

DATALINK='<DataLinkSet><DataLink><DataSender id="CoBo[0]"/><DataRouter name="DataRouter0" ipAddress="127.0.0.1" port="46005" type="TCP"/></DataLink></DataLinkSet>'

step describe  "sm-describe mini"
step prepare   "sm-prepare mini"
step configure "sm-configure mini $DATALINK"
step start     "sm-start"
sleep 3          # 100 Hz x 3 s ぶんのデータが 46005 に届くはず
step stop      "sm-stop"
step breakup   "sm-breakup"
step reset1    "sm-reset"
step reset2    "sm-reset"
step status    "sm-status"

sleep 1

# --- 5. 判定 ----------------------------------------------------------------
echo "=== ECC state transitions ==="
grep "STEP END" "$RUN/ecc.log"

TOTAL=$(grep -c "STEP END" "$RUN/ecc.log")
OK=$(grep -c "STEP END:.*result: 0" "$RUN/ecc.log")
echo "transitions: $OK/$TOTAL with result: 0"

RECEIVED=0
[ -f "$RUN/received.bin" ] && RECEIVED=$(wc -c < "$RUN/received.bin" | tr -d ' ')
echo "data received on 46005: $RECEIVED bytes"

FINAL=$(grep -c "State: IDLE Error: NO_ERR" "$WALK")

STATUS=0
if [ "$TOTAL" -ne 8 ]; then echo "FAIL: expected 8 transitions, got $TOTAL"; STATUS=1; fi
if [ "$OK" -ne "$TOTAL" ]; then echo "FAIL: some transitions did not return result: 0"; STATUS=1; fi
if [ "$RECEIVED" -le 0 ]; then echo "FAIL: no data arrived on 46005"; STATUS=1; fi
if [ "$FINAL" -lt 1 ]; then echo "FAIL: the ECC did not walk back to IDLE / NO_ERR"; STATUS=1; fi

echo "=== vcobo_daq log ==="
cat "$RUN/vcobo_daq.log"
[ "$STATUS" -eq 0 ] && echo "ORACLE WALK: PASS" || echo "ORACLE WALK: FAIL"
exit $STATUS
