# 072 — GET xcfg 設定・チューニングガイド(docs)

**Status: COMPLETED**(起票・実施 Fable 2026-08-18。ユーザー依頼「設定ファイルの設定の仕方、
どういうものを見ながらチューンアップしていくかをまとめたい」)

## 目的

065(実 ECC config 全数調査)・071(ログブック 5 年分の実運用)・066/067/068(我々の実測)が
揃った時点で、xcfg の構造と「実際に何を・何を見ながら調整するか」を実務ガイドとして文書化する。

## 結果

- **成果物**: [docs/XCFG_TUNING_GUIDE_ja.md](../../docs/XCFG_TUNING_GUIDE_ja.md) v1.0。
  構成: ①3 層構造(workspace / hardwareDescription / Conditions + DataLinkSet)と
  ECC 遷移の対応 ②ワイルドカード継承(id="*" 既定 + 個別上書き = 差分ファイル運用の基盤)
  ③実際に触るノブの頻度順整理(triggerDelay が唯一の常用ノブ / クロック・ゲイン・シェーパ /
  ランの種類と readout モード ↔ frameType の対応 / 較正系 / 触らない層)
  ④チューニング実務ループ(1 ノブずつ + ファイル名に条件焼き込み → pedestal → pulser →
  physics、各段の基準値 = 068、確認手段 = decoder カウンタ・StripTime)
  ⑤既知の罠(internal DataRouter クラッシュ、部分 readout の frameType 1 化、
  dataSource ワイヤ値 1、三点セット E·f·delay)⑥参照一覧。
- 内容はすべて既存の実測済みチケット(065/066/067/068/070/071)と一次資料の統合であり、
  新規の推測は含まない。
- **テスト**: 対象外(ドキュメントのみ、コード・SPEC 無変更。リポゲートは 809eb29 時点の
  cargo 468 passed から無変更)。実行環境: macOS ローカル、2026-08-18。
