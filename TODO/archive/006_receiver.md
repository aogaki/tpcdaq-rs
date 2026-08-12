# 006 — receiver コンポーネント(CoBo 毎、never-stop)

**Status: COMPLETED**(2026-08-12。実装 = implementer/Opus、レビュー = Fable。P1 出口条件達成)
**仕様**: SPEC §1.1(責務)、§1.3(状態機械、Arm = bind+listen)、§1.4(過負荷規約)、
§2.2/§2.3(Batch/RawFrames/EOS/Heartbeat/バッチ詰め)、§3(config)
**依存**: 001(config)、003(msg/command/zmq_helper)、004(framer)、005(graw_replay — テストで使用)

## やること

1. **ロギング基盤導入**(003 レビュー裁定の実行): `tracing` + `tracing-subscriber` を依存に追加。
   `src/command.rs` の `eprintln!` を tracing に置換。bin 側で subscriber を初期化。
2. `src/receiver.rs` — receiver 本体(tokio。delila-rs component_architecture のタスク分離):
   - **drain タスク**: TCP 読み → `framer::Framer` → フレームを有界 mpsc へ。**ソケットは常に
     読み続ける(never-stop)**。キュー満杯 = `overflow_frames` カウント + Error 状態遷移 +
     当該フレーム破棄(ブロックしない — SPEC §1.4-3。tracing warn + カウンタで可視化)。
   - **バッチャ/送信タスク**: PUSH ×2(graw-writer 行き / decoder 行き。**独立ソケット・独立
     sequence_number**)。バッチ詰めは「8 MiB 到達 or 10 ms 経過」の早い方(設定可)。
     per-frame の heap 確保・ZMQ send をしない。HWM は zmq_helper 経由(既定 1000)。
   - **コマンド REP タスク**: 003 `run_command_task` を使用。
   - 状態機械: `Configure`(設定確定)→ `Arm`(**bind + listen** — listen-before-start の実装点。
     実際に bind したアドレスを CommandResponse.metrics に含める〔controller が DataLinkSet 生成に
     使う — SPEC §8.2〕)→ `Start{run}`(accept 開始、seq=0 リセット)→ `Stop` / `Reset`。
   - **TCP EOF = run 境界** → 両リンクへ `EndOfStream{source_id=cobo_id, run_number}`。EOF 後は
     次の accept へ戻る(次 run に備える)。`Stop` コマンド → listener close + 未送 EOS を送出。
   - Running 中のアイドル時 Heartbeat 1 Hz。
   - カウンタ(metrics で返却): bytes / frames / batches / overflow_frames / framer reset_count。
3. `src/bin/receiver.rs` — `receiver --config <toml> --cobo-id <k>`。tracing 初期化 + 設定読込 + 起動。
4. config: `[receiver]` セクション(batch_max_bytes=8MiB / batch_max_ms=10 / queue_frames 等)を
   `src/config.rs` に追加(001 レビュー裁定「内部ポート・パラメタはそれを使うユニットで足す」の適用)。
   receiver の PUSH 接続先(graw-writer / decoder の PULL アドレス)も config に追加
   (既定 §3.2: 47001 / 47002)。

## テスト

- 統合(すべて port 0、固定ポート禁止):
  (a) fake CoBo(テスト内 TCP client)→ receiver → PULL ×2 で**バイト再構成一致**(RawFrames 連結 = 入力)
  (b) 両リンクの sequence_number 連続
  (c) EOF → 両リンクへ EOS 到達
  (d) Configure→Arm→Start→データ→EOF→Stop の全シーケンス(metrics に bind アドレスが出ること含む)
  (e) Arm 前は接続できない(listen-before-start の負性テスト)
  (f) run 跨ぎ: EOF 後の再 accept、seq リセット、次 Start の run_number 反映
- **過負荷**: PULL 側を止めて内部キューを溢れさせ、`overflow_frames` が増えて Error 状態になり、
  **その間も drain が継続する**(fake CoBo の送信がブロックされない)ことの検証。
- **実 .graw リプレイ**(env `TPCDAQ_REAL_GRAW` 時、graw_replay バイナリ使用):
  全速リプレイ → PULL 側連結バイトがファイルと完全一致、overflow=0、フレーム数 109(CoBo 108 + 制御 1)。
  ローカル実行して実測値を `## 結果` に記録。

## 受け入れ(= P1 出口条件)

- 上記全テスト green(実 .graw リプレイで byte 一致 + overflow/drop 0 — 100 Hz 相当を大きく超える全速)。
- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 通過。

## 結果

**実行環境**: macOS 26.5.2 / arm64(Apple Silicon)、rustc 1.97.1、cargo 1.97.1、2026-08-12。

**実行コマンド**

```
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
TPCDAQ_REAL_GRAW=/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
  cargo test --test receiver_real_graw -- --nocapture
```

**テスト数**: リポ全体 **153 passed / 0 failed**(006 で新規 16)。clippy `-D warnings` クリーン、fmt 差分なし。

新規テスト:

| ファイル | テスト | 対応 |
|---|---|---|
| `src/config.rs` | `receiver_section_defaults_when_omitted` / `receiver_section_values_override_the_defaults` / `receiver_degenerate_values_are_err` / `receiver_unknown_field_is_err` / `receiver_command_port_base_matches_spec_3_2` | `[receiver]` 追加(§3) |
| `src/bin/receiver.rs` | `parses_config_and_cobo_id_in_any_order` / `rejects_missing_or_malformed_arguments` | CLI |
| `tests/receiver_integration.rs` | `both_links_reassemble_the_cobo_bytes_exactly` | (a)(b)(c) |
| | `eof_without_any_data_still_delivers_end_of_stream` | (c) |
| | `stop_emits_the_pending_end_of_stream_when_the_cobo_never_closes` | Stop の強制 EOS(§1.3-2) |
| | `full_command_sequence_reports_the_bound_address` | (d) |
| | `nothing_listens_before_arm` | (e) |
| | `eof_returns_to_accept_and_the_next_run_resets_sequence_numbers` | (f) |
| | `an_idle_running_receiver_emits_heartbeats_on_both_links` | Heartbeat(§2.2) |
| `tests/receiver_overload.rs` | `queue_overflow_counts_frames_enters_error_and_keeps_draining` | 過負荷 |
| `tests/receiver_real_graw.rs` | `real_graw_replay_is_byte_identical_on_both_links` | 実 .graw |

**実 .graw リプレイ実測**(`CoBo_2025-09-01T08_51_06.203_0000.graw` = 30,108,684 B、全速・ペーシングなし)

| 項目 | 実測 | オラクル |
|---|---|---|
| 両リンクの連結バイト | 30,108,684 B、ファイルと**完全一致** | byte 完全一致 ✔ |
| フレーム数(両リンク / metrics) | 109 / 109 / 109 | 109(CoBo 108 + 制御 1)✔ |
| overflow_frames | 0 | 0 ✔ |
| framer_resets | 0 | 0 ✔ |
| バッチ数 | 5(8 MiB 到達 4 回 + 残り) | — |
| 所要 | 0.136–0.157 s(debug、3 回)= 約 **192–221 MB/s**。release も 0.191 s(律速は graw_replay 側の送出) | 100 Hz(mini で約 28 MB/s)を大きく超える |

参考: 全フレームがファイルを隙間なく埋めること(12 B の制御フレーム 1 + 278,784 B × 108 = 30,108,684 B)は
独立スクリプトでも確認済み。

**過負荷テスト実測**(PULL を止め、HWM=1・`queue_frames=2`・8 KiB フレーム × 512 = 4 MiB)

- fake CoBo の 4,194,304 B 送出が **2.95 ms で完走**(= drain がソケットを読み続けている。
  drain が止まれば TCP バッファが詰まって完走しない)。
- 受信 `bytes=4,194,304`(全バイト読み切り)/ `frames=512` / `overflow_frames=435` / `batches=77`。
- 状態は `Error`、`Stop` は拒否され `Reset` でのみ Idle へ戻る(SPEC §1.3/§1.4-3)。

**フレーク**: `receiver_integration` + `receiver_overload` を 20 連続実行して 0/20 失敗。
実 .graw リプレイは 3 連続で同値。

**逸脱・迷った点**

1. **Cargo.toml が tracing だけで済まなかった**(発注書は「tracing / tracing-subscriber の追加のみ」)。
   `tokio` の features に `net`(TcpListener/TcpStream)・`io-util`(AsyncReadExt)・`signal`(bin の
   Ctrl-C)を足した。既存 features では TCP が使えないため不可避。依存クレートの追加はしていない。
   `tracing-subscriber` は `env-filter`(= `RUST_LOG` 対応)を有効にした。
2. **送信タスクは tokio タスクではなく専用 OS スレッド**(`std::thread`)、キューは
   `std::sync::mpsc::sync_channel`(有界)。理由: ZMQ の PUSH 送信は HWM 到達で**ブロックする**
   のが背圧の実体で、これを tokio ワーカ上で待たせると他タスクを巻き込む。`sync_channel` なら
   drain 側は `try_send`(非ブロック)、送信側は `recv_timeout`(バッチ 10 ms の締切と同じ道具)で
   済み、delila-rs が警告する `blocking_send` の macOS TLS 問題も踏まない。ZMQ は tmq ではなく
   同期 `zmq` を選択(送信側は非同期である必要がない)。
3. **`overflow` → Error の反映点**。溢れの検出はホットパス(drain)、状態への反映は次のコマンド
   処理時(`AtomicBool` のラッチを見る)。状態は REP 経由でしか観測できないので意味論は同じで、
   ホットパスにロックを持ち込まずに済む。`Reset` でラッチも降ろす。
4. **per-frame の `Vec<u8>` 確保だけは残る**。`RawFrames = Vec<ByteBuf>` の型(= 1 フレーム 1 bin)
   に由来し、プール化は KISS 違反と判断した。バッチのペイロードは**借用のまま**符号化して
   (`Batch<&Vec<ByteBuf>>`)リンク毎のコピーは作らない。バッチ用 `Vec` も使い回す。
5. **設定に無い決めごと**: receiver の REP アドレスは SPEC §3.2 の式 `tcp://*:47110+cobo_id` を
   `ReceiverParams::from_config` で組む(TOML には出さない)。`queue_frames` 既定 512 は
   「目標 100 Hz の 2 秒分 = 200 フレーム」(SPEC §1.4-2)に約 5 秒の余裕を見た値。
   `hwm` / `heartbeat_ms` は `[receiver]` の任意項目として追加(発注書「等」の範囲。テストで
   周期・詰まり具合を縮められるようにするため)。
6. **`Arm` の失敗は Error にせず Configured 据え置き**(bind 失敗 = ポート衝突は再 Arm で回復可能)。
   `Start` は下流 PUSH を張ってから listener を取り上げるので、下流の失敗時も Armed のまま。
7. **linger は libzmq 既定(無限)のまま**にした。有限にすると停止時に未送分を黙って捨てうるため
   (保存系はデータを落とさない)。代償として「下流が完全に死んだまま停止」した場合に送信スレッドが
   残るが、run 中の背圧としては正しい挙動なのでこのままにしてある。
8. **`Stop` → `Arm` を即座に連打すると同一ポートの再 bind が競り得る**(旧 listener の close は
   drain タスク側で非同期に起きる)。SO_REUSEADDR は std が付けるので実害は小さいが、
   完全な同期 close にはしていない(KISS)。運用上の Stop→次 run は人・controller の時間尺度。
9. `Configure` の `config` フィールド(JSON 断片)は**解釈していない**。正本は起動時 TOML で、
   発注書にも上書き規則が無いため。必要になれば別チケット。

### レビュー裁定(Fable、2026-08-12)

コードレビュー(drain/送信ループ・EOS 経路・Metrics)+ ゲートと実 .graw リプレイの独立再実行
(153 passed / 0 failed、バイト完全一致 30,108,684 B、frames 109/109、overflow 0、0.160 s ≈ 188 MB/s
= 100 Hz 相当 28 MB/s の 6.7 倍で drop 0)。**P1 出口条件達成を確認**。裁定:

1. tokio features 追加(net/io-util/signal)— 承認(不可避、依存クレート追加なし)。
2. 送信 = 同期 zmq + 専用 OS スレッド + sync_channel — 承認。PUSH のブロック(背圧の実体)を
   tokio ワーカから隔離する判断は正しい。REP = tmq(003)/ PUSH = 同期スレッドの使い分けも妥当。
3. overflow ラッチのコマンド時反映 / Arm 失敗 = Configured 据え置き / linger 無限(ロスレス優先)/
   queue_frames=512 の根拠 / REP アドレスの式生成 / Configure JSON 未解釈 — すべて承認。
4. per-frame の Vec 確保 — 承認。CLAUDE.md の禁止の趣旨は open/close・ログ整形級のコスト。
   278 KB フレームに対する 1 alloc は無視できる(mini 100 Hz で ~400 alloc/s)。プール化は
   プロファイルが要求したときに。
5. **レビュー指摘(非ブロッカー、P2 で再訪)**: 下流 PULL が全死 + EOF 前に Reset された場合、
   `send_end_of_stream` の再試行が畳めず drain タスクが残る可能性がある(linger 無限 × キュー恒久
   満杯の合わせ技)。graw-writer 実装(P2)で停止オーケストレーションを固めるときに、
   shutdown 経路だけ EOS 待ちに上限を設けるか検討すること。
