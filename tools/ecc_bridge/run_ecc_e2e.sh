#!/bin/sh
# run_ecc_e2e.sh — ecc-bridge の統合テスト(TODO/017 §テスト、SPEC §8.3)。
#
#   fake_ecc(Ice servant)を上げる → ecc_bridge を上げる → ecc_e2e_client が ZMQ REQ で
#   全状態遷移と listen-before-start の負性テストを機械照合する。
#
# 使い方(バイナリは make 済みであること):
#   make TPCDAQ_ICE_DIR=<repo>/reference/20190315_patched && ./run_ecc_e2e.sh
#
# ポートは全部 OS 任せ(fake_ecc は "PROXY ..."、ecc_bridge は "BIND ..." を stdout に 1 行出す)
# —— 固定ポートを書かないので並列実行に耐える。

set -eu
cd "$(dirname "$0")"

for bin in ./fake_ecc ./ecc_bridge ./ecc_e2e_client; do
  if [ ! -x "$bin" ]; then
    echo "run_ecc_e2e: $bin が無い。先に make (TPCDAQ_ICE_DIR=... ) すること" >&2
    exit 1
  fi
done

work=$(mktemp -d)
fake_pid=""
bridge_pid=""

cleanup() {
  [ -n "$bridge_pid" ] && kill "$bridge_pid" 2>/dev/null || true
  [ -n "$fake_pid" ] && kill "$fake_pid" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

# 指定ファイルの先頭行に $2 で始まる行が現れるまで待つ(最大 5 秒)。
wait_for_line() {
  file=$1
  prefix=$2
  i=0
  while [ "$i" -lt 100 ]; do
    line=$(grep "^$prefix " "$file" 2>/dev/null | head -n 1 || true)
    if [ -n "$line" ]; then
      echo "$line"
      return 0
    fi
    sleep 0.05
    i=$((i + 1))
  done
  return 1
}

./fake_ecc --port 0 >"$work/fake.out" 2>"$work/fake.err" &
fake_pid=$!
if ! proxy_line=$(wait_for_line "$work/fake.out" PROXY); then
  echo "run_ecc_e2e: fake_ecc が起動しない" >&2
  cat "$work/fake.err" >&2
  exit 1
fi
proxy=${proxy_line#PROXY }
echo "run_ecc_e2e: fake ECC proxy = $proxy"

./ecc_bridge --bind "tcp://127.0.0.1:*" --ecc-proxy "$proxy" \
  >"$work/bridge.out" 2>"$work/bridge.err" &
bridge_pid=$!
if ! bind_line=$(wait_for_line "$work/bridge.out" BIND); then
  echo "run_ecc_e2e: ecc_bridge が起動しない" >&2
  cat "$work/bridge.err" >&2
  exit 1
fi
endpoint=${bind_line#BIND }
echo "run_ecc_e2e: bridge REP = $endpoint"

rc=0
./ecc_e2e_client --bridge "$endpoint" || rc=$?

echo "--- fake_ecc log ---"
cat "$work/fake.err"
echo "--- ecc_bridge log ---"
cat "$work/bridge.err"

exit "$rc"
