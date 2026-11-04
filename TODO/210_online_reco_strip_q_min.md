# 210 — オンラインの再構成(`src/reco`)の strip 電荷の切り `STRIP_Q_MIN = 2000` を peaking ごとに

**Status: TODO(Fable キュー: SPEC §3.1・§5.5 の改訂)**(2026-11-04 起票 Opus。207 のオンライン側を切り出し)

## 何が起きるか
- `src/reco/mod.rs:39` の `pub const STRIP_Q_MIN: f64 = 2000.0`(SPEC §5.5 = macros の analyzeUVW / makeTracks の移植、定数は macros と同じに固定。テスト `src/reco/tests.rs:251` が値を固定)。
- 2026-11-04 からの運転は peaking 232 ns(ユーザー「今後は 232 ns で行こうと思う」)。macros では 207 で 232 ns → 1,100(strip の電荷の分布の谷 950–1,150)にした。オンラインは 2,000 のままなので、**UI の Event Display / QA 再構成の線がトラックの淡い側で短く出る**(表示だけ — graw・ROOT には影響しない)。

## 直し方の案(決めるのは Fable)
- (a) `[monitor] reco_strip_q_min`(既定 2,000、ELITPC は 1,100)を TOML に足して `src/reco` に渡す。KISS。peaking を変えたら運転員が TOML も変える(RUNBOOK に 1 行)。
- (b) params(`current.json`)の `peakingTime_ns` から表で引く(monitor は今 params を読まない — 読み口が増える)。
- 推し(Opus)= (a)。テスト: macros の `test_analyzeUVW_qmin.C` と同じ合成で、Rust の reco が 2,000 / 1,100 で macros と同じヒットを返す(oracle は 207 の macros)。

## 優先度
11/9 の前には要らない(表示だけ)。
