# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-14(P2/P3 前半波の詳細は
[archive/CURRENT_2026-08-14_p2_p3wave.md](archive/CURRENT_2026-08-14_p2_p3wave.md) へ
スナップショット退避 — 本ファイルは一新)**

## いま(1 分で読める要約)

- **P0/P1/P2 完了**(出口はすべて実データオラクルで実測クローズ)。
- **P4 の核も前倒し完了**: 015 logbook / 016 controller / 017 ecc-bridge。
- **P3 は UI を残して完了**: 018 C++ geometry → 020 PEventTPC → 022 ヒスト+PUB →
  026 monitor+WS。**モニタ経路はワイヤまで開通**(リプレイ → root-sink PUB → monitor →
  WS。実データで WS 全ビンが独立再計算と一致)。
- **P2 批判的レビュー完遂**([archive/P2_REVIEW.md](archive/P2_REVIEW.md)): 所見 14 件 →
  実装処置は 023/024/025 で全クローズ。残は R-P2-11(負荷ハーネス)のみ。
- **リポ全体ゲート: 377 passed / 0 failed**(実データ + ecc 実配線込み)+
  C++ 側 make test / test-root 全 green。clippy -D warnings クリーン。
- 実装の正本 = **docs/SPEC_ja.md v1.10**。モデル使い分け・完了時ルール = CLAUDE.md。
- 公開リポ: https://github.com/aogaki/tpcdaq-rs(実データ・FW・実 .dat は
  reference/ = .gitignore)。

## アクティブ

- [027_web_ui.md](027_web_ui.md) — **起票済み・発注可**(Angular + Material + ECharts +
  JSROOT 遅延ロード。TS 側 WS デコーダ + §10.4 適合。Run 制御ボタンは disabled
  レイアウトのみ・モック禁止 = ユーザー決定 2026-08-13。026 の申し送り
  (/ws 固定・casing・staleness)は archive/026 結果節)。

## 次(順次起票)

1. **P3 E2E**(§12-8〜10 + R10。跨 run スループット計測 = v1.10。異常系 1 本:
   eos-timeout 後の畳みで root-sink が正しく fatal する経路 — 016 レビュー注記)
2. **負荷ハーネス起票**(R-P2-11 = §12-5 24h / §12-6 10 分。**Warsaw 前必須**)
3. P5(実機展開)は docs/WARSAW_PLAN_ja.md

## 保留・確認事項

- **Warsaw 確認**: データリンク本数(zCoBo 2 枚 → TCP 1 本か 2 本か、SPEC §13-7)/
  **TPCReco 再配布許諾**(020 — third_party/tpcreco 昇格の条件)/ PROPOSAL v0.5 反映判断。
- **物理屋向け資料・デモは UI + ファイルデータソース完成まで待つ**(ユーザー決定)。
- 小粒フォローアップ(次に該当ファイルを触るユニットへ相乗り): geometry.rs の
  参照アクセサ(Aux ch の per-sample String 確保解消 — 026 申し送り)/
  poisoned 時 metrics を `PoisonError::into_inner` で読む(023 申し送り)/
  022 の残 = TPCReco 許諾のみ。
- **delila-rs への申し送り**: pop_for 競合 → **issue 化済み**
  https://github.com/ELI-NP/delila-rs/issues/26(メモリにも記録)。
  ZMQ fair-queue 飢餓(013)も delila-rs 要点検(メモリ記録済み)。

## 運用メモ(常時適用はメモリ・CLAUDE.md 側が正)

- C++ の make は必ず `-j`(ユーザー指示 2026-08-14。Makefile 並列安全確認済み)。
- worktree 並列発注時は開始時に `git merge --ff-only main` を発注書で指示(分岐点が
  古いことがある)。取り込みは所有ファイル限定 diff + 本体ゲート再実行。
- どんな小修正でも連番チケット + 結果節 + archive(ユーザー方針「なんだって記録に残したい」)。

## 完了ユニット台帳

000〜026 すべて [archive/](archive/) に結果節つきで格納(単位の詳細・テスト実測値・
逸脱の裁定はすべて各 md の「結果」節が正)。直近: 2026-08-14 に
016/022/023/024/025/026 + P2 レビュー + SPEC v1.9→v1.10(コミット d2f3abf..c3378df)。
