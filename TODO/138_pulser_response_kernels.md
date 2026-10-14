# 138 — 整形器の応答核(AsAd 内蔵パルサー、25 MHz、peaking 4 点 × 振幅 2 点)と「揺れ」の出どころ

**Status: IN PROGRESS**(2026-10-14 起票 Fable。ユーザー「デコンボリューションが可能かも考えたい」「ソースと検出器が自由に使える今のうちにガンガン」。139(デコンボリューション試作)の入力)

## 0. 問い
1. 137 で見えた信号 ch の「揺れ」(ピーク後 3〜7 µs、−20 ADC の凹みを含む、peaking とともに電荷と同じ比で増える)は**電子回路(整形器)由来か、検出器(イオン誘導)由来か**。
   パルサー応答に同じ形が出れば電子回路 → デコンボリューションで外せる。出なければ検出器。
2. 4 つの peaking(70 / 232 / 502 / 1014 ns)の**インパルス応答**(平均波形)と線形性(振幅 2 点)。= 139 の核。

## 1. 取得(`~/test/scan/pulser_kernel.sh`、gain スキャンの終了を待って自動開始)
- base = `base_1750.json`(現用 25 MHz / delay 1750)に `triggerMode: onSlowControl`、`pulser: {enabled, voltage_mV, period_ms 50}`、`peakingTime_ns` を上書き → PUT → run/start → `getEccClient asad-pulser-start <mV> 50` → 30 s(≈ 600 pulses)→ `asad-pulser-stop` → run/stop。
- 点: peaking {70, 232, 502, 1014} × 振幅 {−300, −100} mV = 8 run。最後に base を復元。run 番号と params id は `pulser_kernel_runs.json`。
- 091 の参照: 232 ns・−300 mV・120 fC で信号 ch の振幅 ≈ 1,074 ADC(100 MHz)。1014 ns では ×5 で飽和するので −100 mV も取る。

## 2. 解析(次 — ROOT マクロ `tools/scan/pulser_kernel.C`、起票は結果を見て)
- run ごとに、信号 ch の ped(FPN 減算後、cells 1..20)を引いた波形をピーク時刻で揃えて平均(AGET 別・全 ch)。
- 出すもの: 立ち上がり 10–90 %、FWHM、ピーク後のアンダーシュートの深さと時刻、2 µs 以降の尾(揺れ)の振幅、−300 / −100 の振幅比(線形性)、飽和 ch の数。
- 137 の α の「揺れ」(周期 ≈ 1.6 µs、−20 ADC)と形を比べる。

## 経過
- **14:00 run 91–98(delay 1750 のまま)は使えない**: `pulser_kernel.C` の結果、どの run もパルスが窓に無い(70 ns の 2 run はピーク = cell 508 の端の跳ねだけ、他は A > 100 の揺れ・ノイズを拾った偽の波形で振幅中央値 ≈ 130 ADC、`calib scan` でも −300 / −100 mV の振幅が 38 / 38 ADC = 無信号)。
  読み: onSlowControl では CoBo がパルサーを叩いてから数 µs 後にトリガが出る(= パルスはトリガより前)。external の法則 `cell = 508 − delay/4` で delay 1750 の窓は [t0 − 3 µs, t0 + 17.5 µs] なのでパルスが窓の前に落ちる。実験の `configure-pulser.xcfg`(reference/config)は **triggerDelay 750** = 窓 [t0 − 13 µs, t0 + 7.5 µs]。
- **取り直し(`pulser_kernel2.sh`、gain2 の後に自動)**: 232 ns × −300 mV × delay 750 を 20 s 試し、ピーク cell を測って `delay = 750 + 4 × (peak − 80)` でピークを cell ≈ 80 に置き直し(尾 +300 cell が窓に入る)、その delay で 8 点(各 30 s)。run 番号は `pulser_kernel2_runs.json`、ログ `pulser_kernel2.log`。
- **14:20 訂正と追加の読み(VPN 切断中、手元の run 95 の図と archive/091 から)**:
  - **FPN ch にもパルサーのパルスが入る**(091: −300 mV で FPN 中央値 1,590 ADC、信号 1,074)。`pulser_kernel.C` の FPN 共通モード減算は pulser run では**パルス自体を引いてしまう**。`calib scan`(FPN 減算なし、max − baseline)でも run 93/94 の信号 ch は 38 ADC なので「窓に無い」自体は正しいが、取り直しの解析は **FPN 減算なし**でやる(Sonnet implementer に `subtractFpn=false` + `fpn` グループを発注、14:20)。取り直しスクリプトの delay 自動選択はこの罠で「パルス無し」判定 → 参照値 750 に落ちる(安全側)。
  - run 95(502 ns、−300 mV、窓にパルス無し)の FPN 減算後の平均波形は**周期 ≈ 45 cell = 1.8 µs(≈ 550 kHz)の持続する振動**(AGET 0 / 3 のほぼ全 ch、正規化で山 0.5–0.7 が 300 cell = 12 µs 以上続く、`kernel/run0095_kernel.png`)。α の「揺れ」(137: 周期 ≈ 1.6 µs、ピーク後 3–7 µs)と同じ周期帯。**読み: 入力で励起される電子回路のリンギング**(信号 ch − FPN の差に残る = ch ごとに自分の入力で鳴る。α では FPN に入力が無いので FPN は平ら、pulser では FPN も鳴るので差に出る)。取り直しの生(減算なし)波形で、FPN グループと信号グループそれぞれに同じ周期が出れば確定。
- **17:35 原因確定(VPN 復帰後)**: run 101(delay 750、232 ns、−300 mV、20 s、FPN 減算なしで解析)も**パルス無し**(402 event、A > 100 の波形は 1,504 本 = ノイズ拾い、FPN グループ空)。`reference/config/configure-pulser.xcfg` と physics xcfg の diff で判明: pulser 用は AGET の **`TestModeSelection = functionality`** と各 ch の **`isSelectedforTestMode = true`**(+ GlobalThreshold 7、readout partial、100 MHz、1014 ns)。我々の elinp テンプレートは `nothing` / `false` → **AsAd 生成器は動いて(50 ms 周期でトリガは出る)いるがテストパルスが ch に注入されない**。run 91–98 / 101 はこれが理由(delay の話は二次)。両設定は非管理ノブなので、**pulser 用テンプレート `configure-elinp-pulser.xcfg`(elinp の 2 箇所だけ置換)を `GetSoftware_config` に置き、params の `template` で選ぶ**必要がある(= 共有ディレクトリへの追加 → ユーザー承認待ち)。`pulser_kernel3.sh`(テンプレート指定 + 減算なし解析 + delay 750 試験 → 8 点)を用意済み(scratchpad、承認後に転送)。
- run 101 は gain2 との token 競合で stop が失敗し 3.5 h 走り続けた(データは 14:06 で止まっていた、384 MB)→ 17:34 に停止、base 復元(メモリ elitpc-automation-sequencing に教訓)。
- マクロは main に取り込み済み(`df38f8d`、テスト 172 green、Opus implementer の報告は §2 の各項目とテスト名で照合)。

## 結果(取り直しの後に記入)
