# 022 — root-sink ヒスト集計 + モニタ PUB(C++)

**Status: COMPLETED**(2026-08-14 implementer/Opus 実装 → Fable レビュー PASS)

## 結果

- **実装**: 新規 monitor_hist.hpp(283 行、HistAccumulator)/ monitor_pub.hpp(314 行、
  msgpack 書き側 + Encoder)/ test_monitor_hist.cpp(202 CHECK)/ test_monitor_pub.cpp
  (87 CHECK)/ tests/root_sink_monitor_pub.rs(統合 6 本 + #[ignore] 計測 1 本)。
  改修 root_sink.cxx(+322/-23)/ root_recorder.hpp(+93、write_monitor_root +
  bytes_written atomic)/ test_recorder.cxx(+156、monitor.root 2 本)/ Makefile / .gitignore。
- **テスト(エージェント実行 + Fable が全ゲート再実行で裏取り、2026-08-14、
  macOS Darwin 25.5.0 / ROOT 6.36.10)**:
  - `make -C tools/root_sink test`: tpc_wire 68 / rs_core 71 / eb_core 175 / geo 426 /
    **monitor_hist 202** / **monitor_pub 87** 全 green(conformance は GOLDEN 未指定 SKIP —
    従来どおり run_conformance.sh 経由で実行)。
  - `make test-root`: recorder 169→**216**(monitor.root 書き出し・0 イベント run 無出力を
    追加)green。pevent 101(構造一致は TPCDAQ_REAL_PEVENT 未設定 SKIP — 実機ファイルは
    リポに入れない、既存どおり)。
  - `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings`: クリーン。
  - `TPCDAQ_ROOT_SINK_BIN=… TPCDAQ_REAL_GRAW=… cargo test`: 275 passed / 0 failed /
    1 ignored(既存 269 + 新規 6)。
  - **オラクル照合値**: hist_snapshot ビン値 = Rust 独立再計算(ジオメトリ・デコードとも
    Rust 本番実装)と全一致(合成)。実データ(mini 108 イベント / 15,040,512 items):
    **StripTime 総和 = 963,434,812** が Rust 独立計算と一致。monitor.root は TFile
    読み戻しでビン値・軸レンジ一致(test_recorder)。R10: EOS から 10 s 以内を実測 green。
  - **§12-8 初計測**(実 graw ×4 周 = 240.7 MB 全速、release): snapshot 0 Hz 7.803 s /
    1 Hz 7.824 s(+0.27%)/ 2 Hz 7.817 s(+0.18%)— ±2% 内。ただし合成ジオメトリ
    (11 ch)での配信コスト測定。正式ゲートは実ジオメトリの P3 E2E。
- **レビュー(Fable)**: monitor_hist の充填意味論 = §5.2 v1.9 と一致(生 ADC / -1
  センチネル / ≥4095 / セクション別エントリ / range 外カウンタ 2 種)、monitor_pub の
  ワイヤ = §5.3 v1.9 と一致(エンベロープ・キー名・f64 LE bin・§2.4 再直列化 round-trip)、
  Publisher のロック規約 = §5.1 遵守(直列化・send は全てロック外、期限ベース sleep)、
  shutdown 順序(publisher join → ctx_term)良。**逸脱 8 件すべて受理**:
  ① PUB bind 失敗は非 fatal(保存 > モニタ。stderr ERROR + `pub_bind_failed=1` で可視 —
  Fable が単独再現で exit 0・保存継続を実証)② 役割引きはフラグメント毎キャッシュ
  (結果同一)③ frames_per_cobo 非ゼロのみ ④ Hz は非負整数 ⑤ 期限ベース sleep
  (固定 tick だと 20 Hz が 10 Hz に頭打ちになる正しい指摘)⑥ publish_drops に送信失敗も
  計上(published + drops = 全イベントで閉じる)⑦ .gitignore 2 行(受理)
  ⑧ 既存テスト期待値変更ゼロ。
- **既知の申し送り**: フルスイート並列で root_sink_intake 2 本が 1/6 ラウンド flake
  (Fable が原因特定: **`free_endpoint()` の TOCTOU ポート競合** — 確保→手放し→sink bind の
  隙に他者が取ると PULL bind 失敗 = exit 4。022 以前からのテストインフラ弱点で、022 の
  並列ポート取得増で露出率が上がっただけ)。対策(spawn 直後の早期死検出 + 再スポーン、
  Sink Drop ガードの intake への横展開)は次のテストインフラ小ユニットへ。
**仕様**: SPEC **v1.9** §5.1(集計の持ち主 + スレッド規約)/ §5.2(9 枚 + 飽和率、
**v1.9 の波高明確化を含む**)/ §5.3(**PUB ワイヤ形式 v1.9 確定済み — 本書で再定義しない**)/
§2.2(エンベロープ)/ §2.4(Fragment 再直列化)/ §3.2(PUB 47004、source_id=101)/
§6.5(`run{run:04}_monitor.root`)/ §12-8(モニタ非干渉)/ §12-9(R10 = EOS から 10 s)
**依存**: 018(geo.hpp — `Geometry::lookup` / `max_strip`)、020(BuiltEvent / Recorder 骨格)
**発注先想定**: implementer/**Opus**(スレッド追加 + 非干渉の工学判断が残る)

## 背景(1 段落だけ)

モニタ系はここが起点: root-sink(ロスレス系で全イベントが通る唯一の場所)が 9 枚のヒストを
集計し、PUB(47004)で monitor(Rust、次ユニット 023)へ流す。**保存(TTree/Recorder)への
非干渉が絶対条件**(§5.1 — Writer スレッドとロックを共有しない。モニタ tee は drop 可・
ドロップはカウント)。

## やること

1. **`tools/root_sink/monitor_hist.hpp`(新規、純ヘッダ。ROOT/ZMQ 非依存)**
   - `HistAccumulator`: `tpcgeo::Geometry` 参照 + 9 枚分の `std::vector<double>` +
     飽和カウンタ(saturated/counted × 3 面)+ `reset()`(run open で呼ぶ)+
     `on_event(const BuiltEvent&)`。**O(items) の配列加算のみ**。バッファは ctor/reset で
     確保し、per-event の heap 確保をしない(チャンネル波高の scratch は `[4][68]` 固定配列を
     フラグメント毎に初期化)。
   - ビン数はジオメトリから: `nx = max_strip[plane]`、2D は nx×512、1D は 512。
   - フィル規則(§5.2 v1.9 のとおり。要点のみ再掲):
     - per item: `lookup(cobo,asad,aget,raw_ch)` が **Strip のときだけ**。
       StripTime{U,V,W}: `bins[(strip-1)*512 + bucket] += adc`(生 ADC weight、R3)。
     - 波高 = チャンネル毎(= 物理ストリップ毎)の max 生 ADC。**そのイベントで
       サンプル ≥1 のチャンネルのみ**。Charge{U,V,W}: 1 チャンネル 1 エントリ、
       **ビン = 波高 / 8**(512 ビン × 幅 8 = [0,4096)。波高 ≤ 4095 なので clamp 不要)。
       同一ストリップ番号の複数セクション = 同一ビンへ別エントリ。
     - ChargeMax{U,V,W}: イベント毎・面内の波高最大 1 エントリ(その面に計数チャンネルが
       1 本も無ければ入れない)。
     - 飽和率: 計数チャンネル毎に counted++、波高 ≥ 4095 なら saturated++(run 積算)。
     - incomplete イベントも届いた分で fill。late fragment は**呼ばない**(呼び元の責務)。
2. **`tools/root_sink/monitor_pub.hpp`(新規、純ヘッダ。ZMQ 非依存 = bytes を組むだけ)**
   - msgpack **書き側**(既存 tpc_wire.hpp は読み側のみ): map/str/uint/bool/f64/bin の
     最小エンコーダ + §2.2 エンベロープ(`{"Data": [101, run, seq, created_ns, payload]}`)。
   - §5.3 v1.9 の 3 種 payload(`status` / `hist_snapshot` / `built_event`)を SPEC の
     キー名・型・添字順のとおりに。`hist_snapshot.bins` は f64 LE の Bin
     (msgpack float64 連打は 9 B/ビンで 2 倍以上太る — SPEC がそう定めた理由)。
   - `built_event.fragments` = §2.4 positional array の**再直列化**。単体テストで
     tpc_wire.hpp(本番パーサ)に読み戻して round-trip 一致を機械照合(リポ内適合)。
3. **root_sink.cxx 配線**
   - **Publisher スレッド新設**(PUB を所有。bind 既定 `tcp://*:47004`、
     `kPubSndHwm = 10`(定数。スナップショット ~2.4 MB 級なので小さく)、LINGER 0、drop 可)。
     tick 100 ms で回し: status **1 Hz 固定**(run 外でも常時。state は `"idle"` / `"running"`、
     run は idle 時 = 直近クローズした run(無ければ 0))/ hist_snapshot `snapshot_hz`
     (既定 1、**0 = off**)/ built_event ≤ `event_publish_hz`(既定 20、0 = off)。
     seq は全 kind 単一系列・**run リセット無し**(v1.9 ③)。
   - **ロック規約(§5.1)**: `hist_mutex` は集計スレッド ↔ Publisher **のみ**
     (Recorder スレッドは触らない)。ロック中に許すのは配列 fill / memcpy /
     数個の u64 コピーだけ。**msgpack 直列化と zmq send は必ずロック外**。
   - **built_event 最新優先(コピーを間引く設計)**: atomic の hungry フラグ + mutex 付き
     slot。Publisher は配信間隔が来たら hungry を立てる → 集計スレッドは emit 時に
     hungry なら BuiltEvent を**コピー**して slot へ入れ hungry を下ろす(コピーは高々
     event_publish_hz 回/秒に律速 — 100 Hz × ~2 MB/event を毎回コピーしない)。
     hungry でなければ `publish_drops++`(カウンタのみ)。event_publish_hz=0 のときは
     配信 off で **publish_drops も数えない**(設定による停止はドロップではない)。
   - **status 材料**: 集計スレッドが hist_mutex 下で小さな StatusSnapshot を更新
     (run / state / events_built = complete+incomplete / events_incomplete /
     late_fragments / frames_per_cobo は `uint64_t[256]` 固定配列)。`bytes_written` は
     Recorder に atomic アクセサを追加して読む(--no-root は 0)。飽和カウンタは
     HistAccumulator から。
   - **有効条件**: ヒスト集計 + hist_snapshot/built_event は **--geometry があるときのみ**
     (pevent 既定モードは常に有効。gdataframe 回帰モード・--geometry 無し --no-root は
     status のみ)。status は Publisher が居る限り常時。
   - **run 境界**: run open(RunState Opened)で `HistAccumulator::reset()`。
     run close(EOS)で最終ヒストを **RunClose RecordItem に同梱**(run 毎 1 回のコピー)→
     Recorder スレッドが TTree finalize 後に TH1D/TH2D を生成し
     `run{run:04}_monitor.root` を書く(**ROOT IO は Recorder スレッドのみ**の原則を維持。
     ヒスト名 = SPEC §5.2 の `StripTimeU` 等 9 枚、軸レンジ: x=1..N+1(strip)/
     y=0..512(bucket)/ 1D は [0,4096])。R10 = EOS から 10 s 以内(§12-9 — 即時書きで
     自明に満たす。テストで実測)。
   - **0 イベント run は monitor.root も作らない**(§6.5 の遅延オープンと同じ理屈)。
     **SIGINT 等で run 未クローズなら書かない**(inprogress の run.root と同じ
     「完全 run に化けない」原則。ヒストは PUB で見えていた + 生 graw がバックストップ)。
   - 終了 JSON に `snapshots_published` / `events_published` / `publish_drops` を追加。
     起動時 stderr に pub endpoint / snapshot_hz / event_publish_hz を 1 行出す。
4. **CLI**: `--pub ENDPOINT`(既定 `tcp://*:47004`)/ `--snapshot-hz N`(既定 1、0=off)/
   `--event-publish-hz N`(既定 20、0=off)。usage() 更新。
5. **Makefile**: `test_monitor_hist` / `test_monitor_pub` を `make test` に追加。

## テスト(テストファースト。オラクルは手計算 + Rust 二重実装)

- **C++ 単体 `test_monitor_hist.cpp`**(合成 .dat をテスト内文字列で構成):
  StripTime の添字と weight 加算 / Charge のビン = 波高/8・1 ch 1 エントリ /
  セクション合流ビンへの別エントリ / ChargeMax の面毎 max / FPN・Aux・Unmapped 除外 /
  サンプル 0 チャンネル非計数(飽和分母含む)/ incomplete fill / 飽和 ≥4095 /
  reset で全ゼロ — すべて期待ビン値の機械照合。
- **C++ 単体 `test_monitor_pub.cpp`**: エンコーダ出力 vs 手組み msgpack 期待バイト列
  (3 種 × 代表 1 通ずつ)/ エンベロープが §2.2 形 / seq 単調 / fragments 再直列化を
  tpc_wire.hpp で読み戻して round-trip 一致 / bins の f64 LE 添字順。
- **Rust 統合 `tests/root_sink_monitor_pub.rs`(新規、env `TPCDAQ_ROOT_SINK_BIN` gate —
  root_sink_intake.rs の流儀)**: 合成フラグメントを PUSH + SUB で受け、rmp-serde
  (map 形式なので named struct で受かる — **023 の本番パーサの先行形**):
  - status が流れる(idle でも)+ フィールド値(run/state/events_built/saturation)一致。
  - EOS 後の hist_snapshot が **Rust 側で独立再計算した期待ビン値と全一致**
    (ジオメトリ・デコードは Rust 本番実装で再現 = §4.5 と同じ二重実装照合)。
  - built_event: complete フラグ / fragments が Rust 側 Fragment パースで読める /
    event_idx が単調 / 高速送出 + 小 event_publish_hz で `publish_drops > 0`。
  - `--snapshot-hz 0` で hist_snapshot ゼロ・status は継続。
  - EOS → `run{run:04}_monitor.root` 存在 + **mtime − EOS 送信時刻 ≤ 10 s**(R10)。
    run.root(TTree)側の既存回帰が無影響であること。
- **C++ `test_recorder.cxx` 追記**: 既知配列 → monitor.root 書き出し → TFile 読み戻しで
  ヒスト名・ビン数・ビン値・軸レンジ一致。0 イベント run で monitor.root 無し。
- **実データ E2E(env `TPCDAQ_REAL_GRAW` gate、Rust 統合に同居)**: mini 実 graw リプレイ →
  events_built=108、StripTime 総和 = Strip 割り付けチャンネルの生 ADC 総和
  (Rust 側で独立計算)と一致。
- **非干渉の初計測(§12-8 の素材 — 自動テストにしない。結果節に記録)**: 実 graw 全速
  リプレイを snapshot 0/1/2 Hz で各 3 回、wall time を記録(正式ゲートは P3 E2E)。

## 受け入れ

- 上記全テスト green。`make test`(tools/root_sink)+
  `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 通過(既存無影響)。
- ファイル所有権: tools/root_sink/{monitor_hist.hpp, monitor_pub.hpp, test_monitor_hist.cpp,
  test_monitor_pub.cpp, root_sink.cxx, root_recorder.hpp(monitor.root 書き + bytes_written()
  アクセサ追記), test_recorder.cxx(追記), Makefile} + tests/root_sink_monitor_pub.rs(新規)。
  **SPEC・他ユニットのファイルに触らない**。発注書に無い設計分岐に出会ったら実装せず報告。
