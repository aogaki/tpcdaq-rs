# 015 — JSONL ログブック + run 番号永続化(Rust 純モジュール)

**Status: OPEN**
**仕様**: SPEC v1.6 §9(全部 = 本ユニットの核)、§8.1(`tpcdaq_state.json` の next_run)、
§12-11(kill -9 耐性 = 受け入れ)
**依存**: なし(純モジュール。controller = 016 が唯一の利用者になる)
**発注先想定**: implementer/Sonnet(発注書とテストで完全に縛れる)

## やること

1. `src/logbook.rs` — 単一書き手の JSONL 追記器 + リーダ:
   - `LogbookWriter::open(path)` — 追記オープン。**既存ファイル末尾から最後の有効行を走査して
     `seq` を回復**(next_seq = 最終有効行の seq + 1。壊れた最終行は無視 — §9.1 の唯一の許容)。
   - `append(record)` — `ts`(RFC3339・ミリ秒・ローカルオフセット付き、chrono)と `seq`
     (単調増加 u64)を付与し、**1 行 = 1 write(2)**(行全体を 1 つの文字列に組んでから
     `write_all` 1 回)+ 行毎 flush。
   - レコード型(§9.2 の表と一字一句一致、serde tag = `type`): `run_start` / `run_stop` /
     `audit` / `comment` / `psu`。**スキーマ漂流ガード**(§2.5 の方式 — フィールド名と順序を
     定数表に固定しテストで突き合わせ)。
   - `read_since(path, since_seq)` — リーダ(016 の REST 用)。**最終行のみ parse 失敗を許容**
     (それ以外の行の破損は Err — silent にしない)。戻りに「末尾破損あり」フラグ。
2. `src/state.rs` — `tpcdaq_state.json` の `next_run` 永続化:
   - `take_next_run(path) -> u32`: 読み → **next_run+1 を tmp ファイルに書いて rename(atomic)**
     → 元の値を返す。**永続化が成功してから値を使わせる**(クラッシュで番号が飛ぶのは可、
     **重複は不可** — §12-11)。ファイル不在時は 1 から。
   - REST からの手動設定用 `set_next_run(path, n)`(016 が audit 付きで呼ぶ)。
3. `src/lib.rs` に mod 2 行。

## テスト

- 単体: seq 単調増加 / ts 形式(RFC3339 オフセット付き regex)/ 5 レコード型の serialize が
  §9.2 の例と一致(golden 文字列照合)/ スキーマ漂流ガード / read_since のフィルタ。
- **耐久(§12-11 の再現)**: ①行の途中で切れたファイル(手で truncate)→ リーダは最終行以外
  全部 parse + 警告フラグ / writer は正しい seq で再開。②`take_next_run` を「rename 直後に
  クラッシュ」相当(値を使わず捨てる)で繰り返し → 番号は飛ぶが**重複ゼロ**。③同一 writer で
  1000 行追記 → 全行 parse 可・seq 連続。
- 依存追加なし(chrono / serde / serde_json は既存)。

## 受け入れ

- 上記全テスト green。`cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
  通過(既存 226 無影響)。
- ファイル所有権: src/logbook.rs(新規)/ src/state.rs(新規)/ src/lib.rs(2 行)。
  **これ以外に触らない**(並列で 017 が tools/ecc_bridge/ を作業中)。
