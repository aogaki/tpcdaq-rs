# 119 — HiVolta の読み出しを全 ch 一括に(34 → 6 往復)+ 機器ごとの読み出し周期 `poll_s`

**Status: IN PROGRESS**(2026-10-12 起票 Opus。118 の発見への緩和策。ユーザー「読み出しができないと記録ができないので大問題」。Sonnet)

## 背景(118)
- HiVolta(DT1415ET)と USB シリアルで 1 往復するたびに、偽トリガが ≈ 1.4 発入る(A/B 試験で特定)。
- 今の `hivolta_driver::poll` は ch ごとに 4 往復(VMON / IMON / STATUS / VSET)× 8 ch で 32 往復、ボード(BDALARM / BDILK)で 2 往復、合わせて **34 往復/周期**。周期は 1 s(全機器共通の `poll_hz = 1.0`)。
- いまは HV を読めていない(slowctl は LV だけで動かしている)= 記録できない。

## 実機で確認済みの事実(ELITPC、2026-10-12)
- 全 ch 一括(`CH:8`、`ALL_CH`)の応答は ch 0–7 の順に `;` 区切りで 8 値。1 往復は 32 ms(9600 baud)。
  - `$CMD:MON,CH:8,PAR:VSET` → `#CMD:OK,VAL:0350.00;0250.00;0350.00;0250.00;0350.00;0250.00;0600.00;0600.00`
  - `$CMD:MON,CH:8,PAR:VMON` → `#CMD:OK,VAL:0341.06;0250.24;0349.88;0250.22;0349.92;0250.16;0595.96;0599.96`
  - `$CMD:MON,CH:8,PAR:IMON` → `#CMD:OK,VAL:+0009.593;+0008.868;+0009.084;+0009.067;+0009.042;+0008.905;+0003.753;+0005.930`
  - `$CMD:MON,CH:8,PAR:STATUS` → `#CMD:OK,VAL:00033;00001;00001;00001;00001;00001;00001;00001`
  - これらを**テストのゴールデンにそのまま使う**(実機の応答なので一字も変えない)。

## 仕様
1. **`hivolta_driver::poll` を全 ch 一括にする**: `mon_ch(ALL_CH, …)` を VMON / IMON / STATUS / VSET の 4 往復 + ボード 2 往復 = **6 往復/周期**。
   - 返す `Sample` の並びと中身は今と**同一**にする(ch ごとに node = ch、param / unit / alarm の立て方も同じ。下流の monitor / logbook / UI は無変更)。
   - 応答の値の数が `nch`(ident で読んだ BDNCH)と一致しなければ `DriverError::Protocol`(黙って切り詰めない・埋めない)。
   - `ALL_CH` は NUM_CH = 8 の定数。nch ≠ 8 の機種は扱わない(ident で nch ≠ ALL_CH なら Protocol エラーにして可視化、で可)。
2. **機器ごとの読み出し周期 `poll_s`**(`[[slowctl.device]]` の任意キー、秒、f64):
   - 指定があればその機器だけ `poll_s` 周期で読む。無ければ従来どおり `1 / poll_hz`。
   - 設定検証: `poll_s` は > 0 かつ有限(0・負・NaN はエラー)。
   - 各機器は `device_loop` の独立スレッドで回っているので、`poll_interval` を機器ごとに渡すだけでよいはず。
3. ほかは変えない: 識別(ident)、ログブックの `log_every_s` / 2 % 変化の規則、status PUB、ALARM / RECOVER。
4. `config/daq_example.toml` の `[[slowctl.device]]` 例に `poll_s` の 1 行と、理由のコメント(HiVolta は通信ごとに偽トリガを作るので長めに — TODO/118)を足す。
   SPEC §3.1 の例への追記は Opus が別途行うので触らない。

## テスト(TDD)
- driver 単体(fake link): `poll` が送る要求が**ちょうど 6 本**で、文字列が `$CMD:MON,CH:8,PAR:VMON\r\n` 等であること。
  上の実機ゴールデンから作った Sample 列が、旧実装(ch 毎)と同じ値・同じ node・同じ alarm になること(ch0 は STATUS 33 = ON + UNV → alarm、ほかは 1 → alarm なし)。
- 値の数が 7 や 9 のとき Protocol エラー。
- 設定: `poll_s` 省略 = 従来周期 / 指定あり = その周期 / 0・負は検証エラー。
- component: `poll_s` を指定した機器の周期が反映されること(既存の component テストの道具で書けなければ、`poll_interval` を決める関数の単体で可)。

## ゲート
`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`(feature `slowctl-fake` 有無の両方で clippy が通ること — 既存 CI の流儀)。
