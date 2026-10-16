# 152 — monitor: `[system] ped_table` で 0x02 グリッドの ZS 復元(ライブ再構成と UI の基線を直す)

**Status: READY**(2026-10-16 起票 Fable。151 の `src/config.rs` 変更の後に着手。Opus)

## 0. 問題
`src/reco/mod.rs:232-290` の ped = cells 1..20 の平均(FPN なし)、UI の Event Display の基線 = 0x02 の cells 0..24 平均(SPEC §5.4 v1.18、`ui/src/app/monitor/root-histo.ts:309`)。ZS データでは欠けたセルが 0 なので ped ≈ 0 → 全ストリップがヒット、基線表示も 0。

## 1. 仕様
1. `MonitorParams.ped_table: Option<PathBuf>`(`config.system.ped_table`、151 と同じキー)。起動時に `calib::read_json::<PedestalTable>`(`src/calib/pedestal.rs`)で読み、path / `meta.generated_at` / ch 数をログ。
2. `DisplayConverter::with_pedestal_table(...)`(`src/monitor/display.rs`): (cobo, asad) ごとに `base[plane][strip] = Σ round(mean)`(その AsAd のストリップ ch 分、`geometry.lookup_ref` で引く)と `round_mean[aget][raw_ch]` を前計算。
3. `accumulate_uvw` でフラグメントごとに **ZS らしい述語**(151 と同じ定義: サンプルのある ch の過半が ped 窓 `PED_LO..=PED_HI` = 1..20 内のサンプル数 < 20 — 2 パス: 数えてから積む)。ZS らしく表があれば、その AsAd のストリップの全バケットに `base` を足し、存在するサンプルは `(adc − round_mean)` を足す(i32、u16 にクランプ)。= 欠けたセルを round(mean) で埋めたのと同じ(セクション合算にも整合、全データなら恒等)。
4. 非 ZS フラグメント / 表なし: 従来どおり。表なしで ZS らしい → `zs_fragments_no_table` + WARN 1 回。カウンタ `uvw_zs_fragments_restored / uvw_zs_fragments_no_table / uvw_table_missing_channels` を `ConvertCounts` → 停止ログ + status JSON(`src/monitor/json.rs`)。
5. `Reco`、`MM_PER_CELL`、定数、0x03(波形、生のまま 0)は不変。

## 2. テスト
- `display.rs`: (a) ZS フラグメント + 表 → グリッド = 復元オラクル(無いセル = round(mean)、あるセルは不変)、複数セクションのストリップ(片方だけ在る → `adc1 + round(mean2)`)。(b) full + 表 → 表なしと同一(整数の丸めで厳密)。(c) ZS、表なし → 不変 + カウンタ。(d) `Reco::run` が復元グリッドで非 ZS と同じ `strip_hits`(`src/reco/tests.rs` の合成飛跡に ZS カットを掛けて比較)。
- `tests/monitor_e2e.rs`: 設定キーが配線されること。`tests/reco_macro_oracle.rs` 不変。`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。

## 3. 報告に必須
151 と同じ項目 + SPEC §5.4 / §10.2 / §9 の文案。

## 結果
