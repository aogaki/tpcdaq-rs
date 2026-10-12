#!/bin/bash
# InfluxDB 2.x を ELITPC に入れる(TODO/122)。sudo で実行: sudo bash tools/deploy/influx_install.sh
# 公式手順(docs.influxdata.com/influxdb/v2/install, Linux / Ubuntu)のまま。鍵の指紋が一致しなければ何もしない。
set -euo pipefail
[ "$(id -u)" = "0" ] || { echo "sudo で実行してください: sudo bash $0"; exit 1; }
WORK=$(mktemp -d)                       # /tmp の他人所有ファイルとぶつからないように専用ディレクトリ(protected_regular 対策)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
echo "== 鍵を取得"
curl -fsSL -o influxdata-archive.key https://repos.influxdata.com/influxdata-archive.key
if ! gpg --show-keys --with-fingerprint --with-colons ./influxdata-archive.key 2>&1 \
     | grep -q '^fpr:\+24C975CBA61A024EE1B631787C3D57159FC2F927:$'; then
    echo "FAIL: 鍵の指紋が公式の値(24C975CB...9FC2F927)と一致しません。中止します。"; exit 1
fi
echo "鍵の指紋 OK"
install -d -m 0755 /etc/apt/keyrings
gpg --dearmor < influxdata-archive.key > /etc/apt/keyrings/influxdata-archive.gpg
echo 'deb [signed-by=/etc/apt/keyrings/influxdata-archive.gpg] https://repos.influxdata.com/debian stable main' \
    > /etc/apt/sources.list.d/influxdata.list
echo "== apt-get update"
apt-get update
echo "== install"
apt-get install -y influxdb2 influxdb2-cli
systemctl enable --now influxdb
for i in $(seq 30); do
    curl -s localhost:8086/health | grep -q '"status":"pass"' && { echo "InfluxDB is up:"; curl -s localhost:8086/health; echo; exit 0; }
    sleep 1
done
echo "FAIL: 30 s 待っても /health が pass になりません(journalctl -u influxdb を見てください)"; exit 1
