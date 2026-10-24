#!/bin/bash
# test_daq_up_args.sh — daq_up.sh の [root_sink] pevent の解釈(TODO/183 T4)。
# 実プロセスは起こさない: daq_up.sh から toml_get / pevent_args だけを取り出して試す。
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
UP=$HERE/daq_up.sh
eval "$(sed -n '/^toml_get() {/,/^}/p' "$UP")"
eval "$(sed -n '/^pevent_args() {/,/^}/p' "$UP")"

failed=0
check() {  # check <name> <expected> <actual>
    if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected '$2' got '$3'"; failed=1; fi
}

CONF=$(mktemp)
trap 'rm -f "$CONF"' EXIT

# toml_get: 無ければ空、コメント末尾は無視、コメントアウト行は拾わない
printf '[root_sink]\nraw_tree = true\n# pevent = false\n' > "$CONF"
check "absent key is empty" "" "$(toml_get pevent)"
printf '[root_sink]\nraw_tree = true\npevent = false   # mini\n' > "$CONF"
check "pevent = false" "false" "$(toml_get pevent)"
check "raw_tree = true" "true" "$(toml_get raw_tree)"

# pevent_args: キー無し(空)= true = 引数なし / false + raw_tree = --no-root
check "default (empty) -> no args" "" "$(pevent_args "" "false" 2>&1)"
check "true -> no args" "" "$(pevent_args true false 2>&1)"
check "false + raw_tree -> --no-root" "--no-root" "$(pevent_args false true 2>&1)"
check "false without raw_tree -> fail" "FAIL: pevent = false needs raw_tree = true" "$(pevent_args false false 2>&1)"
pevent_args false "" >/dev/null 2>&1; check "false + empty raw_tree exit code" "1" "$?"
check "garbage -> fail" "FAIL: [root_sink] pevent must be true or false (got 'yes')" "$(pevent_args yes true 2>&1)"

bash -n "$UP" || failed=1
exit $failed
