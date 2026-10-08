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

## ユーザーに依頼する sudo 作業(1 回にまとめる)

```sh
sudo apt update
sudo apt install -y build-essential git cmake curl python3-zmq \
  libtbb12 libgsl28 libgslcblas0 libblas3 libglu1-mesa libgl2ps1.4 libftgl2 libopengl0 libgfortran5 libcfitsio10t64
echo 'pool ntp.ubuntu.com iburst' | sudo tee /etc/chrony/sources.d/plain-ntp.sources
sudo chronyc reload sources
sudo usermod -aG dialout daq
sudo install -d -o daq -g daq /opt/ROOT /opt/tpcdaq /srv/tpcdaq
echo 'source /opt/ROOT/bin/thisroot.sh' | sudo tee /etc/profile.d/root.sh
```

## 未決(ユーザー判断)

- データ置き場: NVMe がルート 1 枚なので、`/srv/tpcdaq` をルート上のディレクトリにする案(ext4 の予約 5 % ≈ 47 GB は root 専用 =
  daq の書き込みで満杯になっても OS 側の余地は残る。α 線源 run の 1.5〜19 GB/日なら 874 GB で 46 日以上)を推奨。
  パーティションを分け直すならライブ USB から縮小が要る。

## 次

sudo 作業後: ROOT を `/opt/ROOT` へ移して `root -b -q` / tpcdaq-rs を git clone → `cargo build --release && cargo test` →
`tools/root_sink` を `make -j && make test && make test-root`(gcc 15)→ UI は Mac でビルドした dist を持ち込み → §2 netplan(zCoBo 直結、ユーザー立ち会い)。
