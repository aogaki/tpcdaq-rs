# 102 — デプロイ先候補 puppex の事前調査(読み取りのみ)

**Status: COMPLETED**(2026-10-06 起票・調査 Opus。ユーザー「TPCDAQ のデプロイ先として puppex を考えています。まずはログインして調査して」→ 同日 **ユーザー裁定: 新規インストールから始める**(§5))
調査は完了、**採否と前提条件はユーザー判断待ち** — 下記「判断が要る点」)

## 0. 方法
Mac から `ssh puppex`(ユーザー aogaki、公開鍵、非対話)。**変更は一切していない**(コマンドは参照系のみ、sudo 未使用)。
基準は [docs/NEW_DAQ_SETUP_ja.md](../../docs/NEW_DAQ_SETUP_ja.md) v0.2 の §0〜§5。実 IP はここに書かない(ssh config の別名で参照)。

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

## 5. 裁定(2026-10-06 ユーザー)
「やっぱり新規インストールから始めるべきっぽい」+「オンボードでポートが 2 つ。ZFS のデータボリュームも掃除する」
→ **puppex 自体を OS 新規インストールし、tpcdaq の DAQ 機にする**(当初 Opus が「別の新機」と誤読して記録 → 同日訂正)。
- NIC: マザーボード ASUS WS X299 SAGE/10G のオンボード X550 × 2 = 実験 LAN 用 + zCoBo 直結用(手順書 §0 の要件を満たす)。
- データ: ZFS プール `data` は入れ直し後に import して使う(システムディスクだけ消える)。ユーザーが掃除して空きを作る。
- OS は手順書どおり 24.04 を推奨(22.04 の ZFS 2.2.2 と 24.04 の 2.2.x は互換。**`zpool upgrade` はしない** = 戻せなくなる)。

## 6. 入れ直し前の退避リスト(システムディスク NVMe が消える。読み取り調査 2026-10-06)
| 対象 | 場所 | 中身 / 退避方法 |
|---|---|---|
| delila 系スタック | `/home/daq/delila-rs/`(docker compose = `docker/`、InfluxDB = `docker/data/influxdb`、Grafana = `docker/data/grafana`、`docker/grafana/provisioning`、`mongo-init.js`) | ディレクトリごと /data へ。InfluxDB は停止してからコピー |
| Mongo のデータ | snap 版 Docker の名前付きボリューム(`/var/snap/docker/common/var-lib-docker/volumes/…`) | **bind でなく volume なので要注意**。`docker exec delila_mongo mongodump --archive` で吐き出す |
| HV 監視(CAEN N1408 + SY5527) | `/etc/systemd/system/hv-monitor.service`、`/home/daq/delila-rs/tools/hv_monitor/`(`launcher_config.yaml`) | unit ファイルと設定 |
| elog | `/var/lib/elog`(3.1 GB)、設定ファイル(`/usr/share/elog` 周辺、elogd.cfg の実位置は要確認) | 丸ごと |
| apache / DAQ-Middleware | `/var/www/{daqmw,html}`、`/etc/apache2/`、`/etc/xinetd.d/bootComps`、DAQ-Middleware 本体 | DAQ-Middleware を使い続けるなら再導入手順も |
| NFS | `/etc/exports`(`/data` を別ホストへ ro) | 1 行 |
| ユーザーと home | puppex / aogaki(200 GB)/ daq / vlad | uid を揃えて再作成(/data の所有者が uid で付いている: daq = 1002、aogaki = 1001) |
| その他 | `/opt/caen-toolbox`、`/opt/ROOT`(6.28、入れ直し後は新しい版で良い)、NVIDIA T600 ドライバ、ssh ホスト鍵 `/etc/ssh/ssh_host_*`(残せば known_hosts の警告が出ない)、PCP の設定 | 必要なものだけ |

- **ZFS の掃除**: スナップショットは無いので、消せばすぐ空く。性能のため **使用率 80 % 以下**を目安に(現在 96〜97 %、49 TB 中 47.4 TB)。
- tpcdaq 用には専用データセットを推奨: `zfs create -o compression=lz4 -o atime=off -o recordsize=1M data/tpcdaq`(graw は大きな連続書き込み)+ `reservation` で他の書き込みに食われないように。
  プール全体は現在 compression off / atime on(新しいデータセットだけ変えればよい)。
- ネットワーク: 現状は NetworkManager に「Wired connection 1〜5」の古いプロファイルが残っている。入れ直し後は zCoBo 側を netplan で固定(手順書 §2、NM の「Profile 1」事故の再発防止)。

## 7. 退避状況の確認(2026-10-06、ユーザー「HV 監視・apache/DAQ-Middleware・NFS は不要。InfluxDB / Grafana は移動済み。Mongo・Grafana 関連・eLog が Luna2 にコピー済みか確認を」)
読み取りのみ(Mongo は各コンテナの管理者資格情報を環境変数経由で使い、表示していない。Grafana / InfluxDB は匿名で読める API)。

| 対象 | puppex | Luna2 | 判定 |
|---|---|---|---|
| **Mongo** | `delila`(runs 520 件、run_config_snapshots 483 件、最新 2026-08-28)+ `eliade_es2`(2 + 2 件)。Docker の名前付きボリューム = **システムディスク上** | 稼働中の `mongo_db` は別物の旧 `DELILA.run_information`(1,961 件、最新 2025-01-21)だけ。ダンプも見当たらず(home 深さ 4、/data・/data2 深さ 3) | **✗ 未コピー** |
| Grafana | ダッシュボード 6 枚(最終更新 2026-05-07 以前)、フォルダ・アラート・注釈 0 | `delila_viewer` に API で書き出した 6 枚。uid・題名・パネル数とも一致、版数は同じか +1(既定時間範囲の変更分) | ○ |
| InfluxDB | コピー済み期間は件数一致(抜き取り: 2026-07-15 / 08-29 の system_rate・channel_rate・backlog)。**ただしコピー時刻 2026-09-18 10:18 以降 09-22 10:32 まで system_rate 約 130 万行・backlog 約 35 万行**(channel_rate と HV 系は無し) | 09-18 10:18 で終了 | △(約 4 日分が未コピー) |
| eLog「252Cf Fission July 2024」 | 1,955 ファイル / 1,101.8 MB | 同一(全ファイルのパスとサイズ一致) | ○ |
| eLog「252Cf Fission Phase 2」 | **4,913 ファイル / 2.1 GB、最新エントリ 2026-08-28**、`/etc/elog.conf` に節あり | **無い**(/var/lib/elog にも ~/elog にも、elog.conf にも無い) | **✗ 未コピー** |
| eLog「demo」 | 1 ファイル | 別の 1 ファイル | 無視可 |

## 8. 退避の実施(2026-10-06、ユーザー「組み込みはしなくていい」= Luna2 のサービスには入れず、ファイルとして退避)
Luna2 `~/puppex_backup_2026-10-06/`(700、各ファイル 600)に:
- `mongo_puppex_all.archive.gz`(174 kB): puppex の delila_mongo の全 DB を `mongodump --archive --gzip`。delila.runs 526 / run_config_snapshots 489 / eliade_es2 2 + 2 / admin。`gzip -t` 合格。
  (§7 の 520 / 483 は estimatedDocumentCount の値。ダンプの数が正。)復元時は Luna2 の既存 `DELILA` と大文字小文字違いで衝突するので名前を変える(README に記載)。
- `elog_252Cf_Fission_Phase_2.tgz`(2.0 GB): 4,913 ファイル / 2,110,950,899 バイトが両側一致、**ファイルごとの SHA-256 の束のハッシュも一致**。
- `elog.conf.puppex`(42 行、全 4 節)と `README.txt`。
- 未退避: InfluxDB の 2026-09-18 10:18〜09-22 10:32(ユーザーは移動済みとの認識。必要なら Luna2 README の Refresh 手順)。puppex の Mongo / elog にこの後書き込みがあればダンプは古くなる。

## 9. データ置き場の裁定(2026-10-08 ユーザー「ZFS のボリュームはとりあえず何もしないでおくことは可能か」→「そうしてください」)
- **ZFS プールには当面触らない**(import も掃除も tpcdaq 側の作業としてはしない)。OS は NVMe だけに新規インストール。新しい OS はプールを自動 import しないので中身は不変。
- tpcdaq のデータは **NVMe の別パーティション(ext4、noatime)を `/srv/tpcdaq`** に。`/data` はプールの mountpoint と衝突するので使わない。今の α 線源 run(0.03〜0.4 Hz × 557 kB)なら 1 日 1.5〜19 GB で十分、高レート運転の前に置き場を見直す。
- 手順書を **v0.3** に改訂(`docs/NEW_DAQ_SETUP_ja.md` §0 ディスク / §1 インストール先と分割・✅ / §6 output_root・params_dir / §11)。SPEC §3.1 とコード内テストの `/data/tpcdaq` は例示値なので据え置き。
- インストール前のディスク構成の控え(比較用、識別子を含むのでリポ外): `reference/_infra/puppex_disk_baseline_2026-10-08.txt`。この時点でプール `data` は 49.1 TB 中 37.0 TB 使用(10-06 の 47.4 TB から減少 = ユーザーが整理済みとみられる)。

## 10. OS の版(2026-10-08 ユーザー「26.04 は無理そうか。個人的には最新を使いたい」→「26.04 にします」)
- **Ubuntu 26.04 LTS** に決定。手順書を **v0.4** に改訂: ROOT は `root_v6.40.04.Linux-ubuntu26.04-x86_64-gcc15.2.tar.gz`(公式、Mac と同版)/ UI は Mac ビルド持ち込みが最短 /
  **実 ECC のネイティブ起動(§5 A、24.04 の GDED で実証済み)は 26.04 で未実証 → §1–§2 の直後に最初に確認**、ダメなら §5 C(16.04 コンテナ、`--network host`)。
- ECC の資産(GET の `build/.libs`、xenial の Boost 1.58 / libssl 1.0.0 等、VM 採取の Ice 3.7.2)は GDED の動作実績のある一式を写す(puppex の ~/ELITPC は入れ直しで消えるが不要)。

## 11. `/data/daq.tgz` は /home/daq のスナップショットか(2026-10-08、ユーザー依頼。全数照合は不要との指示)
- `/data/daq.tgz`(124 GB、2026-09-23 12:37 作成)の最上位は `daq/`(所有者 daq、権限 `drwxr-x---`、日付 09-22 13:54)= **/home を起点に /home/daq を tar したもの**。
- 今の /home/daq(daq ユーザーで一覧、456 GB、385,719 ファイル)のうち、アーカイブ作成時刻(09-23 12:27 を境に)以降に更新されたのは **1,288 ファイル・0.40 GB だけ**:
  warp ターミナルのサーバ(229 MB)、各種キャッシュ、firefox snap、そして **delila-rs/docker(33 MB = 稼働中の InfluxDB / Grafana / Mongo のデータ)**。
  → **daq.tgz は実質的に /home/daq の 09-23 時点のスナップショット**。差分はキャッシュと稼働中 DB だけ。
- 稼働中 DB の扱い: Mongo は 10-06 の論理ダンプ(Luna2、§8)が正。Grafana は Luna2 の書き出し JSON。InfluxDB の 09-18〜09-22 分(§7 で未コピーとした分)は **daq.tgz の中に入っている**(作成が 09-23 のため)。ただし稼働中に tar したので DB ファイルとしての一貫性は保証されない。
- daq.tgz は ZFS の /data 上にあり、入れ直しでは消えない(§9: プールには触らない)。
- 未確認: アーカイブ全体の破損検査(一覧作成を途中で止めた。必要なら `gzip -t /data/daq.tgz`、10〜20 分)。/home/daq/delila-rs/docker/data/db に Mongo の WiredTiger 本体があり、稼働中に書き込まれている(§6 の「Mongo は名前付きボリューム」は configdb 側の可能性)。
- ユーザー回答(2026-10-08): delila-rs などは利用者側ですでにバックアップ済み → /home/daq の退避は追加作業不要。
