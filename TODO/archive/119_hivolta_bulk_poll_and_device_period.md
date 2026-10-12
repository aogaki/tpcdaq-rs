# 119 — HiVolta の読み出しを全 ch 一括に(34 → 6 往復)+ 機器ごとの読み出し周期 `poll_s`

**Status: COMPLETED**(2026-10-12 起票・同日完了。118 の発見への緩和策。ユーザー「読み出しができないと記録ができないので大問題」。Sonnet 実装 2 回、Opus レビュー)

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

## 実機の結果と追補(2026-10-12 11:33–11:41)
- 一括読み出し + `poll_s = 30` を ELITPC に入れた(main `bbbfae2`、cargo 900)が、**レートは ≈ 42 Hz のまま連続**だった(30 s ごとの塊にならない)。
  → 「1 往復ごとに偽トリガ」という 118 の見立ては不完全だった。
- **ポートを開いているだけで出る**(`~/hvopen.py`。開くだけでコマンドは何も送らない、30 s 計測、LV のみ読み出し中):

  | 状態 | レート |
  |---|---|
  | HV ポート閉 | 6.40 Hz |
  | 開・DTR / RTS 既定(高) | **24.5 Hz** |
  | 閉 | 6.37 Hz |
  | 開・DTR / RTS 低 | **44.1 Hz** |
  | 閉 | 6.77 Hz |

  機序の推定: tty を開いている間、cdc_acm が bulk-IN を絶えず要求し続ける。その USB の通信活動が HV の出力か接地を経由して検出器に乗る。
  HMP2020(FTDI)の USB は開きっぱなしでも影響なし。
- **開く → 6 往復 → 閉じる**を 5 s ごとに 6 回(`~/hvcycle.py`): 1 回でポートが開いているのは 331–345 ms、30 s の事象数は 252 件(前後のベースライン 174 / 189 件)
  → **1 回あたり偽トリガ ≈ 12 発**。`poll_s = 30` なら ≈ 0.4 Hz、60 なら ≈ 0.2 Hz。

### 追補の仕様(Sonnet)
5. **HiVolta のシリアル(USB)経路は、読むときだけポートを開く**: `poll` のたびに open → 6 往復 → close(drop)。`ident` も同じく開いて閉じる。
   - 間はポートを開いたままにしない。TCP(`addr`)経路は従来どおり接続を保持する(影響は未検証なので変えない)。
   - 開くパスは by-id のまま毎回開く。開けない場合は今の失敗と同じく `DriverError` にして、component の再試行・カウンタに乗せる(silent にしない)。
   - 機器の取り違え防止は今の ident(BDSNUM 照合)を維持する。毎回の poll で BDSNUM を読み直すのは不要(往復が増える)。
6. テスト: fake のオープナを注入できる形にして、「poll 1 回 = open 1 回 + 要求 6 本 + close 1 回」「poll の間はポートを保持していない(open の数 = poll の数)」を確かめる。

## 結果(2026-10-12)
- 実装:
  - §1〜4 は Sonnet `62c6526` → merge `bbbfae2`。全 ch 一括 6 往復(`CH:8`、値の数 ≠ nch は Protocol エラー)と `[[slowctl.device]] poll_s`(> 0 かつ有限)。
  - §5・§6 は Sonnet `0e87c82` → merge `8199012`。`OnDemandHiVolta` で、シリアルは ident / poll のたびに開いて閉じる。TCP は従来どおり接続を保持。
- テスト:
  - driver: 送信行がちょうど 6 本で、実機応答 4 行をゴールデンにしたもの。値の数 7 / 9 → Protocol。nch ≠ 8 → ident エラー。
  - オンデマンド: open 1 回・要求 6 本・close 1 回 / open 回数 = ident + poll / 開けないときは DriverError。
  - config: `poll_s` の省略 / 正 / 0・負・NaN。component: `poll_s` が全体の周期より優先されること。
  - red は 1 回目で確認、2 回目は省略(Sonnet 申告)。
- ゲート(main `8199012`、Mac): fmt OK / clippy 0(feature `slowctl-fake` 有無とも)/ **cargo test 903 passed / 0 failed / 1 ignored**(既存の `tests/root_sink_monitor_pub.rs:1031`)。
- 実機(ELITPC、run 69 実行中):
  - 一括 + 30 s だけでは 42 Hz のまま(ポートを開きっぱなしにしていたため)。
  - オンデマンド + 30 s で 90 s の平均 6.57 Hz(ベースラインと区別がつかない。見積もりは 1 回の読み出しで ≈ 12 発)。
  - **その後ユーザーが USB ケーブルを交換して、偽トリガ自体が消えた**(118)。現在は `poll_s = 5`、オンデマンド方式は保険として残す。全機器読み出しで 5.1 Hz。
