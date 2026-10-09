#!/bin/bash
# daq_down.sh — daq_up.sh で上げたスタックを止める。**プロセスを残留させない**。
#
#   tools/deploy/daq_down.sh
#
# run 中なら先に UI / REST で run/stop すること(ここは止めるだけで EOS は流さない)。
# 上流から畳む。Rust bin の graceful stop は SIGINT のみ(SIGTERM ハンドラ無し — 028 付録 A)、
# root_sink は SIGINT/SIGTERM 両方 graceful、getEccServer は SIGINT を無視するので SIGTERM(107 実測)。
#
# **root_sink だけは書き切るまで待ち、SIGKILL しない**(保存系 — TODO/109)。高レートの run の直後は
# 内部に未書き出しのイベントが溜まっていて、run/stop が返った後も数十秒〜数分書き続ける。
# 上限 ROOT_SINK_DRAIN_S(既定 600 s)を超えたら警告して**残したまま**終わる(pid ファイルも残すので
# daq_up.sh は「already running」で止まる)。
set -uo pipefail

ENV_FILE=${TPCDAQ_ENV:-/opt/tpcdaq/daq.env}
[ -f "$ENV_FILE" ] || { echo "FAIL: no env file $ENV_FILE" >&2; exit 1; }
# shellcheck disable=SC1090
source "$ENV_FILE"
LOGS=$TPCDAQ_LOGS
DRAIN_S=${ROOT_SINK_DRAIN_S:-600}

ORDER="controller ecc_bridge ecc_server receiver0 decoder graw_writer monitor root_sink"
root_sink_left=0

for name in $ORDER; do
    pidfile=$LOGS/$name.pid
    [ -f "$pidfile" ] || continue
    pid=$(cat "$pidfile")
    if kill -0 "$pid" 2>/dev/null; then
        sig=INT
        [ "$name" = "ecc_server" ] && sig=TERM
        kill -"$sig" "$pid" 2>/dev/null
        if [ "$name" = "root_sink" ]; then
            # 書き切るまで待つ。10 s ごとに経過を出す。
            waited=0
            while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt "$DRAIN_S" ]; do
                sleep 1
                waited=$((waited + 1))
                [ $((waited % 10)) -eq 0 ] && echo "  root_sink still writing ... ${waited} s"
            done
            if kill -0 "$pid" 2>/dev/null; then
                echo "  WARNING: root_sink (pid $pid) still writing after ${DRAIN_S} s — left running, NOT killed." >&2
                echo "           Wait for it to exit (tail $LOGS/root_sink.log), then run daq_down.sh again." >&2
                root_sink_left=1
                continue   # pid ファイルを残す
            fi
            echo "  root_sink stopped (${waited} s)"
        else
            # 1 本ずつ最大 10 s 待つ
            for _ in $(seq 1 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
            if kill -0 "$pid" 2>/dev/null; then
                echo "  $name (pid $pid) ignored SIG$sig for 10 s — SIGKILL"
                kill -9 "$pid" 2>/dev/null
            else
                echo "  $name stopped"
            fi
        fi
    fi
    rm -f "$pidfile"
done

# 取りこぼしの点検(pid ファイルの外で残ったもの)。実行ファイルのパスで厳密に絞る。
# 書き切り待ちで残した root_sink は対象外(SIGKILL しない)。
leftovers=0
pats=("$GET_BIN/bin/getEccServer" "$TPCDAQ_REPO/target/release/"{controller,ecc_bridge,receiver,decoder,graw_writer,monitor})
[ "$root_sink_left" = "0" ] && pats+=("$TPCDAQ_REPO/tools/root_sink/root_sink")
for pat in "${pats[@]}"; do
    pids=$(pgrep -f "^$pat( |$)" 2>/dev/null)
    if [ -n "$pids" ]; then
        echo "  leftover: $pat -> $pids (SIGKILL)"
        kill -9 $pids 2>/dev/null
        leftovers=1
    fi
done
if [ "$root_sink_left" = "1" ]; then
    exit 1
fi
[ "$leftovers" = "0" ] && echo "stopped — no leftover processes"
exit 0
