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

## VM の現用設定の取得(2026-10-08 夜、VBoxManage guestcontrol)

ユーザーが getuser のパスワードを GDED の `~/.vm_getuser_pw`(600)に置き、`VBoxManage guestcontrol Ubuntu16.04LTS_ELINP ... --passwordfile` で
VM のネットワーク無しに読めた(Guest Additions 7.1.6、RunLevel 2)。`copyfrom --recursive --target-directory` は**ディレクトリの中身を直下に**置く。
`run --exe X -- args` の args は argv[1] から。

- VM の `/home/getuser/Desktop/TPCConfigFiles/`(17 ファイル)→ GDED `~/vm_TPCConfigFiles_2026-10-08/`。
- 現用 = **`delay_test.xcfg`**(ユーザー確認)= **CKW 100 MHz / peaking 232 ns / 120 fC / external / triggerDelay 3.2 µs**(詳細と 101 への影響は 101 追記 2)。
- VM のパルサー設定 `GetSoftware_config/configure-pulser.xcfg` = md5 86a8ab10(TPC_Share の写しと同一、GDED 版 e1043496 とは別)= onSlowControl / 100 MHz / peaking 1014 ns / 120 fC。
  describe-zCobo・hardwareDescription_zCobo は ELITPC 版と同一。
- ELITPC に配置: `prepare-elinp` / `configure-elinp` = delay_test、`prepare-pulser-vm` / `configure-pulser-vm` = VM のパルサー。原本は `~/ELITPC/vm_configs_2026-10-08/`。
  daq.toml の `config_id` は既に `zCobo / elinp / elinp`。

## 初の実機 run(2026-10-08 18:22〜、ユーザー許可「1, 2 ともに」)

- **run 1 = パルサー**(config `zCobo / pulser-vm / pulser-vm`、`asad-pulser-start -1000 1000`、30 s): run/start **7.47 s** → Running、
  run/stop `eos_closed:true / forced_eos:true / reason normal`。出力 `/srv/tpcdaq/run0001/`: `CoBo0_AsAd0_2026-10-08T18:22:45.532_0000.graw`(8,363,520 B)、
  `run0001.root` 19.0 MB、`run0001_raw.root` 5.7 MB。graw を全数デコード: **30 frames(frameType 2、278,784 B、items 139,264)、eventIdx 0–29 連番、
  CoBo タイムスタンプ間隔 30 本とも 1.000 s** = 取りこぼしゼロ。
- **run 2 = α 線源**(config `zCobo / elinp / elinp` = VM の delay_test.xcfg)18:23:51 開始: DataLinkSet `192.168.4.1:46005 TCP`、
  zCoBo が 7 s 後に接続(`CoBo connected peer=192.168.4.81`)、最初のフレームは開始 38 s 後(557,312 B = 全読み出し)。
  **ユーザーの退席に合わせて走らせたまま**、制御トークンは release(UI から acquire して止められる)。比較(不感時間・レート vs VM)は run 2 を止めてから。
- **CPU(run 2 開始 557 s 時点、26 events)**: 8 プロセスの累積 CPU 計 9.8 s(root_sink 7.05 = ROOT 初期化と圧縮、monitor 1.43、他は各 < 0.4)= 1 コアの 1.8 %。
  アイドル分込みの上限でも **≈ 0.38 コア秒 / event**(VM は 106 で ≈ 5 コア秒 / event)。RSS は root_sink 565 MB、他は各 6〜17 MB。
  ユーザー観察「htop で全く CPU 食わない、元のは描画の負荷?」→ 最有力: VM の `GetController.conf` が `[data] monitoring=true / showAsciiHeader=true`
  (GPU 無し VM で 272 ch × 512 の波形描画 + ヘッダ表示、データルータも GetController 内蔵 = `external=false`)。VM 側で monitoring を切った比較はしていない。
- **イベント間隔(同 26 events)**: eventIdx 0–25 連番、平均 0.049 Hz、最小間隔 **0.888 s**(1 s 未満が 25 中 1。VM では約 1.0 s の固定不感時間 — 104)。
  Poisson なら 1 s 未満は 4.8 % ≈ 1.2 本の期待 = 不感時間なしと矛盾しないが統計不足 → 一晩分(数千 events)で判定。
- 切替のたびに daq.toml の `config_id` を書き換えてスタックを再起動している(078 の params / REST で切り替える運用は未整備)。

## 一晩の α 線源 run 2(2026-10-08 18:23 → 10-09 07:51、48,459 s)

- 健全性: receiver 1,695 frames / 944,643,840 B、`overflow_frames 0 / framer_resets 0 / messages_abandoned 0`、全ログに WARN/ERROR 無し、ECC Running / NO_ERR。
  CPU 累計 root_sink 664 s・monitor 122 s・decoder 32 s・receiver 29 s・graw_writer 14 s・controller 9 s・ecc_bridge 4 s(計 ≈ 874 s = 1 コアの 1.8 %、≈ 0.5 コア秒 / event アイドル込み)。
  RSS は root_sink 670 MB(開始時 565)、他は不変。ディスク使用 28 → 30 GB。
- **イベント間隔**(`gaps.py`、graw ヘッダの CoBo タイムスタンプ、Poisson 期待 = 不感時間なし):

  | | events | rate | 0–0.5 s | 0.5–1.0 s | 1.0–1.5 s | 1.5–2.0 s | 2.0–3.0 s | 最小間隔 |
  |---|---|---|---|---|---|---|---|---|
  | **ELITPC run 2**(tpcdaq-rs) | 1,695 | 0.0350 Hz | **0** / 29.4 | **3** / 28.9 | 35 / 28.4 | 40 / 27.9 | 58 / 54.3 | 0.888 s |
  | VM 10-07 run(GET、`_0000`+`_0001`) | 3,463 | 0.0329 Hz | 4 / 56.5 | 8 / 55.6 | 78 / 54.7 | 59 / 53.8 | 130 / 105.0 | 0.048 s |

  → **約 1 s の不感時間は ELITPC でも同じに残る**(106 の「VM 内の処理」とは食い違う。**ユーザー指示: 104 / 106 の訂正は試験で確かめてから** — 一度入れた訂正 `cd0d086` は取り消した)。eventIdx は両方とも連番 = zCoBo が受け付けなかったトリガは番号も付かない。
  1.0–2.0 s の過剰は不感時間中に来たトリガが失われる + ²²⁰Rn → ²¹⁶Po(半減期 0.145 s)の相関対が落ちる分の名残。レートは 0.035 vs 0.033 Hz で、この低レートでは 1 s の不感時間による損失は数 % しかない。
- **最有力の被疑者 = `triggerPeriod_10ns`(CoBo レジスタ 0x84)**: GET の `CoBoNode::configure` は**トリガモードに関係なく**
  `setTriggerPeriod()` を呼び、`triggerPeriod`(ms)× 100,000 を書く(`reference/20190315_patched/GetBench/src/get/rc/CoBoNode.cpp:538, 3192–3203`)。
  delay_test / VM のパルサー / 2018 FW 同梱の external 用 configure-pedestals まで**全部 1000 ms**。xcfg のコメントは「periodic モード用」だが、
  ZC706 移植 FW が external でも最小間隔として使っている疑い。他に ~1 s の項目は無い(triggerDeadTime 等は 2p 専用、TriggerVetoOption none)。
- 確かめ方(ユーザーの了承待ち): run 2 を止め、`triggerPeriod` だけ 10 ms に変えた設定でパルサーを 200 ms 周期(5 Hz)で打つ → 1 Hz に頭打ちなら別原因、
  5 Hz 通れば確定。その後 α 線源で同じ変更を入れて間隔分布の 0–1 s が Poisson 期待に戻るかを見る。

## レートは上がったか(ユーザー質問、2026-10-09)

`rateblocks.py`(graw ヘッダ、3 時間ブロック):
- VM 10-07 run: 平均 **0.0329 ± 0.0006 Hz**、ブロック 0.0301 / 0.0367 / 0.0362 / 0.0328 / 0.0313 / 0.0297 / **0.0253 / 0.0254** / 0.0382(10:04 開始、各 ±0.0017)。
- ELITPC run 2: 平均 **0.0349 ± 0.0008 Hz**、ブロック 0.0402 / 0.0375 / 0.0364 / 0.0294(18:24 開始)。
→ 平均の差 +6 % は、同じ run の中の時間変動(0.025〜0.040 Hz、統計誤差の数倍)の内側 = **DAQ を替えたことによるレート増は見えない**。この低レートでは
DAQ は律速していない(1 s の不感時間の損失 ≈ 3.5 % は両方同じ、VM の処理の上限 ≈ 0.4 Hz は遥か上)。時間変動は早朝(04–10 時)に低く午後〜夜に高い傾向
= 温度などの日変化の疑い(ガス温度の測定待ち)。

## 不感時間の切り分け試験(2026-10-09 朝、ユーザー指示「試してから」)

run 2 を 07:53 に停止(`run0002.root` 1,697 entries complete / incomplete 0 で確定。1 GB の close に数秒)。`~/pulser_test.sh <config_id> <周期 ms> <秒> "<comment>"`
(daq.toml の config_id 書き換え → daq_down/up → acquire → run/start → `asad-pulser-start -1000 <周期>` → sleep → `asad-pulser-stop` → run/stop → release)で
パルサーを **200 ms 周期(5 Hz)× 30 s**。graw ヘッダで受理数と間隔を数えた:

| run | 設定(pulser-vm からの差分) | frame | 受理 | 間隔 |
|---|---|---|---|---|
| 3 (A) | なし(triggerPeriod 1000 ms) | type 2 / 278,784 B | 150 / 150 | 全部 0.20 s |
| 4 (B) | triggerPeriod 10 ms | type 2 / 278,784 B | 150 / 150 | 全部 0.20 s |
| 5 (C1) | isAllChannelRead false + GlobalThreshold 1(= 線源設定の読み出し) | **type 1 / 557,312 B** | 150 / 150 | 全部 0.20 s |
| 6 (C2) | subscribe + checkAsAd true | type 2 / 278,784 B | 150 / 150 | 全部 0.20 s |

→ **パルサートリガ(onSlowControl)では ~1 s の不感時間は出ない**。triggerPeriod も効かない。zCoBo は 557 kB フレームを 5 Hz(2.8 MB/s)で落とさず送れる。
frameType 1 / 2 の差は isAllChannelRead(部分読み出し = アドレス付き 4 B/item)で決まる(線源設定は閾値 1 で全 ch ヒットのため 272 × 512 が 4 B 形式で来る)。
**残る差はトリガ経路(external = IN1)だけ** → triggerPeriod 仮説は external モードについては未検証のまま。

- **run 7 (E)** 2026-10-09 ~08:05 開始: α 線源、external、`elinp-tp10`(delay_test の triggerPeriod だけ 1000 → 10 ms)。
  数時間で 0–1 s の間隔が Poisson 期待(≈ 3.4 %)に戻るかを見る。戻らなければ被疑者は FW の external 経路か上流(ゲートジェネレータ等)。
  決定打が早く欲しければ、現地で IN1 にパルス発生器(例 5 Hz)を入れて elinp / elinp-tp10 を 30 s ずつ。

## macros で解析できるか(ユーザー質問、2026-10-09)

ELITPC 上で `run0002_raw.root`(root_sink `--raw-tree`)を **graw2root を飛ばして** analyzeUVW.C → makeTracks.C に直結:
1,697 events → `run0002_uvw.root` → `run0002_tracks.root` **ok 1,690 / 1,697**(analyzeUVW 2.9 s、makeTracks 0.9 s、ROOT 6.40.04 / gcc 15)。
`channel_map.csv` は gitignore の内部資産 → `~/ana_107/` にだけ置いた(600)。解析は `/srv/tpcdaq` を汚さないよう `~/ana_107/` にシンボリックリンクで。
注意: analyzeUVW の時間 → z は 25 MHz 前提(101 追記 2: 実機は 100 MHz)と圧力既定 100 mbar のまま。

## 現地試験: IN1 にパルス発生器(2026-10-09 08:58〜、ユーザー現地 + ノート PC 同行)

- run 7(E、tp10、α 線源)は 08:58:16 に停止: 111 events / 3,306 s(0.0333 Hz)、**1 s 未満の間隔 0 本**(Poisson 期待 3.6、p ≈ 3 %)、最小 1.567 s
  → triggerPeriod 10 ms でも不感時間は消えない方向。
- ユーザーがゲートジェネレータの代わりにパルス発生器を IN1 へ: **5 Hz、当初 0–4 V → 指摘して 0–3.2 V に下げた**(README は +3.3 V TTL、xcfg コメントは 2.5 V LVCMOS。
  FPGA 直結なら 4 V は絶対最大定格超えの恐れ — アダプタのバッファ有無は資料で未確認)。パルス幅は発生器の固定値(変えられない)。
- `~/ext_watch.sh`(5 s ごとの受理レート表示付き)で **run 8 = X1: `elinp`(triggerPeriod 1000 ms のまま)、30 s** →
  **154 events / 30.60 s、eventIdx 0–153 連番、間隔 153 本すべて 0.200 s** = 5 Hz を全数受理。
- **結論: zCoBo の外部トリガ経路にも tpcdaq-rs にも ~1 s の不感時間は無い**。triggerPeriod も無関係(X2 = tp10 は不要になった)。
  → ~1 s の不感時間は**トリガ信号の上流**(CFD → ゲートジェネレータ、またはその出力の形 = 幅が ~1 s 近い等)にある。

- 途中で ELITPC の LAN が不通(古い LAN ケーブル)→ ユーザーが GDED のケーブルと交換して復旧(GDED 側が要注意)。
- **run 9**(目隠し): パルス発生器 5 Hz TTL → CAEN TTL→NIM → CAEN NIM→TTL → IN1 = **30 s で 0 発**(zCoBo 接続・ECC Running は正常)。
- **run 10**: パルス発生器を NIM(5 Hz)にして CAEN NIM→TTL 1 段 → IN1 = **30 s で 0 発**(同上)。
  本番(ゲートジェネレータ NIM → 同じ NIM→TTL → IN1)では α のトリガが来ているので、変換器の出力が常に使えないわけではない
  → パルス発生器の NIM 振幅(50 Ω 前提の表示か)・幅、変換器出力の電圧と極性をオシロで確認待ち。

- **run 11**: 0 発(ユーザー: 配線がちゃんとできていなかった)。
- **run 12(決定打)**: ユーザーが **CAEN の NIM→TTL 変換器を別のものに交換**し、本番のチェーン(**α 線源 → CFD → ゲートジェネレータ → 新しい NIM→TTL → IN1**)で 30 s:
  **139 events / 30.46 s = 4.53 Hz**、eventIdx 連番、間隔は**指数分布にぴったり**(0.1 s 刻み 52/31/21/15/4 … vs 期待 50.3/32.0/20.3/12.9/8.2 …)、**最小 4.9 ms**。
  → **これまでの 0.035 Hz(VM・ELITPC とも)と ~1 s の不感時間は、どちらも CAEN の NIM→TTL 変換器が原因**(130 倍のトリガが失われていた)。
  ユーザー確認「これはソースからの信号です」。104 / 105 / 106 の「~1 s の不感時間」「レートが期待の ~1/100」の主因はこれ(archive の訂正は飛跡の確認後にまとめて)。
- **run 13** 09:51:33 開始: α 線源、`elinp`、新しい変換器、91 mbar。飛跡が本物かを数分分で確認する。
- 容量: 1 event ≈ graw 557 kB + run.root ≈ 610 kB + raw ≈ 145 kB ≈ 1.3 MB → **4.5 Hz で ≈ 5.9 MB/s ≈ 510 GB/日**。空き 860 GB = **約 40 時間**。
  週末(〜72 h)を回すなら出力を絞る必要あり(run.root を止める `--no-root` で ≈ 0.7 MB/event ≈ 270 GB/日、graw だけなら ≈ 220 GB/日)。

- run 13 の中身確認(開始 3 分、graw2root → analyzeUVW(91 mbar)→ makeTracks → sourceTracks → stoppedAlphas(−386, 91)):
  924 events / ok 916 / 線源方向 672(73 %、run 2 は 71 %)。停止線 ²¹²Bi −24.4 / ²²⁰Rn −3.5 / ²¹⁶Po +41.4 mm(run 2: −25.5 / −5.0 / +42.5)= **同じ α が 130 倍の量**。
  graw2root はファイル名末尾 `_0000.graw` を前提(仮名のシンボリックリンクだと `stoi` で落ちる)。
- **容量 → ユーザー裁定(10-09): ZFS を使う(本番機として ZFS に保存する予定だった。これまで不要だっただけ)**。オフィスに戻ってから実施。
  事前確認: カーネル 7.0.0-38 に `ubuntu/dkms/zfs/zfs.ko.zst` 同梱、`zfsutils-linux` 2.4.1 候補あり、sda/sdb/sdc は `zfs_member data` のまま。
  SPEC §3 の例も `output_root = "/data/tpcdaq"`。手順案: `apt install zfsutils-linux` → `zpool import`(一覧)→ `zpool import -f data`(ホスト ID 変更のため。**upgrade しない**)→
  `zfs create -o compression=lz4 -o atime=off -o recordsize=1M -o mountpoint=/data/tpcdaq data/tpcdaq` + chown daq →
  run 13 停止 → `tpcdaq_state.json`(next_run)・`logbook.jsonl`・`params/` を新 output_root へ写す → daq.toml の output_root 変更 → 再起動 → run 14。

## 別件メモ(後で)
- UI の Parameters(078)の既定値が `writingClockFrequency_MHz: 25.0`(HIgS の configure-physics 由来)。mini の現用は 100 MHz(101 追記 2)
  → Parameters から保存すると 25 MHz の xcfg ができる。mini で使う前に既定値をテンプレート(elinp)から取るよう直す。

## 残り(ユーザー立ち会い / 移行日)

1. ~~chrony~~ ✅ 済(上記)。
2. ~~線の付け替え~~ ✅(ユーザー、4.1/24)。~~NIC のゲートウェイ除去 + MTU 9000~~ ✅(ユーザー実施:
   `nmcli con mod netplan-enp179s0f0 ipv4.gateway "" ipv4.never-default yes 802-3-ethernet.mtu 9000` → `con up`)。
   確認: 4.x の default route 消滅、mtu 9000 / 1000 Mb/s、**DF ping 1472 / 8972 とも 3/3**(rtt 0.24 / 0.47 ms)= §2 ✅。スタックは再起動不要(ecc Idle / NO_ERR のまま)。
3. ~~describe~~ ✅(VM 側はリンク断で zCoBo に届かない状態で実施)。VM の GET は止めておく(線を戻したときの二重接続防止)。
4. ~~ECC 設定~~ ✅(guestcontrol で取得、上記)。旧記述: **VM 内の `configure-physics-extTrig_240fC_25MHz_70ns.xcfg` を TPC_Share 経由で取得** → `prepare-elinp` / `configure-elinp` に。pulser 用も VM 版(TPC_Share の configure-pulser.xcfg、md5 86a8ab10)と GDED 版(e1043496)が違う。
5. §10 疎通: DF ping 8972 → パルサー run → `/srv/tpcdaq/run0001/` の graw + ROOT → UI → stop/reset の walk-back。
6. 不感時間の比較: 移行後の線源 run で 106 の VM 律速(≈ 0.4 Hz 上限)が消えるか。
7. 後回し: slowctl(§9)、systemd ユニット、ufw。
