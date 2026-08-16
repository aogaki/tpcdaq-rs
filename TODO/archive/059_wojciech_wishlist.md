# 059 — Wojciech 要望 3 点の成立性調査と WARSAW_PLAN への反映

**Status: COMPLETED**(2026-08-16 — 調査 + 文書化ユニット。実装なし)

## 背景

Warsaw 大学 Wojciech 教授からの「あったらいいな」リクエスト 3 点(2026-08-16 ユーザー経由):
①ゼロサプレッション ②Unfolding to the raw signal for each strip ③Gain normalization
for each strip。WARSAW_PLAN に入れる前に成立性を評価せよ、がユーザー指示。

## ユーザー裁定(2026-08-16 — 本チケットの正)

- **生データは graw のみ**。ROOT は各種処理込みでよい(解析をすぐ始めるための
  プロダクト。生 graw があれば原理的に作り直せることが保険)。
- **優先順位: Unfolding > ゲイン正規化 > ZS**。今後のスピードアップ(055)が効けば
  ZS は不要になる。Unfolding で信号が峻険になれば解析アルゴリズムがトラックを
  追いやすくなり時間短縮になる — これが ROOT を並列で作る動機の延長。

## 結果(調査 — 一次資料 file:line つき)

### GET のゼロサプレッション(実装あり・ELI-NP FW 対応済み)

- 設定 3 点: `Module.enableZeroSuppression` / `channel[*].zeroSuppressionThreshold`
  (0–4095、チャンネル毎)/ `Module.zeroSuppressionInverted`
  (GetBench/doc/config_parameters.dox:132-136, 445-446, 614)。
- ソフト経路: `CoBoNode::setCoBoZeroSuppressionMode`(CoBoNode.cpp:3083)が
  pipeCtrl の `zeroSuppressionEnable`/`zeroSuppressInvert` を書く。閾値は AsAd 毎
  `zeroSuppress` デバイスへ。
- zCoBo FW(ZC706): hardwareDescription_zCobo.xcfg に pipeCtrl bit10/bit29 +
  `zeroSuppress` デバイス(0x60000000)定義済み。**現行 physics 設定は
  `enableZeroSuppression=false`、閾値 600 が種入れ済み**。
- 帰結: ZS 有効時は **frameType 1 系**(chan/bucket 明示)になる(compact rev5 =
  ChangeLog #45「channel/bucket index なし」はフル読み出し専用)。我々は対応済みだが
  実データ照合ゼロ。FPN は閾値 0 運用が必要(ペデスタル入力の保護)。

### TPCReco の Unfolding — **存在しない。ただし順方向応答は実装済み**

- 逆畳み込み(unfold/deconvol)のコードは無い(唯一のヒットは
  recoEnergyScaleFitter.cpp:299 等の HPGe/HORST の話 = 無関係)。
- **`Reconstruction/StripResponseCalculator`** が順方向応答を実装:
  拡散 σ_xy/σ_z + **AGET peaking time** + 隣接 ±strip/±timecell/±pad 応答、
  事前計算応答ヒストの save/load(StripResponseCalculator.h:37-44、.cpp:819
  `initializeTimeResponse`)。利用者は MC digitizer
  (MonteCarlo/Modules/TPCDigitizerSRC)のみ。
- **含意: Unfolding は「この応答の逆問題」として定式化すべき** — カーネルの
  パラメタ化が既存 MC と共有され、自己整合する。ゼロから応答測定は不要
  (較正での検証は別途価値あり)。

### TPCReco のストリップ毎ゲイン較正 — **存在しない(greenfield)**

- "gain" の全ヒットは無関係(ファイルサイズ・統計のコメント等)。
  makeCalibrationPlots はトラックレベル解析プロット。チャンネル/ストリップ毎の
  ゲイン表・正規化機構は無い。
- 一方 DAQ 側の道具は揃っている: ZC706 に configure-pulser.xcfg、AsAd generator の
  較正ランプ(amplitudeStart/Stop/Step — configure xcfg Generator 節)。
  較正 run は我々の run 制御でそのまま取れる。

### ZS × Unfolding のトレードオフ(WARSAW_PLAN に明記)

ZS は閾値以下(ベースライン・パルス裾)を不可逆に捨てるが、逆畳み込みは
まさにそこを必要とする。強い ZS と高品質 Unfolding は両立しない。
ユーザー裁定により **Unfolding 優先・ZS は 055 スピードアップの成否を見て判断**
(効けば不要)なので、この緊張は当面顕在化しない。

## 反映

WARSAW_PLAN_ja.md に §7(Wojciech 要望 3 点)を新設。§6 残確認に関連項目を追加。
055 に優先順位裁定を追記。
