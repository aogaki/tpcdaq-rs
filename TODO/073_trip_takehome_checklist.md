# 073 — HIGS 2026 出張の持ち帰りチェックリスト(現地でしかできないこと)

**Status: READY**(起票 Fable 2026-08-18。実行者 = Aogaki 本人・現地。
帰還後に取れたものへ ✔ と所在を記入 → 結果節化して archive。
**2026-09-15: 現地フェーズ終了 — 残項目は [080](080_mini_experiment_readiness.md) §1-E に吸収(メール / 遠隔 / 破棄に振り分け)。archive はユーザー指示で**)

**大原則: 走っている実験の設定・プロセスには一切触らない。コピーと観察のみ。**
(9 のみ例外 — ビーム停止窓 + シフトリーダー了解が条件)

## A. コピーするだけ(リスクゼロ、優先順)

- [x] **1. GET 実機ツリー** — **済(2026-08-20。reference/20190315_patched_v2 に配置済みと
  ユーザー申告 → 主対話で全数 diff 実施、疑義決着)**:
  **ソース差分は `GetBench/src/get/daq/FrameStorage.{cpp,h}` の 2 ファイルのみ**
  (2021-04-29 改変 — logbook p.030 の日付と一致)。内容 = 環境変数
  **`GET_SPLIT_OUTPUT_BY_ASAD`** が立っていれば sourceUID=(cobo,asad) で分割し
  `CoBo{K}_AsAd{A}_{TS}_{idx}.graw`、無ければ (0,0) に束ねて
  `CoBo_ALL_AsAd_ALL_{TS}_{idx}.graw`(+ eventIdx リセット時の新ファイル則も split 時のみ)。
  **ECC/MemRead/DataSender/getHwServer/MDaq/mfm 等は 1 バイトも違わない → 我々の
  制御プレーン・プロトコル検証(SPEC §8、vcobo、067 照合)は全て有効なまま**。
  ログブックの「draw2 frame hack」は OCR 乱れで、実体はこの分割 hack 1 件とみられる。
  副産物: 命名 3 系統の出自も確定 — ①`CoBo{K}_AsAd{A}` = dataRouter(v2 FrameStorage +
  env)②`CoBo_ALL_AsAd_ALL` = 同 env 無し(2021 ラボ期の run 名と一致)
  ③素の `CoBo_{TS}` = **GetController TestController の内蔵 raw 保存**
  (TestController.cpp:389 — pedestal/pulser 単一ファイルはこれ。topology frame 込みで
  書く理由も説明がつく)。
- [ ] **2. physics run 追加数本**: 別日・別 trigDelay(1660/1848 系があれば特に)。
  → 066 ハーネスの統計増強 + 窓位置系統の実データ。
  ~~geometry .dat~~ **済(2026-08-20 ユーザー確認)**: 現地は reference/TPCReco の
  latest にある .dat を無改変で使用 = 066 が同定した `geometry_ELITPC.dat` が正。
- [ ] **3. Magboltz 計算テーブル**: 運転点表(logbook p.254/257–259)の元になった
  「MAGBOLTZ22」出力の所在を探してコピー。→ D_L/D_T の一次値(C-6 完全クローズ)。
- [ ] **4. DCS/InfluxDB エクスポート**: physics run 日の圧力・温度時系列 + HiVolta 電流ログ。
- [x] **5. 解析環境の特定** — **済(2026-09-15 ユーザー確認)**: ROOT は 6.08.06 の想定どおり、
  TPCReco は reference/TPCReco と同一とみなしてよい。→ Warsaw 旧 ROOT 互換のターゲット確定
  (std::tuple StreamerInfo 問題 = 066 申し送りの実地照合先)。
- [ ] 6.(軽)ログブック原本の写真: p.030(patched-v2 記載)、p.123–126(ZS 試験)、
  p.254/257–259(運転点表 — 圧力・HV は確認済みなので v_drift 列の確認用)。

## B. 観察するだけ(実験に触らない)

- [x] **7. run 中の `netstat -tn`** — **済(2026-08-20 実測)**:
  データリンクは **`192.168.4.2:46005 ← 192.168.4.84:44951` の 1 本のみ**。
  zCoBo がエフェメラルポートから接続を張る側(= 「リンクを張るのは CoBo 自身」の実機初観測)。
  4 AsAd 全部が単一 TCP に載る。他は SSH(4.2→4.84:22)、HiVolta .30:1470、
  LV SCPI .20/.21:5025、シリアル変換 .104:5301-5303、InfluxDB 8086、Grafana 3000。
- [x] **8. `ps aux | grep -i router`** — **済(同日実測)**:
  `dataRouter 192.168.4.2 192.168.4.2 FDT FrameStorage`(edaq、8/19 起動)。
  **2026 運用の flowType = FDT 確定**(2022 ログブックと同一)。単一リンク →
  CoBo{K}_AsAd{A} 4 ファイルへの分割は dataRouter 側(patched-v2 の extended 版とみられる —
  A-1 の GET ツリー持ち帰りで実装を確認する)。Ice glacier2router も稼働(制御面)。

## C. 人に聞く(現地にいれば)

- [ ] 9. Mikolaj / Warsaw メンバーへ 3 点: ①応答モデルパラメータの状況(062 受領待ち)
  ②ゲイン較正ツールの所在(061)③TPCReco 再配布許諾(020)。
  ついでに ④D_L/D_T の出典(Magboltz 設定ファイルの場所)を聞ければ 3 が早い。

## D. 任意(ビーム停止窓 + シフトリーダー了解が絶対条件)

- [ ] 10. パルサー振幅スキャン 1 セット(数点 × 10 分)→ ゲイン直線性。
  既存 xcfg の Generator 振幅だけを変えた別名ファイルで(072 ガイドの流儀)。

## E. 開発用ベンチ: ラップトップ直結(2026-08-24 追記)

**位置づけ = DAQ 開発用の仮設ベンチであり、本番構成ではない。**
本番は通常のデスクトップ + オンボード/PCIe の NIC を使う。したがって
**USB-C アダプタ由来の制約(MTU 9000 の可否、per-packet コスト、ドライバ品質)を
本番の設計判断に持ち込まない**。特に:

- **このベンチで測った転送レート・取りこぼしを 100 Hz 目標の根拠にしない。**
  USB Ethernet は per-packet オーバーヘッドが大きく、PC 側がボトルネックに見える。
  性能評価は本番デスクトップでやり直す。
- **コード側への影響はゼロ** — receiver は TCP ストリームを読むだけで MTU は透過。
  ベンチ都合の分岐をコードに入れない(KISS)。

### 構成

```
laptop --[USB-C 2.5G アダプタ]--RJ45--> [1GbE SFP-to-RJ45]--> ZC706 SFP+ = eth1
```

**ケーブル 1 本で制御もデータも通る**(= ELI-NP の 2 AsAd 手順書と同じ構成)。
eth0(オンボード RJ45・100 Mbit・192.168.1.40)は挿さないままでよい。

| 項目 | 値 | 根拠 |
|---|---|---|
| laptop IP | `192.168.4.3/24` static | `ZC706.../scripts_2asads/README_SCRIPTS.txt:12,49`。**`192.168.4.2` は本番 DAQ PC(項目 7 実測)なので名乗らない** |
| MTU | `9000` | `ZC706.../docs/README.txt:61`(FW 側の明示要求) |
| リンク速度 | `1000` 固定。2.5G は出ない | SFP-to-RJ45 が 1000BASE-T(`docs/README.txt:14`)。README も「1 Gbit/s に設定せよ」 |
| GW / DNS | なし | 直結 |
| ポート | 46001 hwServer(ボード) / 46002 eccServer / 46003 dataRouter ctrl / **46005 FDT データ** | `MDaq/src/mdaq/DefaultPortNums.h:48-53` |

**制御が SFP+ 側に載る確証**(1 本で足りる理由):
- `scripts_2asads/describe-zCobo-ZC706.xcfg` — `Node id="CoBo" Instance id="0"` の
  Endpoint が addressField[2]=4, [3]=81 で `*` を上書き → **192.168.4.81:46001**。
- `GetBench/src/GetHwServer.cpp:101,168` — Linux の `main()` は `INADDR_ANY` で bind。
  `Utilities/src/utl/net/SocketAddress.cpp:130` が `isAny()` のとき `-h` を出さない
  → Ice はワイルドカード = 全インタフェース listen。`normalized()` の書き換えは
  `#ifdef VXWORKS` の中だけなので Zynq Linux では素通り。
- `192.168.1.40` が出るのは古い 1-AsAd の `configs_1asad_physics_20180605/zCobo.init:1` のみ。

### 帯域見積り(1G で余裕)

compact rev5 は `itemSize=2`(`scripts_2asads/CoboFormats-Rev-5-Compact.xcfg:15`)。
2 AsAd @100 Hz = 68×512×2×2 ≒ 139 kB/event → **14 MB/s ≒ 112 Mbps**。
frameType 1(4 B/item)でも ≒223 Mbps。Ice 制御が同居しても問題なし。
→ **ジャンボの目的は帯域ではなく PC 側の per-packet コスト削減。**

### 現地チェック(接続順)

- [ ] 11. **ボード eth1 の実 IP を先に確定** — `.81` は FW 工場デフォルト。
  **本番で走っている zCoBo は `192.168.4.84`(項目 7 実測)**。持ち出し機/予備機が
  どちらかは現物で確認する。不明なら → 項目 15 の復旧経路。
- [ ] 12. リンク速度が `1000baseT` になっているか(`ifconfig en<N> | grep media`)。
  SFP モジュールによってはオートネゴが成立せず link down することがある。
- [ ] 13. **`sm-start` の前に DF ping を通す**(下記)。ジャンボ破綻の症状が
  極めて紛らわしいため、これは省略しない。
- [ ] 14. `DataLinkSet` の `DataRouter ipAddress` をラップトップの実 IP に合わせる。
  **リンクを張るのは zCoBo 側**(項目 7: エフェメラル → PC:46005)なので、
  ここが違うと接続そのものが来ない。

```bash
sudo ifconfig en<N> 192.168.4.3 netmask 255.255.255.0 mtu 9000
ping -D -s 8972 192.168.4.<81|84>          # macOS。Linux は ping -M do -s 8972
                                            # 8972 = 9000 - 20(IP) - 8(ICMP)
ifconfig en<N> | grep media                 # 1000baseT か
ssh root@192.168.4.<81|84> 'ip link show eth1'   # ボード側も mtu 9000 か
```

### 落とし穴

- [ ] 15. **ジャンボ破綻は制御が通るので気づけない。** Ice は小パケットなので
  経路が 9000 を通さなくても `sm-status` / `sm-configure` は正常に通り、
  `sm-start` した瞬間にデータだけ来ない・固まる。→ 必ず項目 13 を先に。
  疑わしければ MTU 1500 に落として動くか比較する(切り分けとして確実)。
- [ ] 16. **USB ポート 1 個 = シリアルコンソールと排他。**
  `README_SCRIPTS.txt:33` の `screen /dev/ttyUSB0 115200`(J21=UART)は
  IP を見失ったときの最終復旧手段だが、Ethernet アダプタと同時に挿せない。
  中間の復旧経路として **ZC706 オンボード RJ45(eth0 = 192.168.1.40, 100M)に
  挿し替えて ssh → `ip addr show eth1`** が使える(こちらは USB アダプタを流用可)。
- [ ] 17. **単一障害点。** アダプタが 1 個なら故障で作業停止。
  可能なら**別チップの予備を 1 個**(RTL8156 系 / ASIX AX88179 系はドライバ都合の
  MTU 9000 可否が異なり、片方が蹴っても他方が通ることがある)。
  USB-C ポートが 1 個なら PD パススルー付きでないと作業中に充電できない。
- [ ] 18. `192.168.4.2` を名乗らない(本番 DAQ PC と衝突)。ベンチを本番ネットワークに
  戻す可能性がある限り、IP は借り物という前提で扱う。

## 帰還後の接続先

1 → reference 照合ユニット(新規起票)/ 2 → 066 ハーネス再走 / 3–4 → C-6・SURVEY 追記 /
5 → Warsaw 互換確認(CURRENT 保留)/ 7–8 → 069 の該当項目を前倒しクローズ。
11–18(E)は成果物ではなく開発ベンチの手順 — 帰還後は本番デスクトップ + 通常 NIC で再構築し、
ベンチ固有の設定は破棄する。
