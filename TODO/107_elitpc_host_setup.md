# 107 — ELITPC(旧 puppex)を本番 DAQ 機に立てる(NEW_DAQ_SETUP v0.4 の実施)

**Status: IN PROGRESS**(2026-10-08 開始、Opus)

**位置づけ**: 080 A-1 の実施。102 の裁定(puppex を OS 新規インストールして tpcdaq 機に、ZFS は触らない)を受け、
ユーザーが Ubuntu 26.04 を導入し `ssh ELITPC`(ユーザー `daq`)で入れるようにした。手順の正本 = docs/NEW_DAQ_SETUP_ja.md。
v0.4 の指示どおり **§1–§2 の確認 → §5 A(ECC ネイティブ)を最初に** → §3/§4 → §6 以降。

## 現況(2026-10-08 15:00 EEST、読み取りのみで確認)

| 項目 | 状態 |
|---|---|
| OS | Ubuntu 26.04.1 LTS(デスクトップ版、カーネル 7.0.0-38)、ホスト名 `ELITPC`、TZ Europe/Bucharest |
| CPU / メモリ | 20 スレッド / 123 GiB、GPU NVIDIA T600 |
| NVMe | 953.9 GB = EFI 513 MB + **ルート ext4 953.4 GB の 1 枚**(手順書 §1 の「データ用パーティションを分ける」はされていない)。使用 17 GB |
| ZFS | sda/sdb/sdc(16.4 TB × 3)= `zfs_member data` のまま。zfsutils 未導入 = **未 import、無傷** |
| NIC | X550 × 2: `enp179s0f1` = LAN(DHCP 相当の固定、/22)、**`enp179s0f0` = 空き(NO-CARRIER)= zCoBo 直結用** |
| sudo | 要パスワード(daq は sudo 群)→ sudo の要る作業はユーザーにまとめて依頼 |
| 時刻 | chrony 稼働中だが**未同期**(下記) |

## §5 A — 実 ECC ネイティブ起動 ✅(26.04 で初実証)

- 資産を GDED から Mac 経由でストリーム(`tar czf - | ssh ELITPC tar xzf -`、計約 90 MB、`*.graw` と `vivado.*` は除外):
  `~/ELITPC/GetSoftware_bin/20190315`(bin/lib)、`~/get_docker/stage`(xenial の Boost 1.58 / libssl 1.0.0 / libbz2 / readline6 / tinfo5)、
  `~/get_docker/vm_ice`(VM 採取の Ice 3.7.2)、`~/ELITPC/GetSoftware_config`。GDED と同じパス構成。
- `ldd`(LD_LIBRARY_PATH = vm_ice : GETLIB : stage/usr/lib : stage/lib)で getEccServer / getEccClient とも **not found 0**。
  26.04 の glibc / libstdc++ で 16.04 ビルドがそのまま動く。
- `getEccServer --config-repo-url ~/ELITPC/GetSoftware_config -a 127.0.0.1:46002` → `127.0.0.1:46002` LISTEN、
  ログ `TRANS: [#__INIT__#:__OFF__->Idle]`。getEccClient: **`sm-status` = `State: IDLE Error: NO_ERR`**、`config-list` = 20 トリプレット
  (describe zCobo / doublezCobo × prepare/configure 10 種)。`!! Failed to retrieve logging topic 'GetEccLogger'` は IceStorm 不在の警告で無害(GDED と同じ)。
- 試験後 `pkill -TERM -x getEccServer` で停止、46002 / 46011 とも閉。zCoBo 側の線は未接続なので実機には一切触れていない。
- **結論: 16.04 コンテナ(§5 C)は不要**。

## 時刻同期 — NTS が研究所 FW で遮断

- 26.04 の chrony 既定 = Ubuntu の **NTS 付き** pool(`/etc/chrony/sources.d/ubuntu-ntp-pools.sources`、`nts prefer`)。
  NTS-KE の **TCP 4460 が遮断**(`1.ntp.ubuntu.com:4460` / `ntp-bootstrap.ubuntu.com:4460` とも不通)→ Reach 0 のまま。
- 素の NTP(UDP 123)は通る: `ntp.ubuntu.com` / `pool.ntp.org` に SNTP 問い合わせ可、現時点のずれ −0.23〜−0.30 s。
  GDED も timesyncd で `ntp.ubuntu.com`(素の NTP)に同期している。
- 対処(sudo): `/etc/chrony/sources.d/` に `pool ntp.ubuntu.com iburst` を足して `chronyc reload sources`。

## §4 ROOT 6.40.04 — 実行時依存の洗い出し

- `root_v6.40.04.Linux-ubuntu26.04-x86_64-gcc15.2.tar.gz`(342,924,179 B)を ELITPC で直接取得 → `~/root_stage/root`(810 MB)に展開。
- `ldd bin/root.exe bin/rootcling lib/*.so`(145 本)の not found:
  `libtbb.so.12`(191 件 = 必須)、`libGLU` / `libOpenGL` / `libgl2ps` / `libftgl`(グラフィクス)、`libgsl` / `libgslcblas` / `libblas` / `libgfortran`(数学)、
  `libcfitsio`、および使わないもの(`libR` / `libRInside` / `libunuran` / `libXrd*`)。
- 26.04 の該当パッケージ(`apt-cache policy` で存在確認): `libtbb12 libgsl28 libgslcblas0 libblas3 libglu1-mesa libgl2ps1.4 libftgl2 libopengl0 libgfortran5 libcfitsio10t64`。
  `libunuran16` と XRootD クライアント(`libxrdcl3` 等)は 26.04 に無い → 該当の ROOT ライブラリ(Unuran / NetxNG)は使わないので放置。

## §3 ツールチェーン

- rustup(`--profile minimal -c rustfmt,clippy`)→ **rustc / cargo 1.99.0**(Mac は 1.97.1)。C リンカ(`cc`)は build-essential 待ち。
- 26.04 デスクトップ版には **gcc / git / curl / cmake が入っていない**(wget と python3 3.14.4 はある)。

## sudo 作業(ユーザー実施、2026-10-08)

1 回目(15:12): `build-essential git cmake curl python3-zmq` + ROOT 実行時依存 10 個 + 素の NTP 追加 + dialout + `/opt/ROOT /opt/tpcdaq /srv/tpcdaq`(daq 所有)+
`/etc/profile.d/root.sh`。2 回目(15:17): **`libzmq3-dev libboost-dev`**(root_sink 用。v0.4 の手順から抜けていた)。

## 結果(2026-10-08、ELITPC = Ubuntu 26.04.1 / gcc 15.2.0 / Rust 1.99.0 / ROOT 6.40.04)

| 節 | 内容 | 結果 |
|---|---|---|
| §4 ROOT | 公式バイナリを `/opt/ROOT` へ。`root -b -q -e 'TH1D ... FillRandom'` | ✅ 起動・cling OK。未解決 ldd は不要な libR / libRInside / libunuran / libXrd* のみ |
| §6 リポ | 手元が origin より **194 コミット先行**(未 push)→ push せず **git bundle** で `~/tpcdaq-rs` に(`git clone -b main`) | HEAD `546f31a` |
| §6 cargo | `cargo build --release` | ✅ 30 s |
| §6 cargo | `cargo test`(ソフト上限 1024 のまま) | ❌ `controller_integration` が `Too many open files (zeromq-src epoll.cpp:65)` → SIGABRT(それまでの 19 スイート 652 件は green) |
| §6 cargo | `ulimit -n 65536; cargo test --no-fail-fast` | ✅ **58 スイート 848 passed / 0 failed / 1 ignored** |
| §6 実データ | `TPCDAQ_REAL_GRAW`(パルサー 30,108,684 B)+ `TPCDAQ_REAL_GEOMETRY_MINI` + `TPCDAQ_ROOT_SINK_BIN` で `cargo test --release` 全数 | ✅ 848 / 0 / 1。個別 `--nocapture` で **events 108 / items 15,040,512 / malformed 0**(decoder)、graw 書き出しバイト一致・seq_gaps 0、mini raw-tree e2e = 108 events complete / incomplete 0 / late 0、`run0001.root` 108 entries |
| §6 SKIP | `TPCDAQ_FAKE_ECC_BIN` 未設定(p3_e2e の E2E-C/D/G/H、p3_error_paths F0–F2、soak smoke)、`TPCDAQ_REAL_GEOMETRY_ELITPC` 未設定 | fake_ecc は Ice 3.8 の C++ テストダブルで本番には不要(Mac で検証済みの範囲) |
| §4 root_sink | `make TPCDAQ_TPCRECO_DIR=/opt/tpcdaq/assets/TPCReco-HIGS2026_online -j` | ✅ **gcc 15 で警告・エラーゼロ** |
| §4 root_sink | `make test` | ✅ tpc_wire 68 / rs_core 71 / eb_core 175 / geo 426 / monitor_hist 202 / monitor_pub 92(conformance はゴールデン無しで SKIP) |
| §4 root_sink | `make test-root` | ✅ recorder 170 / pevent 99 / raw_tree 88(実 PEventTPC 無しの SKIP 1) |
| §8 起動 | `tools/deploy/daq_up.sh`(新規、`546f31a`)で 8 プロセス | ✅ `api/status`: graw-writer / decoder / receiver0 reachable・Idle、**ecc = Idle / NO_ERR**(Rust ecc_bridge → 実 getEccServer)。Mac(同 LAN)から UI `http://<ELITPC>:8080/` HTTP 200・WS 9000 到達 |
| §8 停止 | `tools/deploy/daq_down.sh` | ✅ 8 本とも graceful(SIGKILL 無し)、`stopped — no leftover processes`、全ポート閉 → 再度 up して**起動したまま** |

配置(リポ外): `/opt/tpcdaq/daq.env`・`/opt/tpcdaq/daq.toml`(chmod 600、パスフレーズは `secrets.token_urlsafe(12)` で生成して TOML にのみ記載)、
`/opt/tpcdaq/assets/`(実ジオメトリ `.dat`、パルサー `.graw`、TPCReco 部分集合)、`/opt/tpcdaq/ui`(Mac ビルドの dist、09-23 版 = ui/src 最終変更と同じ)、
データ・ログ `/srv/tpcdaq`(`logs/`、`params/`)。daq.toml: `listen = 0.0.0.0:46005` + **`router_ip = 192.168.4.2`**(NIC 設定前でも上がるように)、
`config_id = zCobo / physics / physics`、`ecc_config_dir = ~/ELITPC/GetSoftware_config`(GDED のコピー)。

## データ置き場

NVMe がルート ext4 1 枚なので **`/srv/tpcdaq` = ルート上のディレクトリ**で進めた(推奨案をユーザーに提示、異論なし)。
daq が書くので ext4 の予約 5 %(≈ 47 GB)が OS 側に残る。α 線源 run(1.5〜19 GB/日)なら 874 GB で 46 日以上。高レート運転の前に別ボリュームへ(手順書 §11)。

## 時刻同期の続き — NTS 源を足すだけでは同期しない

素の NTP 源(ntp.ubuntu.com × 4)は Reach 377 で届いているのに全部 `^-`(不採用)、`Leap status: Not synchronised` のまま。
chrony 4.8 の既定 `authselectmode mix` は NTS(認証付き)源に `require` を付ける = **NTS 源が 1 つも選べない限り同期しない**。
→ NTS の源ファイル `ubuntu-ntp-pools.sources` を `.disabled` に改名して `chronyc reload sources` が要る(sudo)。手順書 v0.5 §1 に反映。
**ユーザー実施(2026-10-08 15:3x)→ ✅ 同期**: 4 源とも Reach 377、`^*` = 185.125.190.58(prod-ntp-5.ntp4.ps5.canonical.com)、Stratum 3、
Last offset −7.3 ms、`Leap status: Normal`、`timedatectl` = `System clock synchronized: yes`。

## 移設と zCoBo 接続(2026-10-08 18:00 EEST〜)

ユーザーが機体を GDED の場所へ移設 → **`ssh daq@192.168.144.220`**(LAN = `enp179s0f1` DHCP)、zCoBo は `enp179s0f0` に直結済み(ユーザーは ping のみ確認)。

- `enp179s0f0`(NM 接続 `netplan-enp179s0f0`): **192.168.4.1/24・MTU auto(1500)・`ipv4.gateway = 192.168.4.1`(自分自身)** →
  `default via 192.168.4.1 dev enp179s0f0 metric 20102` ができている(GDED の「Profile 1」と同じ形。metric が大きいので実害は今は無い)。
  IP は 4.1 のまま使う → daq.toml の `router_ip` を **192.168.4.1** に変更。ゲートウェイ除去と MTU 9000 は sudo(ユーザー依頼)。
- zCoBo: `ping 192.168.4.81` 0.16 ms、DF 1472 OK。4.2 / 4.3(GDED / VM)は無応答。GDED の `enp1s0f0/1` は carrier 0、VM `Ubuntu16.04LTS_ELINP` は起動したまま
  (TPC_Share への書き込みは 16:18 の `CoBo_2026-10-07T10:04:32.170_0001.graw` で止まっている)→ VM の ECC が zCoBo に届く経路は無い。
- スタック起動(daq_up.sh)→ ecc_bridge に直接 `describe(zCobo)` → **`Connected to 1 CoBo board(s) and 0 MuTanT module(s)`**(0.07 s)→ `reset` で Idle。
  ECC ログの `isActive cannot be resolved` は describe-zCobo の既知の警告。
- **設定の取り違えを発見**: GDED から持ってきた `configure-physics.xcfg`(md5 1d7b8854)は
  `reference/config/configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg` と同一 = **HIgS 2026 の ELITPC 用**。
  VM の GET が使っているのは workspace.xcfg の `/home/getuser/Desktop/TPCConfigFiles/configure-physics-extTrig_240fC_25MHz_70ns.xcfg`(VM 内)。
  GDED ホストの `~/Documents/TPCConfigFiles/` にある同名ファイルは md5 51fd8666 = 2018 年の FW 同梱版と同一(VM 内の版が改変されているかは未確認)。
  → 誤って HIgS 設定で configure しないよう、`prepare/configure-physics.xcfg` を `*-physics-higs2026.xcfg` に改名、daq.toml の
  `config_id` を `zCobo / elinp / elinp`(**ファイルはまだ無い** = run/start は prepare で止まる)にしてスタック再起動。雛形 `config/daq_example.toml` もプレースホルダに。

## 残り(ユーザー立ち会い / 移行日)

1. ~~chrony~~ ✅ 済(上記)。
2. ~~線の付け替え~~ ✅(ユーザー、4.1/24)。残り = NIC のゲートウェイ除去 + MTU 9000(sudo)。
3. ~~describe~~ ✅(VM 側はリンク断で zCoBo に届かない状態で実施)。VM の GET は止めておく(線を戻したときの二重接続防止)。
4. ECC 設定: **VM 内の `configure-physics-extTrig_240fC_25MHz_70ns.xcfg` を TPC_Share 経由で取得** → `prepare-elinp` / `configure-elinp` に。pulser 用も VM 版(TPC_Share の configure-pulser.xcfg、md5 86a8ab10)と GDED 版(e1043496)が違う。
5. §10 疎通: DF ping 8972 → パルサー run → `/srv/tpcdaq/run0001/` の graw + ROOT → UI → stop/reset の walk-back。
6. 不感時間の比較: 移行後の線源 run で 106 の VM 律速(≈ 0.4 Hz 上限)が消えるか。
7. 後回し: slowctl(§9)、systemd ユニット、ufw。
