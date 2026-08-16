# 056 — ELITPC 構成デモ(vcobo eventIdx マージ + 4 AsAd プロファイル)

**Status: COMPLETED**(2026-08-16 — 結果は末尾。起票同日。
データ = `reference/exp_data/2026/` の実 4 本組)
**仕様**: SPEC §6.3(イベントビルダ 4 AsAd マージ)/ §4(vcobo は 041 の設計、
docs/VIRTUAL_ZCOBO_ja.md v1.2)/ CLAUDE.md 不変条件「ch 数を焼き込まない」
**発注先想定**: implementer/**Opus**(vcobo のマージは工学判断が残る。デモ整備込み)

## 背景(起票時調査で確定)

- `vcobo_link.hpp::load_graw_set` は複数 graw を**単純連結**で索引する。ELITPC の
  AsAd 毎 4 ファイルをこのまま送ると AsAd0 全部 → AsAd1 全部…の順になり、
  イベントビルダは全イベントで残り 3 フラグメントを待って timeout する = 全滅。
  **021 オラクルで graw_replay に「eventIdx マージ送出」を実装したのと同じ問題**。
- ECC 設定(`reference/_spike/run040/configs/`)は mini(AsAd0 のみ isActive)しかない。
  configure xcfg は `<AsAd id="0..3">` 構造を既に持つ(616〜650 行目)。
  vcobo は configure 時の AsAd マスクと `asad_count` の整合を検査する
  (vcobo_daq.cpp:425 `asad_mask_matches_config`)。

## やること

1. **vcobo: eventIdx マージ送出(TDD — tools/vcobo/ 所有)**
   - `load_graw_set` の後段(または内部)で、全ファイルのフレームを
     **eventIdx 昇順に安定マージ**した送出順に並べる。正本は
     `src/bin/graw_replay.rs` の複数ファイルマージの意味論(eventIdx は frameType 1/2
     ヘッダ 22..26 big-endian。**制御フレーム(データ以外の frameType)の扱いも
     graw_replay と同じに**)。同一 eventIdx 内の順序はファイル指定順で安定。
   - バイト列は無改変(書き換えるのは送出順だけ。フレーム内バイトは触らない)。
   - メモリ: 4×1 GiB を RAM に持つ現方式は維持でよい(デモ用途。KISS)。
     起動ログに合計バイト・フレーム数・マージ後の eventIdx 範囲を出す。
   - テスト(`test_vcobo_core.cpp` + 必要なら `make_graw_fixture` 拡張):
     ①複数ファイル(異 AsAd)がイベント毎にインターリーブされる
     ②単一ファイルの送出順は従来と完全一致(mini 回帰無変更)
     ③制御フレームの位置が graw_replay の流儀と一致 ④eventIdx 逆転入力でも安定。
2. **ECC 設定: elitpc プロファイル(`reference/_spike/run040/configs/` に新規追加)**
   - `describe-elitpc.xcfg` / `prepare-elitpc.xcfg` / `configure-elitpc.xcfg` を
     mini 版から派生(**AsAd 0..3 全て isActive=true**、他は無改変)。
     既存 mini 3 ファイルは**無改変**。config_id = "elitpc" で ECC が読める
     (フラットディレクトリ規約 — 041 README)。
3. **デモスクリプト: プロファイル切替(`reference/_spike/demo/` — ローカル専用)**
   - `TPCDAQ_DEMO_PROFILE=elitpc` で: geometry =
     `reference/TPCReco/TPCReco-HIGS2026_online/resources/geometry_ELITPC.dat` /
     vcobo.conf `asad_count = 4` + graw_files = `reference/exp_data/2026/` の 4 本
     (**読み取り専用 — 移動・改名・削除禁止**)/ demo.conf `config_id = "elitpc"`、
     root_sink `--expect "0:0,0:1,0:2,0:3"`(実引数形式は root_sink.cxx を確認)/
     送出レート既定 20 frames/s(= 5 events/s — root-sink の ELITPC 実測 ≈10 /s に
     対し余裕を取る。`TPCDAQ_DEMO_RATE_HZ` で上書き可)。
   - 既定(変数なし)は従来どおり mini。README.md にプロファイル節を追記。
4. **受け入れ実測(スタック一周)**: elitpc プロファイルで start_demo → run_once
   (60 s 程度)→ stop。**events_complete = 送出イベント数・incomplete = 0・
   late = 0**、UI(または ws_probe)でヒストが動くこと、graw 出力が AsAd 毎
   4 ファイルで入力と `--laps 1` 相当のバイト一致(先頭部分)、run.root が単一 run
   として閉じること。実測値を報告に記録。

## 受け入れ

- vcobo テスト全 green(既存 155+ 無改変 green 含む)+ 新規マージテスト。
  `make -C tools/vcobo -j` クリーン。Rust/リポ他部分に触らない
  (root_sink / demo.conf の生成は script 内のみ)。
- mini デモの従来挙動が無変更(プロファイル未指定の start_demo → run_once 一周 green)。
- 報告: 変更ファイル一覧 / テスト数 / 受け入れ実測(events_complete・incomplete・
  カウンタ・送出所要)/ 逸脱があれば理由。**コミットはしない**(発注側レビュー後)。

## 非スコープ

- loop 送出(eventIdx 書き換え周回)— 別チケット候補。
- reference/ の正本(20190315_patched / ZC706 / TPCReco / exp_data)への変更は絶対禁止。
  run040/configs への**新規ファイル追加**と demo/ スクリプト編集のみ可。
- UI の変更なし。

---

## 結果(2026-08-16 — implementer/Opus 実装、発注側(Fable)レビュー PASS)

### 実装

- **vcobo eventIdx マージ**: `peek_event_idx` / `merge_by_event_idx`(純関数、k-way マージ。
  制御フレームはソース位置で最優先 = graw_replay の流儀、同 eventIdx はファイル指定順で
  安定)+ `load_graw_set` が per-file 索引を集めて最後にマージ。起動ログに
  frames/bytes/eventIdx 範囲を出す。相乗り(受理): `index_graw` ポインタ版 +
  事前 reserve(1 GiB 一時コピー除去 / 4 GiB vector の doubling 回避。挙動不変)。
- **ECC 設定 elitpc**: `run040/configs/{describe,prepare,configure}-elitpc.xcfg` 新規。
  mini との差 = AsAd 1/2/3 の isActive true 化のみ(diff で機械確認、mini 3 ファイル無改変)。
- **デモ**: `TPCDAQ_DEMO_PROFILE=elitpc` 切替(ジオメトリ = ELITPC .dat / asad_count 4 /
  expect 0:0..0:3 / build_timeout_ms 5000 / VCOBO_READY_S 180 = 4 GiB ロード対策 /
  run_once に `TPCDAQ_DEMO_WS_PROBE_S`)。既定は従来 mini。

### テスト(発注側追試済み)

- `test_vcobo_core` **92 → 148 passed / 0 failed**(マージ 6 本 + peek。単純連結への
  ミューテーションで 17 fail = 赤の実在確認済み)。ci 57 / heartbeat 6 / oracle walk PASS
  全て無改変 green。Rust・他リポ部分は非接触。

### 受け入れ実測(2026-08-16)

- **elitpc 一周**: merge ログ `4 files, 15408 frames, 4295503872 bytes, eventIdx 0..3851`。
  61.46 s 取得で **events_complete=304 / late=0**、receiver frames 1218
  (AsAd 毎 305/305/304/304)、overflow/malformed 0。graw 4 ファイル出力は各ソース先頭と
  **sha256 一致**。run0001.root 単一(305 entries / 646 MB)。ws_probe: ELITPC ジオメトリ
  (U132/V225/W226)のヒスト・イベント配信を確認。run/start 13.3 s(configure 8.3 s)/
  run/stop 1.3 s。
- **mini 回帰(変数なし)**: events 108 / incomplete 0 / graw バイト一致 / ヒスト sum
  改修前と完全一致。無変更 green。

### 逸脱の裁定(発注側)

- **incomplete=1(受け入れ文言との差)— 受理**: `ecc stop` はフレーム境界で止まるため
  4 AsAd では高確率で末尾イベントが欠ける。実機の stop でも同じことが起きる性質のもの
  であり、欠けは counted + 当該イベントも記録される(silent でない)。
  「vcobo をイベント境界で止める」改造は**実機と挙動が変わるのでやらない**(裁定)。
- build_timeout 5000 ms(elitpc 生成設定のみ)/ WS probe 窓 / READY_S 延長 — 全て受理。
- **要調査(→ 057 起票)**: 初回 elitpc run で実 ECC の configure が 261 s(2 回目 8.3 s、
  原因未特定)。その間 controller の `DEFAULT_ECC_TIMEOUT = 60 s`(REQ rcvtimeo)が
  **発火しなかった**。タイムアウトが効いていない疑い。

- 実行環境: macOS Darwin 25.5.0、実 ELITPC 4 本組(reference/exp_data/2026、読み取りのみ)。

**Status: COMPLETED**
