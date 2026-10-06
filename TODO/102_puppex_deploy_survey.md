# 102 — デプロイ先候補 puppex の事前調査(読み取りのみ)

**Status: READY**(2026-10-06 起票 Opus。ユーザー「TPCDAQ のデプロイ先として puppex を考えています。まずはログインして調査して」。
調査は完了、**採否と前提条件はユーザー判断待ち** — 下記「判断が要る点」)

## 0. 方法
Mac から `ssh puppex`(ユーザー aogaki、公開鍵、非対話)。**変更は一切していない**(コマンドは参照系のみ、sudo 未使用)。
基準は [docs/NEW_DAQ_SETUP_ja.md](../docs/NEW_DAQ_SETUP_ja.md) v0.2 の §0〜§5。実 IP はここに書かない(ssh config の別名で参照)。

## 1. 結果(2026-10-06 09:2x、起動後 31 分の状態)

| 項目 | puppex | 手順書の要件 | 判定 |
|---|---|---|---|
| OS | Ubuntu 22.04.3、kernel 6.8 | 24.04 LTS を想定 | △(22.04 でも可。§5 の Ice / GET 手順は 22.04 で未検証) |
| CPU / RAM | i9-10900X 10C/20T、125 GiB、GPU T600 | — | ◎(mini も ELITPC も余裕) |
| NIC | Intel X550-T2(ixgbe、10GBASE-T × 2)。f1 = 実験 LAN(リンク 1 Gb/s)、**f0 = 未接続で空き** | LAN 用 + zCoBo 直結用の 2 本 | ○(f0 を zCoBo 直結に。1000BASE-T・MTU 9000 は X550 で可) |
| システムディスク | NVMe 938 GB、**89 % 使用(空き 103 GB)** | — | △ |
| データディスク | ZFS プール `data` 49 TB(18 TB HDD × 3)、**96〜97 % 使用(空き 1.6 TB)**、断片化 50 % | 別ボリューム、mini 28 MB/s 持続 | **✗**(ZFS は 90 % 超で書き込みが落ちる。28 MB/s なら 1.6 TB は約 16 時間) |
| 時刻 | Europe/Bucharest、NTP 同期済み | TZ 固定 + 同期 | ◎ |
| 権限 | aogaki は sudo(要パスワード)・dialout・docker | sudo、dialout | ○ |
| Rust | ~/.cargo に rustc / cargo 1.93 | stable(手元 1.97 系) | ○(`rustup update` で足りる) |
| ROOT | /opt/ROOT = 6.28/06(C++17) | 6.36 以降を想定 | △(root_sink の `make test-root` で要確認) |
| Node.js | 12.22(apt) | 22 LTS(UI ビルド) | △(UI は Mac でビルドして `ui/dist` を持ち込む、で回避可) |
| pyzmq | 無し | ベンチ・疎通スクリプト用 | △(`pip install --user pyzmq`) |
| GET / Ice | ~/ELITPC/GetSoftware_bin/20190315 に getEccServer あり(2025-03)。Ice 3.7 ライブラリはシステムに無い | §5 A(16.04 ビルド + VM 採取の Ice 3.7.2) | △(GDED と同じ非 root 展開で行けるはず、未検証) |
| ポート | tpcdaq 既定(46000–46009、47000–47099、8080、9000)は全て空き | — | ○ |
| シリアル(079 スローコントロール) | /dev/ttyUSB* / ttyACM* 無し | Bronkhorst ×2、HMP2020、Arduino | —(機器を繋ぐ場所次第) |
| 外部到達 | github.com / crates.io に HTTPS で到達 | ビルドに必要 | ○ |
| GDED への到達 | 不通(ただし調査時点で Mac からも GDED 不通 = GDED 停止中の可能性) | — | 保留 |

## 2. 共用機であること(最重要)
puppex は他実験の稼働中 DAQ 機:
- **delila 系の常駐**: InfluxDB 3 / Grafana / Mongo / mongo-express(Docker、`delila_*`)、`hv-monitor.service`(CAEN N1408 + SY5527 の HV 監視、`/home/daq/delila-rs/tools/hv_monitor`)。
- elog(ログブック)、apache2、NFS サーバ(`/data` を別ホストへ ro で公開)、PCP(pmcd / pmlogger)。
- `/data` に他実験のデータ(ELIFANT2025、Fission2026、d_55Mn、E9 ほか)と 124 GB の `daq.tgz`。daq ユーザーの最終ログインは 2026-09-25。
- tpcdaq を載せること自体は CPU・メモリ・ポートの面で衝突しない。衝突し得るのは **ディスク(容量と ZFS の I/O)**と、**他実験の run 中の作業**(再起動・ネットワーク設定変更・負荷試験)。

## 3. 判断が要る点(ユーザー)
1. **物理的な位置**: zCoBo 直結(f0 に RJ45)が必要。puppex は TPC / zCoBo の近くにあるか、ケーブルが届くか。
2. **データ置き場**: `/data` は満杯。tpcdaq 用の別ボリューム(空き HDD/SSD の追加、または ZFS の整理)を確保できるか。
3. **共用の可否**: 他実験(delila 系)の運用担当との調整。tpcdaq の常駐(receiver / decoder / writer / controller / ECC / UI)を同居させてよいか、run 中に再起動やネットワーク設定変更をしてよい時間帯。
4. OS は 22.04 のまま進めるか(他実験の都合で上げられないはず)。手順書は 24.04 前提なので、22.04 版の差分を追記する。

## 4. 前提が揃った後の作業(見積もり、Sonnet / Opus で発注可)
- NEW_DAQ_SETUP の 22.04 差分節(ROOT 6.28 の root_sink テスト、Ice 3.7.2 非 root 展開、Node は Mac ビルド持ち込み)。
- f0 の netplan(192.168.4.2/24、MTU 9000、ゲートウェイ無し)— sudo とネットワーク変更なので **他実験の run が無い時間に、ユーザー立ち会いで**。
- tpcdaq-rs のビルドとテスト(cargo 848 前後、root_sink `make test-root`)、getEccServer の起動確認、GDED と同じ疎通チェック。
