# 157 — SPEC v1.49(ZS 対応)+ WARSAW_PLAN §7

**Status: COMPLETED**(2026-10-16 起票 Fable。156 の数字を見てから Fable が 1 回で確定 — 同日執筆)

## 改訂項目
- §3.1 / §3.2: `[system] ped_table`(任意、存在検査、root_sink `--pedestal-table`、monitor)。表 = `calib pedestal` JSON、(asad, aget, ch) キー、1 論理 CoBo。
- §6.4: 「ZS 表モード」— ZS らしいフレームの述語、correction = 表の mean、FPN 項なし、存在サンプルのみ、`pedestalSubtracted`、表なしのフォールバック + カウンタ、非 ZS 不変(§12-3 維持)、性質(正側の外れ値のみ残る / 閾値未満の裾の損失 ≈ −3 %)。
- §5.4 / §10.2: 0x02 の復元(ZS らしい + 表 → round(mean) で埋める、セクション合算は ch ごとの mean の和)、0x03 は生。§5.5 の基線 = 復元グリッドの cells 1..20。
- §6.7: raw ツリーは ZS でも 0 詰めのまま、`zsfilled` は tools/scan の産物(規則と命名)。
- §9: monitor 停止ログの `uvw_zs_*`、run_stop の `pevent_zs_*`、decoder の frameType カウンタ。
- §13: ZS で frameType 2 が来ないことの実機確認 + compact サイズ検査。
- §3.3: `zeroSuppression.threshold` の例外は `zs_exceptions.py` / `zs_params.sh` で作る。
- `docs/WARSAW_PLAN_ja.md` §7: ZS graw は grawToEventTPC の PedestalCalculatorGRAW を壊す(offset ≈ 0、FPN 0)→ 我々の PEventTPC(表で減算済み、run_stop にカウンタ)を使うか、TPCReco に pedestal 表の入力を足す。

## 結果
**2026-10-16 11:40(Mac、Fable 主対話)**: 文書のみの改訂(コードは変えていない)。

- `docs/SPEC_ja.md`: status を **v1.49(2026-10-16)**、改訂履歴に v1.49 を追加。改訂箇所:
  - §3.1 例に `ped_table`(コメント行)、§3.2 に `[system] ped_table` の規定(任意・1 本、`calib pedestal` JSON、キー (asad, aget, raw ch) = 1 論理 CoBo、parse 時に存在検査、読み手 = root-sink `--pedestal-table` と monitor、電子回路設定に結び付く)。
  - §3.3 に ZS 閾値の例外リストの作り方(`calib pedestal` → `zs_exceptions.py` → `zs_params.sh`、FPN は既定で閾値 4095 = 抑制、例外数 = ch 数)。
  - §5.4 に **0x02 の ZS 復元**(述語、round(mean) 埋め、セクションは和、表なし / 表に無い ch のカウンタ、0x03 は生)。§5.5 の基線 = 復元後グリッドの cells 1..20。
  - §6.4 に **ZS 表モード**(述語の定義、表モードの減算、表なしの WARN 1 回 + 保存継続、非 ZS 不変 = §12-3 不変、性質 −3 %、3 カウンタ名)。
  - §6.7 に「ZS でも raw ツリーは 0 詰め、復元は `zs_restore.C` の `zsfilled` 同名ファイル」。
  - §9.2 run_stop.counters に `frames_type1` / `frames_type2` / `compact_size_mismatch`(158)。
  - §10.2 0x02 行に復元の注記、§10.3 status に `uvwZsFragmentsRestored` / `uvwZsFragmentsNoTable` / `uvwTableMissingChannels`。
  - §13-9 新設: ZS は frameType 1 のみ、run 170–173 で type2 = 0 / mismatch = 0、バイト実測、ELITPC 4 AsAd の確認を Warsaw に残す。
- `docs/WARSAW_PLAN_ja.md`: §7 の ZS 行を「実装・実測済み(SPEC v1.49)」に書き換え、オフラインの罠(ZS graw は grawToEventTPC の `PedestalCalculatorGRAW` を壊す → PEventTPC の表モードを使うか TPCReco に表入力を足す)を先方への事実共有として明記。§6 の「事実上不採用」行に 2026-10-16 追記。
- `docs/RUNBOOK_ja.md`「ZS 運用」のカウンタ名を実装どおりに訂正(`zs_frames_no_table` → `pevent_zs_frames_no_table` / `uvwZsFragmentsNoTable`、decoder の 3 カウンタ、run ごとの経路は 159)。
- 実装との照合元: `tools/root_sink/pevent_fill.hpp`(述語・表モード・カウンタ名)、`src/monitor/{display,json,mod}.rs`、`src/decode.rs` / `src/decoder.rs` / `src/logbook.rs`(158)、`src/config.rs`(存在検査)、archive/156 の表。
- テスト: 文書のみのため実行なし(直前の main `9f4e27f` のゲート = cargo 1104 passed、root_sink test-root 全 passed、tools/scan 各テスト緑 — archive/156 参照)。
- 残件は **159**(close 行 / status / run_stop の run ごとの経路、PEventTPC 電荷和、UI 基線の目視)として起票。

## 実行できなかった操作
なし。
