#!/bin/bash
# daq_down.sh — daq_up.sh で上げたスタックを止める。**プロセスを残留させない**。
#
#   tools/deploy/daq_down.sh
#
# run 中なら先に UI / REST で run/stop すること(ここは止めるだけで EOS は流さない)。
# 上流から畳む。Rust bin の graceful stop は SIGINT のみ(SIGTERM ハンドラ無し — 028 付録 A)、
# root_sink は SIGINT/SIGTERM 両方 graceful、getEccServer は SIGINT を無視するので SIGTERM(107 実測)。
set -uo pipefail

ENV_FILE=${TPCDAQ_ENV:-/opt/tpcdaq/daq.env}
[ -f "$ENV_FILE" ] || { echo "FAIL: no env file $ENV_FILE" >&2; exit 1; }
# shellcheck disable=SC1090
source "$ENV_FILE"
LOGS=$TPCDAQ_LOGS

ORDER="controller ecc_bridge ecc_server receiver0 decoder graw_writer monitor root_sink"

for name in $ORDER; do
    pidfile=$LOGS/$name.pid
    [ -f "$pidfile" ] || continue
    pid=$(cat "$pidfile")
    if kill -0 "$pid" 2>/dev/null; then
        sig=INT
        [ "$name" = "ecc_server" ] && sig=TERM
        kill -"$sig" "$pid" 2>/dev/null
        # 1 本ずつ最大 10 s 待つ(root_sink は ROOT ファイルを閉じ切るまで返らない)
        for _ in $(seq 1 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
        if kill -0 "$pid" 2>/dev/null; then
            echo "  $name (pid $pid) ignored SIG$sig for 10 s — SIGKILL"
            kill -9 "$pid" 2>/dev/null
        else
            echo "  $name stopped"
        fi
    fi
    rm -f "$pidfile"
done

# 取りこぼしの点検(pid ファイルの外で残ったもの)。実行ファイルのパスで厳密に絞る。
leftovers=0
for pat in "$GET_BIN/bin/getEccServer" "$TPCDAQ_REPO/tools/root_sink/root_sink" \
           "$TPCDAQ_REPO/target/release/"{controller,ecc_bridge,receiver,decoder,graw_writer,monitor}; do
    pids=$(pgrep -f "^$pat( |$)" 2>/dev/null)
    if [ -n "$pids" ]; then
        echo "  leftover: $pat -> $pids (SIGKILL)"
        kill -9 $pids 2>/dev/null
        leftovers=1
    fi
done
[ "$leftovers" = "0" ] && echo "stopped — no leftover processes"
exit 0
