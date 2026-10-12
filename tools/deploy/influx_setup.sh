#!/bin/bash
# InfluxDB の初期設定と slowctl 用トークン(TODO/122)。daq ユーザーで実行(sudo 不要): bash tools/deploy/influx_setup.sh
# ユーザー名・パスワードは端末で聞かれる(チャットに書かない)。トークンは画面に出さず /opt/tpcdaq/influx.token に保存する。
set -euo pipefail
ORG=elitpc; BUCKET=slowctl; TOKEN_FILE=/opt/tpcdaq/influx.token
command -v influx >/dev/null || { echo "influx CLI がありません(先に sudo bash tools/deploy/influx_install.sh)"; exit 1; }
curl -s localhost:8086/health | grep -q '"status":"pass"' || { echo "InfluxDB が動いていません"; exit 1; }
echo "== 初期設定(管理者ユーザー名とパスワードを聞かれます。保持期間は 0 = 無期限)"
influx setup --org "$ORG" --bucket "$BUCKET" --retention 0
BID=$(influx bucket list --org "$ORG" --name "$BUCKET" --hide-headers | awk '{print $1}')
[ -n "$BID" ] || { echo "FAIL: bucket $BUCKET が見つかりません"; exit 1; }
echo "== slowctl 用の書き込み専用トークンを作成(bucket $BUCKET への write だけ)"
umask 077
influx auth create --org "$ORG" --write-bucket "$BID" --description "slowctl write (tpcdaq, TODO/122)" --json \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["token"])' > "$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"
[ -s "$TOKEN_FILE" ] || { echo "FAIL: トークンの保存に失敗"; exit 1; }
echo "OK: org=$ORG bucket=$BUCKET(id $BID)、トークンを $TOKEN_FILE に保存しました($(wc -c < "$TOKEN_FILE") bytes、中身は表示しません)"
