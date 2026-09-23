# 新 DAQ 機セットアップ手順(Ubuntu 24.04 LTS)— v0.2(2026-09-23: Rust ブリッジ・slowctl・params を反映)

**位置づけ**: TODO/080 A-1。順序は TODO/076 §6 の裁定どおり —
**OS → ネットワーク → ツールチェーン → ROOT → 実 ECC(ホストでネイティブ、Ice 3.7)→ tpcdaq-rs → UI →
ECC ブリッジ(Rust 076、当面は C++ 併存)→ 疎通チェック → 常駐化**。Ice 3.8 は Rust ブリッジが実機で通れば不要。
根拠 = 2026-09-14 の GDED 遠隔ベンチ(TODO/069 結果節、メモリ geteccserver-native-gded)。
**実験の実パス・実 IP・パスフレーズは本文に書かない**(環境変数と設定ファイルで与える — CLAUDE.md)。
各節末尾の ✅ が完了条件。1 つでも ❌ のまま次へ進まない。

## 0. 用意するもの

| もの | 例 | 備考 |
|---|---|---|
| 機体 | デスクトップ + オンボード/PCIe NIC ×2(LAN 用 + zCoBo 直結用) | USB Ethernet は使わない(073 §E の教訓) |
| ディスク | データ用に別ボリューム(mini 28 MB/s 持続、run 1 本 ≈ 数 GB) | `noatime`、空き監視 |
| 資産(リポ外) | 実 mini ジオメトリ `.dat`、ECC 設定一式(describe/prepare/configure xcfg)、GET 20190315_patched(ソース + 16.04 ビルド済み) | reference/ 相当。USB か scp で持ち込む |
| ケーブル | zCoBo 直結 RJ45(1000BASE-T)、USB-RS232(Bronkhorst 7.03.366 + 9.09.122)、USB-B(HMP2020)、Arduino USB | 079 |

## 1. OS とアカウント

- Ubuntu 24.04 LTS Server(GUI 不要。ブラウザは別 PC から)。ユーザー `daq`(sudo 可)。
- `sudo apt update && sudo apt install -y build-essential git cmake pkg-config curl python3 python3-pip python3-yaml openssh-server chrony`
- 時刻: `chrony` を実験 LAN の NTP か公開 NTP に。**ログブックの ts は RFC3339 オフセット付き**なので TZ を決めて固定
  (`timedatectl set-timezone Europe/Bucharest` 等)。
- `daq` を `dialout` に(シリアル用): `sudo usermod -aG dialout daq`(再ログイン)。
- ✅ `timedatectl` が同期済み / `groups` に dialout。

## 2. ネットワーク

- zCoBo 直結 NIC: `192.168.4.2/24`、**MTU 9000**、ゲートウェイ無し(GDED の enp1s0f1 と同じ)。netplan 例:
  ```yaml
  network: {version: 2, ethernets: {enp1s0f1: {addresses: [192.168.4.2/24], mtu: 9000, dhcp4: false}}}
  ```
  LAN 側は別 NIC(DHCP 可)。**この NIC には自分経由のルートを作らない**(NM の「Profile 1」事故 = メモリ参照)。
- 確認: `ping -M do -s 8972 192.168.4.81`(zCoBo getHwServer 側の IP。実機は 4.81 か 4.84 — 現物で確認)。
  `ip link show <nic>` で mtu 9000 と 1000Mb/s。
- ファイアウォール(任意、ufw): 8080(REST)/ 9000(WS)は LAN から、46005(データ)は 192.168.4.0/24 からのみ。
- ✅ DF ping 8972 が通る / 1500 も通る。

## 3. ツールチェーン

- Rust: `curl https://sh.rustup.rs -sSf | sh` → stable(手元 1.97 系)。`zmq` クレートは vendored(`zeromq-src`)なので
  libzmq の apt は不要(GDED で実証、ビルド 1 分)。
- Node.js 22 LTS(UI ビルド用。Angular 22 系): NodeSource か `nvm`。ビルドは Mac でやって `ui/dist` を持ち込んでもよい。
- `python3-zmq`(ベンチ/疎通スクリプト用): `pip install pyzmq` か apt `python3-zmq`。
- ✅ `cargo --version` / `node --version`(≥ 22)。

## 4. ROOT(root-sink 用)

- 公式バイナリ(Ubuntu 24.04 / gcc13 用 tarball、6.36 以降)を `/opt/ROOT` に展開。`/etc/profile.d/root.sh` に
  `source /opt/ROOT/bin/thisroot.sh`。ソースビルドは不要(TPCReco の旧 ROOT 互換は**出力ファイル側**で担保 — 014 の ZLIB-1)。
- 確認: `root -b -q` が起動、`root-config --cflags --libs`。
- tpcdaq-rs 側: `cd tools/root_sink && make -j && make test && make test-root`(test-root は recorder 170 / pevent 99 /
  raw 88 が green のはず)。
- ✅ `tools/root_sink/root_sink --help`。

## 5. 実 ECC(getEccServer 20190315)をホストにネイティブ導入

2 通り。**A が最短(GDED で実証済み)**、B は恒久版。

**A. 16.04 ビルド済みバイナリ + 不足ライブラリの非 root 展開**(メモリ geteccserver-native-gded の手順そのまま)
1. `20190315_patched.tgz`(ソース + `build/.libs`)を `~/ELITPC/GetSoftware_bin/20190315/` に展開。
2. 不足 = Boost 1.58 / libssl 1.0.0 / libbz2 → old-releases.ubuntu.com(xenial)の deb を `apt-get download` →
   `dpkg-deb -x` で `~/get_docker/stage/` に展開(インストールしない)。
3. **Ice 3.7.2**(改名前)を VM(Ubuntu16.04LTS_ELINP)から採取して `~/get_docker/vm_ice/`。noble の 3.7.10 は
   `IceUtil::SyscallException` 改名で ABI 非互換(**罠**)。soname が `.so.3.7.2` のみなら `libIce.so.37` 等を symlink。
4. `LD_LIBRARY_PATH=vm_ice:GETLIB:stage/usr/lib/...:stage/lib/...` で
   `getEccServer --config-repo-url ~/ELITPC/GetSoftware_config`。
5. 確認: `ss -ltn | grep 46002`、`printf 'sm-status\nexit\n' | getEccClient`(xenial の libreadline6/libtinfo5 も dpkg-deb -x)。

**B. ソースビルド(Ice 3.7.10 + Boost 1.83、`reference/_spike/build_all.sh` の Linux 版)**
- `sudo apt install libzeroc-ice-dev zeroc-ice-slice libboost-all-dev liblog4cxx-dev libxerces-c-dev`(不足は configure が言う)。
- macOS で要ったパッチは 2 行(`Factory.hpp` include / `PeriodicTask.cpp` io_context)+ configure の Ice パス再標的。
  **3.7.10 の `SyscallException` 改名が GET ソースに当たるかは未確認** — 当たったら `IceUtil::` → `Ice::` の機械置換で済むはず。
  結果を本書に追記すること。
- ✅ どちらでも: `0.0.0.0:46002` LISTEN、`sm-status` = `IDLE / NO_ERR`、`config-list` が持ち込んだ xcfg のトリプレットを返す。

**設定リポジトリ**: `describe-<id>.xcfg / prepare-<id>.xcfg / configure-<id>.xcfg` の命名は厳格(ECC は
`prepare-<id>` が無いと prepare で失敗 — GDED では configure の写しを置いた)。id の組は controller の
`config_id`(3 相同値)か `config_ids`(非同値、SPEC v1.13)。

## 6. tpcdaq-rs

```sh
git clone https://github.com/aogaki/tpcdaq-rs && cd tpcdaq-rs
cargo build --release && cargo test           # 812 件 green(2026-09-23 時点)
cd tools/root_sink && make -j && cd -
cd ui && npm ci && npm run build && cd -       # → ui/dist/ui/browser(controller が配信)
```
設定 TOML(GDED の `bench.conf` が雛形。SPEC §3):
```toml
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"                     # データボリューム
geometry = "/opt/tpcdaq/assets/new_geometry_mini_eTPC.dat"   # リポ外
[[cobo]]
id = 0
listen = "0.0.0.0:46005"
data_sender_id = "CoBo[0]"
[decoder]
workers = 1
[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
raw_tree = true                                  # run{N}_raw.root(macros 直結、075)
[monitor]
ws_listen = "0.0.0.0:9000"
slow_sub = "tcp://127.0.0.1:47006"               # slowctl の status PUB(v1.31)。slowctl を動かすまでは省略可
[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "<環境変数か別ファイルから>"
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"
config_id = "pulser"                             # 実験に合わせる(078 のプリセットが来るまで手動)
ui_dir = "/opt/tpcdaq/ui"                        # ui/dist/ui/browser のコピー
ecc_config_dir = "<ECC の config repo>"          # v1.33: テンプレート xcfg の置き場 = 生成 xcfg の出力先(getEccServer --config-repo-url と同じ)
params_dir = "/data/tpcdaq/params"               # v1.33: パラメータ JSON(current.json / <id>.json)
# [slowctl] は現地 Phase 0(node scan、by-id の実名)の後に config/slowctl_example.toml を写して足す(§9)
```
- ✅ `cargo test` green、`root_sink --help`、`ls ui/dist/ui/browser/index.html`。

## 7. ECC ブリッジ

- **Rust が唯一の実装(TODO/076 Phase 3、2026-09-23 に C++ 版を撤去)**:
  `target/release/ecc_bridge --bind tcp://*:47200 --ecc-proxy "Ecc:tcp -h 127.0.0.1 -p 46002"`。
  依存無し(Ice ライブラリ不要 —— `src/ice/` の自前クライアントが encoding 1.1 を直接喋る)。
  実 ECC + 実 zCoBo での 1 周(describe → … → stop → breakup → reset → reset)と
  audit `ecc_walk_back` が C++ 版時と同一であることを実機で確認済み(076 Phase 3 前提)。
- `tools/ecc_bridge/` に残るのは fake-ECC(Ice servant のテストダブル、`fake_ecc.cpp`)と
  e2e クライアント(`ecc_e2e_client.cpp`)だけ —— どちらも Rust bin をローカルで機械照合する
  ための C++/Ice 資産で、本番のブリッジではない。ビルドは
  `cd tools/ecc_bridge && make TPCDAQ_ICE_DIR=<20190315_patched のパス> -j`
  (Ice 3.8 が要る。ECC 本体は Ice 3.7 —— **同一プロセスに混ぜない**、無言 SIGSEGV。
  VIRTUAL_ZCOBO §4.1 の罠。ECC と fake_ecc は別プロセスなので無関係)。
- ✅ `python3 -c "import zmq; s=zmq.Context().socket(zmq.REQ); s.connect('tcp://127.0.0.1:47200'); s.send_json({'action':'status'}); print(s.recv_json())"`
  → `{"ok":true,"state":"Idle","ecc_error":"NO_ERR"}`。

## 8. 起動順序と常駐化

順序は `reference/_spike/demo/start_demo.sh` と同じ(下流から): **root_sink → monitor → graw_writer → decoder →
receiver → getEccServer → ecc_bridge → controller → slowctl**(slowctl は最後でよい: controller 不在中の IDENT/ALARM は再送キューで届く — 079 Phase 2)。停止は逆順、Rust bin は **SIGINT**(SIGTERM ハンドラ無し — 028 付録 A)、
root_sink は両方 graceful。まずは `daq_up.sh` / `daq_down.sh`(GDED の `bench_up.sh` を整えたもの)で運用し、
安定したら systemd ユーザーユニット(`After=` で順序、`KillSignal=SIGINT`)へ。**listen-before-start**(receiver が
46005 を listen してから ECC start)は controller が担保するので、手順として気にするのは「receiver が上がっていること」だけ。
- ✅ `curl -s http://localhost:8080/api/status` が全コンポーネント alive。

## 9. シリアル機器(079)

- `/dev/serial/by-id/` で固定名を確認(`ls -l /dev/serial/by-id/`)。同型で個体シリアルが無いものは `by-path`。
- Bronkhorst: USB-RS232 → 先頭機器 D-sub(ProPar 38400 8N1)。HMP2020: USB-B(CDC → `/dev/ttyACM*`、HO720)。Arduino MKR1000: USB。
- HiVolta: Ethernet(TCP :1470、LOCAL モードのまま読むだけ)。
- ドライバは実装済み(2026-09-23): `bronkhorst_flowbus`(079 Phase 2)/ `arduino_env`(Phase 3、スケッチは `tools/arduino_env/`)/
  `hivolta` / `hmp2020`(089)。**現地 Phase 0 = node scan** は Python を入れず、`[[slowctl.device]] type = "bronkhorst_flowbus", nodes = "scan"` の
  TOML で `target/release/slowctl --config x.toml` を起動し、logbook の `slow` レコード(`event: "IDENT"`)と `RUST_LOG=info` の起動ログで
  ノード番号・機種・シリアル・単位を確認する。確定したら `nodes = [..]` と `expect_serial` を書く。
- ✅ 各ポートで機器が名乗る(ProPar node 128 の 113/3、SCPI `*IDN?`、HiVolta `BDSNUM`、Arduino の JSON 行)= IDENT レコードが揃う。
  不一致は `identity_mismatch` カウンタ(status の `slow.counters`)に出る。

## 10. 疎通チェック(TODO/069 Phase 1–2 の抜粋 — 全部 ✅ で「本番機」)

| # | 確認 | 期待 |
|---|---|---|
| 1 | DF ping 8972 → zCoBo | 通る |
| 2 | `api/status` | 全 alive、ECC state Idle |
| 3 | `describe`(config_id) | Described、ECC ログに `Connected to 1 CoBo board(s)` |
| 4 | pulser run(`asad-pulser-start` は getEccClient か 078 プリセット) | receiver `CoBo connected`、~1 Hz、`malformed=0` |
| 5 | 出力 | `<output_root>/run0001/CoBo0_AsAd0_<TS>_0000.graw`(実機命名)、`run0001.root` + `run0001_raw.root` |
| 6 | UI | 別 PC のブラウザで monitor / waveform / logbook が更新 |
| 7 | stop → reset | audit `ecc_walk_back` が `["status->Ready","breakup->Prepared","reset->Described","reset->Idle"]` |
| 8 | logbook | `run_start` / `run_stop` 行、`forced_eos` / `eos_closed` が期待どおり |
| 9 | slowctl(079) | logbook に各機器の `IDENT`、WS status JSON に `slow`(UI の Slow control ビュー)、`run_start.slow` に直近値 |
| 10 | params(078) | UI の Parameters で保存 → `<ecc_config_dir>/configure-<id>.xcfg` が生え、run/start の audit `params_id` と ECC の configure が `<id>` |

## 11. 未確定・追記予定

- 5-B の 3.7.10 コンパイル可否、ROOT の具体版、systemd ユニットの実体。slowctl / params は実装済み(上記 §6 / §9 / §10-9・10)— 実機で初めて通す。
- 実施ログは TODO/080 A-1 の結果節へ(日付・版・所要時間・つまずき)。
