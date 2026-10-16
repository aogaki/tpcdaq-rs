# 152 — monitor: `[system] ped_table` で 0x02 グリッドの ZS 復元(ライブ再構成と UI の基線を直す)

**Status: COMPLETED**(2026-10-16 起票 Fable。Opus)

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
- 2026-10-16: Opus `a46c8be`。`MonitorParams.ped_table`(起動時に `calib::read_json::<PedestalTable>`、path / generated_at / ch 数をログ、読めない・空は起動エラー)、`DisplayConverter::with_pedestal_table`((cobo, asad) ごとに `round_mean` と `base[plane][strip]` を前計算)、2 パスの述語(151 と同形、窓 1..20、半数ちょうどは非 ZS)+ 復元(base + (adc − round_mean)、i32、clamp)、カウンタ `uvwZsFragmentsRestored / uvwZsFragmentsNoTable / uvwTableMissingChannels`(status JSON、停止ログ)、0x03 と Reco は不変。テスト 13 本(赤 9 + e2e 2 を実測、不変テストは変異で確認): (a) 復元オラクル + セクション合算、恒等、述語の境界、(b) full + 表 = 不変、(c) 表なし = 不変 + 計数、(d) `Reco` が復元グリッドで full と完全一致(表なし ZS では不一致を `assert_ne`)。`cargo test` 1104 / 0、clippy 0。`Ingest::new` / `spawn_sub_thread` が `DisplayConverter` を受ける形に(前計算を起動時に)。
- 注意(implementer、156 で確認): (d) の完全一致は ZS 閾値 ≤ 35 ADC かつノイズ無しの条件。実機の閾値(ped + 5σ ≈ +40〜+100)では (35, 閾値) のセルが失われ q が少しずれる。monitor の `uvw_zs_*` は run_stop には載らない(157 で経路)。

**Status: COMPLETED**(2026-10-16、Fable 照合)
