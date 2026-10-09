# 104 — LED と記録が 1:1 でない件: 約 1 秒の固定不感時間 / multiplicity 選別は無し(参照設定)/ 10-07 の半減は線源側

**Status: COMPLETED**(2026-10-08 起票・解析 Opus。ユーザー「LED と 1:1 でなくなっている。Vth を下げる前から。別のノイズ源が busy にしているか、AsAd の設定で multiplicity 等のフィルタリングをしていないか」)

## 1. イベント番号は全 run で連番(`evGaps.C`)
09-25 / 09-29 / 09-30 / 10-01 / 10-02 / 10-07 の 6 run とも eventId は 0 から欠番なし。
= **CoBo が受け付けたトリガは全部保存されている**。LED と記録の差は「CoBo が受け付けなかったトリガ」= CoBo の手前、または CoBo の busy 中。

## 2. 事象間隔に約 1 秒の不感時間(`deadTime.C`、`hazard.C`、図 `reference/_spike/stop_0929/deadTime.png`)
- 全 run で 0.84〜0.98 秒未満の間隔がゼロ(0.36 Hz のポアソンなら 26 % あるはず)。
- ハザード(直前の事象から t 秒後の受付率):
  - 低レートの 09-29 / 10-01 / 10-02 / 10-07: **1.0 秒まで 0、以後は一定**(0.24 / 0.08 / 0.06 / 0.035 /s)= 固定・非麻痺型の不感時間 τ ≈ 1.0 s + ポアソン。隠れた高レートのトリガ(雑音で延び続ける麻痺型)の兆候は無い。
  - 高レートの 09-25 / 09-30: 1 秒後 0.4 /s から 5〜10 秒かけて 0.75〜0.9 /s へ回復 = 待ち行列(下流の吐き出しが追いつかない)型。
- ライブタイム推定: 09-25 ≈ 50 %、09-30 ≈ 45 %、09-29 ≈ 80 %、10-01〜10-07 ≥ 93 %。**LED と 1:1 でないのはこれで説明でき、Vth と無関係に以前から起きていたことと整合**。
- 1 フレーム 278,784 B を約 1 秒 = 0.3 MB/s 相当。GET の VM 側の保存経路(dataRouter / TestController / ディスク)が律速と推定。tpcdaq-rs の receiver はソケットを常時吸い出すので(never-stop)、puppex 移行で解消するはず。確認法: AsAd パルサーを 2 Hz で入れて記録が 1 Hz に頭打ちになるか。
- ただし不感時間で説明できるのは最大でも約 2 倍。線源強度からの 1/100(097 後の議論)は説明できない。

## 3. multiplicity での選別(参照設定で確認、VM の実設定は未確認)
- `reference/config` の旧 1 AsAd 物理設定と HIgS 現用 4 AsAd 設定とも、CoBo の `triggerMode = external`、`secondaryTriggerMode = noTrigger`、`frameType = standard`、`readoutDepth = 512`、`enableZeroSuppression = false`。
- L0/L1/L2 の multiplicity 条件(L1A/L1B の閾値など)は **`<Node id="Mutant">`** の節 = MuTanT トリガボード用で、zCoBo 単体構成では使われない。CoBo の `multiplicityThreshold`(50)は `onMultiplicity` モード時のみ有効。
- → 参照設定どおりなら multiplicity 選別は無い。**ミニの実機設定(VM 内 `configure-physics-extTrig_240fC_25MHz_70ns.xcfg`)は手元に無い**ので、ユーザーに `grep -n -E "triggerMode|multiplicityThreshold|enableZeroSuppression|readoutDepth" <file>` の確認を依頼。

## 4. 10-07 の半減は線源側(`srcVsOther.C`)
| run | 線源方向(左壁から、\|φ\| ≤ 15°) | それ以外(電荷 ≥ 3.2e5) | それ以外(全部) |
|---|---|---|---|
| 10-01(93 mbar) | 243.0 /h | 16.2 /h | 16.9 /h |
| 10-02(91 mbar、Vth 500) | 185.6 /h | 13.8 /h | 14.2 /h |
| **10-07(Vth 300)** | **92.4 /h** | **13.7 /h** | 19.6 /h(新しい低電荷分 +5.9) |
- 線源方向だけが半減し、他方向の α(同じ電荷カット)は不変 = トリガ・DAQ の効率は変わっていない。線の停止位置も同じ = 距離も同じ。**線源から見たケージへの流量が半分**(ホルダの向き・横ずれ・遮蔽物など)。
