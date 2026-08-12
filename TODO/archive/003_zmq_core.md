# 003 — ZMQ メッセージ核 + コンポーネント状態機械

**Status: COMPLETED**(2026-08-12。実装 = implementer/Opus、レビュー = Fable)
**仕様**: SPEC §2 全節、§1.3(状態機械)、§1.4(HWM・背圧規約)
**依存**: 001

## やること

1. `Message` / `Batch` / `Fragment` 型(SPEC §2.2/§2.4)+ rmp-serde 直列化(positional)。
   **スキーマ漂流ガード**: フィールド順序・型タグの定数表 + 実構造体との一致テスト
   (delila-rs `delila_schema.rs` 方式 — §2.5)。
2. item u32 パック/アンパック(§2.4 のビット割り)+ 境界値テスト。
3. `zmq_helper`: **有限 HWM** 方針の単一チョークポイント(§1.4)。delila-rs の HWM=0 と違う
   選択であることと理由を docstring に明記。
4. JSON `Command` / `CommandResponse` + `ComponentState` 遷移表(§2.6)。
   REP コマンドタスク(ソケットエラー時は再 bind — delila-rs TODO 58 の教訓)。
5. 最小配線デモ: PUSH/PULL(1:1)+ REQ/REP の echo コンポーネントをテスト内で回す
   (tokio、コンポーネント骨格 = P0 の「ZMQ 配線」の実証)。

## テスト

- 直列化ラウンドトリップ + 漂流ガード。golden バイト列 fixture の生成機能
  (root-sink C++ 側デコーダの照合材料 — §10.4 と同じ発想)。
- 状態遷移の合法/非合法全パターン。
- EOS がいかなる間引き経路にも乗らないことの経路レベル担保(§2.2)。
- 有界 PUSH/PULL で受け手停止時に送り手がブロックする(背圧の実証、小さい統合テスト)。

## 受け入れ

- 全テスト green。echo デモが Configure→Arm→Start→Stop の全遷移を通る。

## 結果

**実行環境**: Darwin 25.5.0 arm64 / rustc 1.97.1 / cargo 1.97.1 / libzmq 4.3.5、2026-08-12。

**実行コマンド**

```
cargo fmt
cargo clippy --tests -- -D warnings        # 警告ゼロ
cargo clippy --all-targets -- -D warnings  # 警告ゼロ
cargo test
```

**テスト数**: 003 で追加した 48 件すべて green(red → green を確認。実装前の
`cargo test` は 167 compile error = red)。skip したテストなし。

| 置き場所 | 件数 | 内容 |
|---|---|---|
| `src/msg.rs` | 24 | 直列化ラウンドトリップ、ワイヤ表現(bin/fixmap/バイト固定)、スキーマ漂流ガード、item パック、長さ前置ストリーム、間引き分類 |
| `src/command.rs` | 15 | 遷移表 5×5 全パターン、JSON 表現(SPEC §2.6 の形)、ラウンドトリップ |
| `src/zmq_helper.rs` | 4 | 有限 HWM の設定(push/pull/pub/sub)、HWM=0 の拒否 |
| `tests/zmq_echo.rs` | 1 | echo デモ(REQ/REP + PUSH/PULL)。Configure→Arm→Start→Stop→Reset 全遷移 + 非合法遷移 + 壊れた JSON + Data/EOS 受信 |
| `tests/zmq_backpressure.rs` | 2 | 有界キューの EAGAIN(決定的)/ 受け手停止で送り手がブロック → 排出で解放 |
| `tests/zmq_golden_stream.rs` | 2 | golden fixture のファイル生成と既知値照合、決定性 |

リポ全体では 98 件 green(002 の geometry/config 27 件 + config 既存分を含む)、0 failed。

**確認した数値・値**

- `pack_item(2,45,300,1234)` = `0x96CB_04D2`(手計算: `2<<30 | 45<<23 | 300<<14 | 1234`)。
- 全フィールド最大 `pack_item(3,127,511,4095)` = `0xFFFF_CFFF`、予約ビット `[13:12]` は 0。
- `Message::EndOfStream{1,7}` のワイヤバイト列 = `81 ab "EndOfStream" 92 01 07`(16 バイト)。
- 生フレーム・items が msgpack **bin**(`0xc4` 系マーカー)で載ること = 配列フォールバックなし。
- 漂流ガードの有効性を変異テストで確認: `Fragment` の `cobo`/`asad`(同型)を入替えると
  `schema_table_matches_fragment_wire_layout` が落ちる。
- 背圧: HWM=2・256 KiB/通で、受け手停止時に送り手が 64 通送り切る前に停止し、排出で解放。
- ZMQ 統合テスト 20 連続実行でフレークなし(0/20 失敗)。

**逸脱・迷った点**

1. **tmq を選んだ**(zmq + spawn_blocking ではなく)。理由: REP タスクは `tokio::select!` で
   shutdown と多重化する必要があり、tmq なら非同期のまま書ける。zmq 同期版だと
   `RCVTIMEO` ポーリング + ブロッキングスレッド占有になる。delila-rs `command_task.rs` も tmq で、
   再 bind の作法をそのまま踏襲できる。
2. **ログは `eprintln!`**。tracing/log は依存追加になるため入れていない(発注書の許可は
   `serde_bytes` のみ)。ログ基盤の選定は別チケットで決める必要がある。
3. **`run_command_task` に `bound_endpoint: Option<oneshot::Sender<String>>`** を足した。
   `tcp://127.0.0.1:0` で bind したときの実ポートをテストへ返すため(固定ポート禁止の要請)。
   production は `None`。再 bind 時には再通知しない(動的ポートは production では使わないため)。
4. **`pack_item` は `Result`**(マスクして黙って切り詰めない)。`chan` の検査はビット幅
   (7 bit = 0–127)に対して行う。SPEC §2.4 の運用値 0–67 の検査は電子回路空間を知る
   デコーダ側の責務と判断した。
5. **`items_to_bytes` / `items_from_bytes` を追加**(発注書に明記なし)。`Fragment::items` の
   u32 LE 連結を各所で手書きすると境界ずれが silent に入るため。`items_from_bytes` は
   長さが 4 の倍数でなければエラー。
6. **遷移表は delila-rs のものをそのまま採用**(SPEC §1.3「そのまま採用」)。結果として
   `Idle → Error` は不許可のまま。Configure 中の失敗を Error にできないが、仕様どおりなので
   変更していない(仕様側の判断が要るなら別途)。
7. 背圧テストで **linger=0 のソケットを送信スレッドが drop すると未送出分が破棄される**罠を踏んだ。
   受信完了まで送り手にソケットを持たせる形に直した(テスト側の問題であり production 経路の話ではない)。

### レビュー裁定(Fable、2026-08-12)

コードレビュー(msg.rs 全体 + command.rs 遷移表/再 bind + zmq_helper)+ ゲート独立再実行で確認。
ワイヤ表現(fixmap(1) enum / positional array / serde_bytes bin)は SPEC §2 と delila-rs 方式に
一致。裁定:

1. tmq 採用 — 承認(delila-rs command_task の作法をそのまま踏襲できる)。
2. `eprintln!` — 暫定承認。**P1 receiver ユニットで tracing を導入し、この eprintln! を置換する**
   (発注書に含める)。
3. `Idle → Error` 不許可 — 妥当。Configure 失敗は「Err 応答 + Idle 滞留」で表現でき、
   delila-rs と同じ意味論。SPEC 変更不要。
4. `pack_item` の chan 検査はビット幅まで(0–67 の運用検査はデコーダ責務)— 承認。
5. 発注書外の追加(items_to_bytes/from_bytes、bound_endpoint)— 承認。境界ずれの silent 化
   防止と port-0 テストはどちらも規約に沿う。
6. 変異テストで漂流ガードの有効性を実証した点は特筆(同型フィールド入替の検出)。
