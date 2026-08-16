# 062 — 応答パラメータとパルサー較正の回答記録

**Status: COMPLETED**(2026-08-16 — 記録ユニット)

## 回答(ユーザー 2026-08-16、Mikolaj/Wojciech 面談より)

1. **StripResponseCalculator の応答パラメータ**: Mikolaj が**ストリップとガスの特性から
   構築済み**。実測値合意は不要 — 残タスクは値/応答ファイル(save/load 機構あり)の
   受領のみ。Unfolding は既存応答の逆問題として設計できることが確定。
2. **ゲイン較正のパルサー**: Warsaw の外部注入ツールは**無いようなので内製で行く**
   (ユーザー裁定)。FW 内蔵 AsAd generator の較正ランプ(configure-pulser.xcfg、
   amplitudeStart/Stop/Step)を使った較正 run を我々の run 制御で実施する方針。

## 反映

WARSAW_PLAN §7 の 2 行を解消/内製方針に更新。
両教授から好感触 → **英語ミニプロポーザル(UI スクリーンショット入り)作成へ**。
その前提として UI 表示文字列の英語化 = TODO/063。
