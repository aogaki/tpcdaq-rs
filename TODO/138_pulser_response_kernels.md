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

## 結果(取得後に記入)
EOF
