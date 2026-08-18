# 065 — reference/config(現行実験 ECC 設定の生コピー)の全数調査

**Status: COMPLETED**(2026-08-18、調査ユニット — コード変更なし)

## 目的

ユーザーが現行実験(HIgS_2026)で使用中の config ファイル群を `reference/config/` にコピーした
(2026-08-18 配置、.gitignore 済み)。全ファイルを分類し、既存一次資料
`reference/ZC706_20181031_ELINP/` と照合して「真に新しい情報」を特定、SPEC の前提
(frameType 2 / フル readout / 生 ADC / `CoBo[0]` 命名)と突き合わせる。

## 対象

- `reference/config/` 全 184 ファイル(約 6.5 MB)。
  内訳: ルート(workspace / configure-* / describe-* / hardwareDescription / CoboFormats /
  .ecc スクリプト群)+ `before_HIGS_Apr2022/`(101)+ `before_HIGS_Aug2026/`(28)+
  `OBSOLETE_xcfg_zcobo_1AsAd/` + `OBSOLETE_xcfg_zcobo_4AsAd/` + 空の `pedestals/` `pulser/`。

## 結果

### 実行方法(2026-08-18、macOS ローカル、コマンドは全て `reference/config/` 起点)

- 全数リスト: `find . -type f | wc -l` → **184**、`du -sh` → 6.5M
- ZC706 照合: ルートの全 .xcfg/.ecc/.init について
  `find ../ZC706_20181031_ELINP -name <basename>` + `cmp -s` で同名探索 → バイト比較
- 系譜確認: 現用 physics xcfg を Apr2022 / Aug2026 スナップショット・OBSOLETE 各版と `diff`
- workspace 3 世代(`workspace.xcfg` / `.orig` / `~`)相互 `diff`・`cmp`
- git 保全: `git check-ignore -v reference/config/workspace.xcfg` →
  `.gitignore:18:reference/` で除外効果を実測確認(working tree クリーン)

### 判明した事実(要旨)

1. **出自**: 実験 ECC マシンの `/data/edaq/GetSoftware_config` 作業ディレクトリの生コピー。
   `workspace.xcfg` は 2026-08-17 時点 = HIgS_2026 実験の最中のライブ状態。
2. **ZC706 照合の結論**: hardwareDescription(zCobo / fullCoBoStandAlone)、CoboFormats
   全リビジョン(Rev-0〜5 / 5-Compact)、MergedDataFormats、describe 4 種、.ecc スクリプト
   全 20 本は **ZC706_20181031_ELINP とバイト同一**。FW 側レジスタ意味論・フレーム形式定義に
   ドリフトなし。**真に新規**なのは以下のみ:
   - `workspace.xcfg`(ライブ)+ `.orig`/`~`(= before_HIGS_Aug2026 スナップショットと同一)
   - `configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg`(2026-08-03、現用)
   - 同 `12.5MHz_232ns_trigDelay3576.xcfg`(2026-08-04、予備)
   - `configure-pedestals-zCobo1k_120fC_{12.5,25}MHz_232ns.xcfg`(2021-11-24)
   - `configure-pulser.xcfg`(2020 改変版 — 下記 5)
3. **現用ラン設定**(workspace の Test `Physics_extTrg_zCobo`):
   `hardwareDescription_zCobo.xcfg` + 上記 trigDelay1748 xcfg、TARGET 192.168.4.84 の
   **zCoBo 1 台 × 4 AsAd 全 active**(zCobo1k = 1024 ch)、`coboId=0`・`dataSource=0`。
   外部トリガ、CKW 25 MHz、gain 120 fC、peaking 232 ns、triggerDelay 1748×10 ns = 17.48 µs
   (512 buckets @25 MHz = 20.48 µs 窓内)、`readoutDepth=512`、
   **`enableZeroSuppression=false` + `isAllChannelRead=true` + `isFPNRead=true`**
   → 物理ランは固定長・全 68 ch/AGET・FPN 込み・生 ADC。**我々のデコーダ既定と完全一致**。
   AsAd 毎循環バッファ 32 MB × 4(0x8000000–0x10000000)。
   pedestals は periodically 100 ms(10 Hz)+ GlobalThreshold 7 + hit register off の同型。
4. **設定の系譜が安定**: physics xcfg は 2022-04(前回 HIGS)から現在まで
   **差分は `triggerDelay`(とクロック周波数)ただ 1 行**。ラン毎に触るノブは実質トリガ遅延のみ
   → run 制御 UI は「設定ファイルの選択」で十分、設定エディタ不要(現行 SPEC 方針の実証)。
5. **pulser 設定だけ部分 readout に改変**(`isAllChannelRead=false` / `isFPNRead=false` /
   `enableWriteHittedregister=true`): 実機パルサーランは**可変長・ヒット ch のみ・FPN 無し**の
   フレームを吐く。物理ランには影響なし。パルサー .graw をリプレイする際は
   デコーダの「68 ch 固定」非仮定を確認するテスト観点(Apr2022 に READ_IF_HIT_ONLY /
   PARTIAL_READOUT / ZERO_600 掃引群あり = 実運用検討の履歴)。
6. **describe-elitpc.xcfg(2018、ZC706 と同一)は CoBo 2 インスタンス**
   (192.168.10.40 / 192.168.3.40)+ `sm-init-elitpc.ecc` は `CoBo[0]`・`CoBo[1]` を別々の
   DataRouter(NarvalActor @192.168.10.1 / NarvalActor1 @192.168.3.1、port 46005、FDT)へ。
   SPEC v1.7「2 枚の zCoBo を 1 論理 CoBo として扱う」との関係は**時代の違い**と読むのが自然:
   `zCobo1k`(1 台 4 AsAd)の初出は 2021-11 で、OBSOLETE_1AsAd(2018–2021、256 ch)→
   zCobo1k 移行後は**物理的に 1 台**。**裁定(Fable 2026-08-18): 現行実装方針に変更不要**。
   防御として「describe に CoBo が複数現れても壊れない = 既存の複数 CoBo 前提」を維持。
   Warsaw 本体が今も 2 台構成かは P5 現地確認項目に追加(CURRENT.md 反映済み)。
7. **ネットワーク定数の再確認**: ノード制御 46001 / ECC 46002 / DAQ 46003、DataRouter
   46005(FDT)、`DataSender id="CoBo[0]"` — 既知事実と全一致、反例なし。
8. こぼれ: `vivado.jou/.log` は 2017-12 に **ZCU102**(ZC706 ではない)のブロックデザインを
   開いて閉じただけの GUI ログ(ビルドなし、実務上無関係)。`nbEvent=100` は GetController の
   テストラン用 stop-after-N。`pedestals/` `pulser/` は空(データ出力先の残骸)。

### テスト

- 本ユニットは調査のみでコード・SPEC 本文の変更なし → cargo/C++ テストは**対象外(スキップ)**。
  リポのゲートは直前の 064 時点(cargo 454 passed / clippy クリーン)から無変更。

### 記録先

- メモリ: `reference-config-live-ecc.md`(現用 = trigDelay1748@25MHz、physics xcfg は
  2022 年以来 delay 以外不変、ほか要点)を新規作成 + MEMORY.md 索引更新(2026-08-18)。
- CURRENT.md: P5 Warsaw 残確認に「zCoBo 台数構成(2018 describe = 2 台 vs 現行 1 台 4 AsAd)」
  を追記。
