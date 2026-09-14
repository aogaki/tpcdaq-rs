# 076 — ecc-bridge の Rust 化(Ice クライアント部分集合の自前実装)

**Status: DRAFT**(調査完了 + 立案 2026-09-14、Fable。**着手はユーザー指示待ち**。
発端 = ユーザー質問「zCoBo を変えない限り Ice の版数問題からは逃れられないか」→ 3 段の現実解のうち
「2. ecc_bridge だけを Rust Ice クライアントに置換」の詳細調査依頼)

## 0. 結論(1 分で)

- **やる価値がある小さな仕事**: 置換対象は C++ 361 行(Ice 呼び出し 9 種)+ 純ロジック 469 行。
  Ice で要るのは **twoway クライアントの部分集合**(プロトコル 1.0 / エンコーディング 1.1、
  string 引数 ≤ 2、戻りは void か enum 2 つの struct、例外 1 種)。サーバ側・IceStorm・SSL・
  クラス・optional・バッチは不要。
- **効き目**: DAQ ホストの Ice 依存が「ECC 自身の 3.7 ランタイム」だけになる。現行 ecc_bridge は
  Ice **3.8 固有の C++11 API**(`StateMachinePrx(comm, str)` ctor 等)で書かれていて 3.7 では
  そのままコンパイルできず、ECC が 3.7 を要求するホストでは 3.7/3.8 共存が要る。**本日の遠隔
  ベンチでは GDED に bridge を建てられず、Mac の bridge を SSH トンネルで結線した**
  (メモリ geteccserver-native-gded)— この不便が消える。C++ サテライトは root_sink(ROOT)だけになる。
- **既製 crate は採用しない**(ice-rs は 2023-02 アーカイブ + GPL-2.0)。自前実装で 2k 行前後。
- **受け入れ資産は全部手元にある**: fake_ecc(Ice servant)、run_ecc_e2e.sh、`TPCDAQ_ECC_BRIDGE_BIN`
  を見る Rust 統合テスト 26 本、実 ECC(macOS keg 3.7.11 / GDED 3.7.2)、仮想 zCoBo デモ。
- 工数目安 **3 日 + レビュー 2 回**(Phase 0 Sonnet 0.5 / Phase 1 Opus 1 / Phase 2 Opus 1 / Phase 3 0.5)。

## 1. 背景 — 縛りの 2 層(2026-09-14 の整理)

| 層 | 何が固定するか | 中身 | 逃げ道 |
|---|---|---|---|
| ワイヤ(zCoBo getHwServer ↔ ECC) | 板の FW(変更不能) | Ice encoding 1.0 | 無い(が、ライブラリ版の縛りではない) |
| バイナリ(GET software 20190315 = ECC) | C++98 マッピング + IceUtil | Ice ≤ 3.7 でしかビルド/リンク不可 | ソースから 3.7.x で再ビルド(macOS で実証済み) |
| **我々の ecc_bridge** | **自分で選んだ 3.8 API** | 3.7 ではビルド不可 | **本ユニット** |

「ECC を捨てて板に直接話す」案は、板側が既に最薄の汎用レジスタサーバ(Slice `Node`/`Device` 33 op)で
知能は全部 ECC 側(GetBench/src/get/rc 14,054 行 + CompoundConfig 21,009 行)にあるため、Ice 輸送層を
どう置換しても意味論の再実装(数人月 + AsAd 電源手順の物理リスク)が残る → 取らない
(CLAUDE.md「制御プレーンを改変しない」)。本ユニットは **ECC はそのまま、橋だけ Rust** の話。

## 2. 調査結果

### 2.1 置換対象の実面積(tools/ecc_bridge)

| ファイル | 行 | 役割 | 行き先 |
|---|---|---|---|
| ecc_bridge.cpp | 361 | ZMQ REP + `EccClient`(Ice 呼び出し: `ice_ping` / describe / prepare / configure / start / stop / breakup / reset / getStatus)+ 例外 → Result | `src/bin/ecc_bridge.rs` |
| ecc_core.hpp | 469 | 純ロジック: `State`/`Error`/`Step` enum、`to_string`/`state_from_string`、`data_link_set_xml`/`parse_data_link_set`(+ xml_escape/unescape)、`parse_request`/`make_response`(JSON 契約 §8.2)、`next_state` | `src/ecc/core.rs`(1:1 移植) |
| test_ecc_bridge.cpp | 558(457 CHECK) | ecc_core の仕様書 | Rust 単体テストへ移植(件数不変が受け入れ) |
| fake_ecc.cpp | 379 | Ice servant のテストダブル(043 の error set/clear 規則込み) | **残す**(C++、Ice のまま) |
| ecc_e2e_client.cpp / run_ecc_e2e.sh | 291 / — | ZMQ 側の機械照合 | run_ecc_e2e.sh は Rust bin で回す(client は Rust テストに吸収可) |
| json_min.hpp / Makefile / slice/ | 273 / 120 / 生成物 | 自前 JSON / slice2cpp | 撤去(serde_json / .ice 不要) |

Ice の実使用は **`get::rc::StateMachine` の 8 op + `getStatus(out Status{State s; Error e})` + 組込み
`ice_ping`** だけ(GetEcc.ice:70-82)。引数は string ≤ 2(configure = configId + DataLinkSet XML、
XML は 255 B を超える)。例外は `mdaq::utl::CmdException { string reason }` 1 種(継承なし、
CmdException.ice)。Identity は `"Ecc"`(category 空)、エンドポイント `tcp -h H -p 46002`。
encoding は C++ bridge が **1.1 を明示強制**(ecc_bridge.cpp:116-119)。invocation timeout は未設定 =
無期限(controller 側の 60 s/REQ が正、SPEC §8.2 v1.20)。

Rust 側の受け口は既にある: controller は JSON 契約の型と組み立てを持つ(src/controller.rs
`EccReply` / `RouterLink` / `configure_request` / `ecc_error` 素通し)。**ZMQ REQ/REP と JSON は無変更**。

### 2.2 Ice ワイヤ仕様(必要部分だけ。正 = archive.zeroc.com Ice 3.7 Protocol and Encoding)

- プロトコルは 1.0 のみ。エンコーディングは 1.0/1.1 の 2 つ → **1.1 固定**(ECC = Ice 3.7 は 1.1 可)。
- **ヘッダ 14 B**: magic `IceP` / protocolMajor 1 / protocolMinor 0 / encodingMajor 1 /
  encodingMinor 0(ヘッダは常に 1.0)/ messageType / compressionStatus 0 / messageSize int32 LE(ヘッダ込み)。
  messageType: 0 Request / 1 BatchRequest / 2 Reply / 3 ValidateConnection / 4 CloseConnection。
- **接続**: サーバが ValidateConnection(ヘッダのみ 14 B)を**先に**送る。クライアントはそれを
  受けてから Request を出す。以後 type 3 が届いてもハートビートとして無視。
- **Request 本体**: requestId int32(> 0。0 = oneway、使わない)/ Identity { name string, category string } /
  facet(要素 0 の string seq)/ operation string / mode byte(0 Normal / 2 Idempotent)/
  context(要素 0 の dict)/ **encapsulation**(size int32 = 6 + 本体、major 1、minor 1、パラメータ列)。
- **Reply 本体**: requestId / replyStatus byte / payload:
  0 OK = encapsulation(out パラメータ → 戻り値)/ 1 UserException = encapsulation(例外)/
  2 ObjectNotExist・3 FacetNotExist・4 OperationNotExist = RequestFailedData(identity, facet, operation)/
  5 UnknownLocalException・6 UnknownUserException・7 UnknownException = 1.0 エンコードの string。
- **1.1 の例外**: slice 列。先頭 byte = slice flags(bit0 = type id が文字列 / bit1 = index /
  bit4 = slice size あり / bit5 = 最終 slice)。Ice 3.7 の既定 = compact format(type id は最派生
  slice にだけ、size 無し)。CmdException は継承なし = slice 1 つ: flags, `"::mdaq::utl::CmdException"`,
  reason(string)。sliced format(flags 0x32 = 50)で来ても size で読めるよう両対応にする。
- **基本型**: int32/int64 LE、bool/byte 1 B、string = size + UTF-8(size は 1 B、255 以上は
  `0xFF` + int32)、seq/dict = size + 要素、enum(1.1)= size 形式(enumerator 数 < 255 なら 1 B)、
  struct = メンバ順。`Status` = enum State(7 値)+ enum Error(10 値)= 2 B。
- **切断**: type 4 受信 / EOF / 送信失敗 → 接続を捨てて次回張り直す。Ice 3.7 サーバの ACM
  (既定 timeout 60 s)でアイドル接続が閉じられうるので、**「まだ書き出していないリクエストだけ
  再送、書き出した後の切断は失敗として返す」**(C++ Ice の再試行規則と同じ。describe 等は非冪等)。

### 2.3 既製 crate の評価

| 候補 | 評価 |
|---|---|
| ice-rs 0.3.0(github cemoktra/ice-rs) | **不採用**。2023-02-14 にアーカイブ(保守なし)、**GPL-2.0**(将来の公開リポと相性が悪い)、Ice 3.7 想定、encoding 版数・例外の対応が文書化されていない。読む分には参考可 |
| ice0 | Ice「風」の別 RPC(互換ではない)→ 対象外 |
| 自前(部分集合) | **採用**。ワイヤ仕様が小さく、オラクル(C++ Ice 3.8 クライアントの実バイト列 / fake_ecc / 実 ECC 3.7)が全部手元にある |

### 2.4 受け入れ資産(既にあるもの)

- C++: `fake_ecc`(Ice servant、043 の error 規則込み)、`run_ecc_e2e.sh` + `ecc_e2e_client`。
- Rust: `TPCDAQ_ECC_BRIDGE_BIN` を見る統合テスト **6 ファイル 26 本**(controller_ecc 2 /
  controller_integration 12 / p3_error_paths 2 / soak_smoke 3 / ecc_bridge_intake 2 / p3_e2e 5)+
  soak_harness。**env を Rust bin に向けるだけで全部が受け入れテストになる**。
- 実 ECC: macOS keg(3.7.11、reference/_spike/prefix、`build_all.sh`)と GDED ネイティブ(3.7.2)。
  仮想 zCoBo デモ(041、`start_demo.sh`)= 実 ECC 歩き戻し(breakup → reset → reset)の再現。

## 3. 立案

### Phase 0 — ゴールデン採取(スパイク、0.5 日、Sonnet 可)

1. TCP タップ(Python asyncio 数十行、または Rust example)で **fake_ecc ↔ C++ ecc_bridge** の全 9 op
   のリクエスト/リプライ生バイトを採取 → `tests/fixtures/ice/<op>_{req,rep}.bin` + README(採取手順、
   requestId の値)。合成データなのでコミット可。configure は **255 B 超の DataLinkSet XML**
   (2 リンク)で採る(0xFF + int32 の string size 形式を含める)。
2. **実 ECC(macOS keg)相手にも採取**: 特に `CmdException` のリプライ(不正 config_id の describe で
   出る)と `getStatus` の Reply。fake_ecc は Ice 3.8 の出力なので、3.7 サーバの実物を必ず持つ。
3. 成果: 注釈付き hexdump(SPEC §8.2 付録候補)。`ice_ping` の mode(0 か 2 か)もここで確定。

### Phase 1 — `src/ice/`(codec + protocol + client、Opus、1 日)

- `codec.rs`: 1.1 の OutputStream/InputStream — byte / bool / int32 / int64 / string / seq<string> /
  dict<string,string> / enum(size 形式)/ encapsulation の begin/end(size は末尾で埋め戻し)。
- `proto.rs`: `MessageHeader`、`Request`、`Reply`(ReplyStatus 8 種を型に)、ValidateConnection /
  CloseConnection の parse/build。壊れ入力(短い / magic 違い / size 矛盾 / 未知 type)は `Err`。
- `client.rs`: `IceConnection`(**同期 `std::net::TcpStream`** — bridge は逐次 REP なので tokio は
  不要、KISS)。connect → ValidateConnection 待ち(タイムアウト付き)→
  `invoke(identity, op, mode, params) -> Result<Vec<u8> /* out encapsulation */, IceError>`。
  requestId 連番、受信ループで type 3 は捨て、type 4 / EOF で `Lost`。再接続規則は §2.2。
- `error.rs`: `IceError { Local(Connect | Lost | Protocol(String)), UserException { type_id, body },
  RequestFailed { kind, identity, facet, operation }, Unknown { kind, message } }`。
- テスト: **手書き期待バイト**の単体(仕様から起こす)を主、Phase 0 のゴールデンとのバイト一致を従。
  例外 compact/sliced 両形式、string 255 境界、ValidateConnection 割り込み、途中切断。
- 規約: `.unwrap()` 禁止、`unsafe` 無し、per-request の heap は気にしない(制御面、Hz 級)。

### Phase 2 — `Ecc` 型付きプロキシ + bridge bin(Opus、1 日)

- `src/ecc/proxy.rs`: `EccProxy`(describe / prepare / configure / start / stop / breakup / reset /
  get_status / ice_ping)。`State`(7)/ `Error`(10)は **GetEcc.ice の順序を定数表で固定**
  (§2.5 スキーマ漂流ガードと同じ流儀 — 表と enum の突き合わせテスト)。CmdException → `reason`。
- `src/ecc/core.rs`: ecc_core.hpp の 1:1 移植(JSON 契約 §8.2、DataLinkSet XML、next_state)。
  **test_ecc_bridge.cpp の 457 CHECK を Rust テストに移植**(件数不変 = 仕様の輸送)。
- `src/bin/ecc_bridge.rs`: ZMQ REP(既存 zmq_helper)+ 上記。CLI 互換 `--bind` / `--ecc-proxy
  "Ecc:tcp -h H -p P"`(proxy 文字列パーサ: identity / `tcp` / `-h` / `-p`、`-e` は無視)、stdout の
  `BIND ...` 1 行など run_ecc_e2e.sh・Rust テストの期待を維持。LocalException 相当で proxy を捨てて
  次回再接続(C++ と同じ)。
- 受け入れ: ①run_ecc_e2e.sh を Rust bin で green ②§2.4 の Rust 統合テスト 26 本を env 差し替えで
  green ③実 ECC 歩き(041 デモ: describe → … → stop → breakup → reset → reset、audit の
  `ecc_walk_back` が C++ 時と同一)④034 の連続 run(ACM アイドル切断 → 再接続の実証)。

### Phase 3 — 切替と撤去(0.5 日)

- SPEC §1.1 コンポーネント表 / §8.2(「ecc-bridge(C++)」→ Rust、Ice 3.8 の記述を落とす)/
  CLAUDE.md(Tech Stack の Ice 行)/ docs/VIRTUAL_ZCOBO_ja.md §4.1 / 069 の起動手順 / メモリ。
- tools/ecc_bridge から ecc_bridge.cpp・ecc_core.hpp・test_ecc_bridge.cpp・json_min.hpp・
  ecc_e2e_client.cpp を削除。**fake_ecc.cpp は残す**(Ice servant はテストダブルとして必要。
  3.7 でもビルドできるか要確認 — 3.8 固有 API なら `#if` で分岐)。Makefile 縮小。
- GDED: bridge を GDED ネイティブ(Rust バイナリのみ)に → SSH トンネル(-L 46002 / -R 47200)不要。
  069 の残項目(#7 連続 ×10)をこの構成で実施。

## 4. 見積もり

| 部分 | 行数目安 |
|---|---|
| codec / proto / client / error | ≈ 900 |
| ecc proxy + core 移植 + bridge bin | ≈ 600 |
| テスト(457 CHECK 移植 + codec 単体 + ゴールデン) | ≈ 800 |

工数: Phase 0 0.5 日(Sonnet)/ Phase 1 1 日(Opus)/ Phase 2 1 日(Opus)/ Phase 3 0.5 日、
Fable レビューはフェーズ境界 2 回(Phase 1 後 = ワイヤ、Phase 2 後 = 実 ECC)。Phase 0 と Phase 1 の
手書き単体は独立なので並列可。

## 5. リスク・未確定

1. **1.1 例外の実バイト** — compact / sliced 両対応で吸収。Phase 0 で実 ECC(3.7)の例外を必ず採る。
2. **ACM の挙動** — Ice 3.7 サーバ既定(timeout 60 s)でアイドル後の次リクエストが切断を踏む。dispatch 中
   (configure 261 s)は idle ではないので閉じられない。再接続で吸収し、034 連続 run で実証。
3. **`ice_ping` の mode / context の有無** — ゴールデンで確定(仕様上は Idempotent 相当)。
4. **string ≥ 255 B** — DataLinkSet XML で必ず踏む → 単体 + ゴールデン両方に置く。
5. **ハートビートの割り込み** — リプライ待ち中の type 3 を読み飛ばす(requestId 照合も必須)。
6. **fake_ecc の 3.7 互換** — Phase 3 の前提。3.8 固有 API があれば分岐。
7. **ライセンス** — GET の .ice を読んで自前で書くのはプロトコル互換実装であり CeCILL コードの取り込み
   ではない(third_party 隔離不要)。op 名 / enum 名は API 事実。ice-rs(GPL)のコードは持ち込まない。

## 6. ユーザーに仰ぐ判断

1. **優先度**: 069 残 / 新 DAQ 機セットアップとの前後。**推奨 = 新 DAQ 機の初期設定の前**
   (セットアップ手順から「Ice 3.8 を別途ビルド」が消える。ECC 用の 3.7 だけになる)。
2. 同期実装(`std::net`)で良いか(controller は tokio だが bridge は別プロセスの逐次 REP — KISS)。
3. C++ bridge の撤去タイミング(実機で 1 回通してから、を推奨)。

## 参照

- Ice 3.7 Protocol and Encoding: https://archive.zeroc.com/ice/3.7/ice-protocol-and-encoding
  (Protocol Messages / Ice Encoding / Data Encoding for Exceptions / Protocol and Encoding Versions)
- ice-rs(不採用の根拠): https://github.com/cemoktra/ice-rs / https://crates.io/crates/ice-rs
- 一次資料(リポ内、reference/20190315_patched): GetBench/src/get/GetEcc.ice、MDaq/src/mdaq/Ecc.ice、
  MDaq/src/mdaq/utl/CmdException.ice、MDaq/src/mdaq/hw/Control.ice
- 現行実装: tools/ecc_bridge/{ecc_bridge.cpp, ecc_core.hpp, fake_ecc.cpp}、SPEC §8.2、
  docs/VIRTUAL_ZCOBO_ja.md §4.1/§4.2、メモリ geteccserver-native-gded
