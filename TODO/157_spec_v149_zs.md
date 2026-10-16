# 157 — SPEC v1.49(ZS 対応)+ WARSAW_PLAN §7

**Status: READY**(2026-10-16 起票 Fable。156 の数字を見てから Fable が 1 回で確定)

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
