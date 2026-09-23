# 076 — ecc-bridge の Rust 化(Ice クライアント部分集合の自前実装)

**Status: READY**(調査完了 + 立案 2026-09-14、Fable。**同日ユーザー裁定で §6 の 3 点が確定** —
着手指示があれば Phase 0(Sonnet)と Phase 1(Opus)を並列で出す。
発端 = ユーザー質問「zCoBo を変えない限り Ice の版数問題からは逃れられないか」→ 3 段の現実解のうち
「2. ecc_bridge だけを Rust Ice クライアントに置換」の詳細調査依頼)

**ユーザー裁定(2026-09-14)**:
1. **優先度 = 新 DAQ 機で最初に試す 2 項目のうちの 1 つ**(「ROOT のインストール」と「本ユニットの
   Rust bridge」)。→ 新 DAQ 機の初期設定より**前に**実装・宅内受け入れを済ませておく。
2. **同期(シリアル)実装で可** — `std::net::TcpStream`、tokio 不要。
3. **C++ bridge の撤去(Phase 3)は Rust bridge が実機でちゃんと動くと分かってから**。それまで
   tools/ecc_bridge の C++ 一式は残す(fake_ecc は撤去後も残す)。

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

### Phase 2 発注メモ(2026-09-22、Opus)

- 着手前に worktree で `git merge --ff-only main`(ff できなければ固有コミットが無いことを確認して `git reset --hard main`)。発注書 = 本節 +
  上の Phase 2 節 + Phase 1「結果」節の公開 API。本体の絶対パス `/Users/aogaki/Workspace/tpcdaq-rs/TODO/076_rust_ice_client.md` で読んでよい。
- reference/ は worktree に無い。`.ice` は本体の絶対パスで読む: `/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched/GetBench/src/get/GetEcc.ice`
  (`State` 7 / `Error` 10 の**順序が値**)、`.../MDaq/src/mdaq/Ecc.ice`、CmdException の型 id は `tests/fixtures/ice/README.md`(Phase 0)が正。
- 移植元(リポ内): `tools/ecc_bridge/ecc_core.hpp`(JSON 契約・DataLinkSet XML・next_state)、`ecc_bridge.cpp`(CLI・REP ループ・再接続)、
  `test_ecc_bridge.cpp`(**457 CHECK → Rust テストへ 1:1**、件数を結果節に書く)。C++ 側は**削除しない**(Phase 3)。
- 配置: `src/ecc/{mod,proxy,core}.rs` + `src/bin/ecc_bridge.rs`(Cargo の bin 名 `ecc_bridge`。C++ 実行ファイルは tools/ 配下で別物)。
- 受け入れは ①② のみ(③④ は実 ECC が要る = 新 DAQ 機。結果節に「未実施」と明記):
  ① `tools/ecc_bridge/run_ecc_e2e.sh` に環境変数 `ECC_BRIDGE_BIN`(既定 `./ecc_bridge`)を足し、Rust bin を指して green。fake_ecc / ecc_e2e_client は
    ビルド済み(Mac、Ice 3.8)。無ければ `make TPCDAQ_ICE_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched -j`。
  ② `TPCDAQ_ECC_BRIDGE_BIN` を Rust bin に向けて `tests/{ecc_bridge_intake,controller_ecc,p3_e2e,p3_error_paths,soak_smoke}.rs` が green
    (C++ bin を指しても従来どおり green)。
- 再接続: LocalException 相当(Connect / Lost / Protocol)で接続を捨て、**次の呼び出しで**再接続(C++ と同じ)。再送はしない(非冪等 op がある)。
  リプライ待ち中の validate(type 3)/ close(type 4)は Phase 1 の client のとおり。
- 設計判断が残ったら実装せず報告(例: Ice `Error` enum → CmdException `reason` の写像 — `ecc_core.hpp` を正とする)。

### Phase 3 — 切替と撤去(0.5 日、**実機で Rust bridge が動いてから** — ユーザー裁定 3)

- 前提 = 新 DAQ 機(または GDED)で実 ECC + 実 zCoBo に対し Rust bridge で run 1 周(describe → … →
  stop → breakup → reset → reset)が通り、audit の `ecc_walk_back` が C++ 時と同一であること。
  それまで C++ bridge は残し、`TPCDAQ_ECC_BRIDGE_BIN` で切替可能な状態を保つ。
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

## 6. ユーザーに仰いだ判断(2026-09-14 裁定済み — 冒頭の「ユーザー裁定」が正)

1. ~~優先度~~ → **新 DAQ 機で最初に試す項目**(ROOT インストールと並ぶ)。宅内で先に仕上げる。
2. ~~同期実装で良いか~~ → **同期(シリアル)で可**。
3. ~~C++ 撤去のタイミング~~ → **Rust bridge の実機動作確認後**。

新 DAQ 機の初期設定順(この裁定から導かれる形): Ubuntu 24.04 → Rust toolchain → **ROOT**(root_sink 用)
→ **ECC = GET 20190315_patched をホストの Ice 3.7.x でソースビルド**(build_all.sh の Linux 版)→
**Rust bridge(本ユニット)で実 ECC 歩き** → tpcdaq-rs フルスタック。Ice 3.8 は登場しない。

## 参照

- Ice 3.7 Protocol and Encoding: https://archive.zeroc.com/ice/3.7/ice-protocol-and-encoding
  (Protocol Messages / Ice Encoding / Data Encoding for Exceptions / Protocol and Encoding Versions)
- ice-rs(不採用の根拠): https://github.com/cemoktra/ice-rs / https://crates.io/crates/ice-rs
- 一次資料(リポ内、reference/20190315_patched): GetBench/src/get/GetEcc.ice、MDaq/src/mdaq/Ecc.ice、
  MDaq/src/mdaq/utl/CmdException.ice、MDaq/src/mdaq/hw/Control.ice
- 現行実装: tools/ecc_bridge/{ecc_bridge.cpp, ecc_core.hpp, fake_ecc.cpp}、SPEC §8.2、
  docs/VIRTUAL_ZCOBO_ja.md §4.1/§4.2、メモリ geteccserver-native-gded

## 結果(Phase 0 — ゴールデン採取、2026-09-17、Sonnet レーン → main `cad5a4c`)

- `tests/fixtures/ice/`: 9 op × {req,rep}(fake_ecc、Ice 3.8.2)+ `validate_connection.bin` / `close_connection.bin` +
  実 ECC(3.7.11、reference/_spike ビルド)の `real_ecc_{ice_ping,describe_cmdexception,getstatus}_{req,rep}.bin` +
  `real_ecc_validate_connection.bin`。README に採取手順・requestId・注釈付き hexdump。タップ = `tools/ecc_bridge/tap_ice.py`
  (asyncio、`annotate` サブコマンド)+ `tap_drive_{fake,real}.py`(pyzmq)。既存 C++ 無変更、`make test` 457 / e2e 52 green。
- 確定事実: **`ice_ping` = mode 2(Idempotent)、context 空**(3.7/3.8 同一)/ StateMachine 8 op = mode 0 / configure の
  XML 283 B で 0xFF+int32 形式を踏む / ValidateConnection は最初の要求前に必ず来る / CloseConnection はクライアント側
  `Communicator::destroy()` で 1 回観測(サーバ発は未観測)/ **CmdException = status 1、単一 compact スライス、flags 0x20
  (IS_LAST のみ)+ type-id 文字列 `::mdaq::utl::CmdException` + reason 文字列**(Ice は例外スライスの type-id を常に
  文字列で書く — フラグビットは class 用。Fable 裁定)/ 実 ECC の getStatus out = `01 01` = {Idle, WhenDescribe}。
- §5 リスク 1(1.1 例外の実バイト)・3(ice_ping の mode)はこれで解消。

## 結果(Phase 1 — `src/ice/`、2026-09-17、Opus レーン → main `9fe6c2f`)

- 実装: `src/ice/{mod,error,codec,proto,client}.rs`(実装 ≈1,140 行 + 同ファイル内テスト ≈1,040 行)+ `tests/ice_golden.rs`(363 行)。
  同期 `std::net::TcpStream`、依存追加なし、`unsafe` 無し、production に `.unwrap()` 無し。`MAX_MESSAGE_SIZE` 16 MiB の安全弁、
  `set_nodelay(true)`、ヘッダ検査は Ice 本体と同じ厳しさ(protocol 1.0 / ヘッダ encoding 1.0 / 非圧縮のみ)。
- テスト: **cargo test(main)514 → 578 passed / 0 failed / 1 ignored**(ice 単体 55 + ゴールデン 9)。fmt / clippy clean。
  TDD: proto 23 / client 12 は `unimplemented!()` で赤を実測してから緑、codec は変異(`< 255` → `<= 255`)で噛み合いを実証。
- **手書きの期待バイトは実機と全一致**: `tests/ice_golden.rs` は初回で 9 本とも緑。12 リクエスト(9 op + 実 ECC 3 本)を
  `Request::to_bytes` で完全再現、11 リプライを parse/往復、実 ECC の CmdException(115 B)も `Reply::parse → to_bytes` で
  バイト往復。変異(ice_ping を Normal に)で落ちることも確認。
- 実機に合わせた修正 1 箇所: compact 例外の builder flags `0x21 → 0x20`(decoder は元から「type-id ビットを見ず常に文字列」で正)。
- 公開 API(Phase 2 が使う): `IceConnection::connect(SocketAddr, Duration)` / `invoke(&Identity, op, OperationMode, params) ->
  Result<Vec<u8>, IceError>`(params・戻りとも encapsulation の中身)、`OutputStream` / `InputStream`、
  `IceError { Local(Connect|Lost|Protocol(String)), UserException{type_id, body}, RequestFailed{kind, identity, facet, operation},
  Unknown{kind, message} }`。
- 承認した逸脱: LocalError に原因文字列 / ReplyStatus 再利用 / invoke の戻りは中身 / connect は SocketAddr / 再接続・再送は
  **Phase 2 のプロキシの責務**(CloseConnection 受信 = 未 dispatch の旨をエラー文字列に残す)。
- Phase 2 へ: State(7)/ Error(10)の値表(ゴールデンは `02 00` = Described/NoErr、`01 01` = Idle/WhenDescribe の 2 点)、
  サーバ発 CloseConnection(ACM)は未観測 → 034 連続 run で実証、sliced 例外の実バイトは未採取(読み飛ばしは実装済み)。

## 結果(Phase 2 — `Ecc` 型付きプロキシ + ecc_core 移植 + `ecc_bridge` bin、2026-09-22、Opus レーン)

- **実装**: `src/ecc/{mod,core,proxy}.rs` + `src/bin/ecc_bridge.rs`(実装 ≈1,225 行 + 同ファイル内テスト ≈1,200 行)。
  依存追加なし(zmq / serde_json は既存)、`unsafe` 無し、production に `.unwrap()` 無し、tokio 不使用(同期 REP 逐次)。
  `tools/ecc_bridge/run_ecc_e2e.sh` に `ECC_BRIDGE_BIN`(既定 `./ecc_bridge`)を追加。
  **C++ 一式(ecc_bridge.cpp / ecc_core.hpp / test_ecc_bridge.cpp / json_min.hpp / fake_ecc.cpp / ecc_e2e_client.cpp)は
  無変更・無削除**(撤去は Phase 3 = 実機確認後)。

- **実行コマンド / テスト数(before → after)**:
  - `cargo test` … **614 → 652 passed / 0 failed / 1 ignored**(+38 = core 18 + proxy 13 + bin 7)。
  - `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo clippy --tests -- -D warnings` … いずれも clean。
  - 環境: macOS 26.6.2(Darwin 25.6.0)、rustc 1.97.1、2026-09-22。

- **TDD の赤を実測**: 実装本体を `unimplemented!("TDD red")` に置いてからテストを回した。
  `cargo test --lib ecc::` = **30 failed / 1 passed**(唯一通ったのは serde_json だけを触る
  `the_json_substrate_behaves_like_json_min`)、`cargo test --bin ecc_bridge` = **7 failed / 0 passed**。
  実装を戻して全緑(31 / 7)。

- **457 CHECK の移植 = 457 件**(`tools/ecc_bridge/test_ecc_bridge.cpp` の green 経路で実行される CHECK 数と同数。
  実測 `./test_ecc_bridge ../../tests/fixtures/ecc_transitions.txt` → `457 passed, 0 failed`):

  | C++ テスト | CHECK | Rust テスト(`src/ecc/core.rs`) | assert |
  |---|---:|---|---:|
  | data_link_set_single_cobo / two_cobo / escapes_attributes | 1+1+1 | 同名 | 1+1+1 |
  | data_link_set_round_trip | 13 | `data_link_set_round_trip` | 13 |
  | parse_configure_request / simple_actions / rejects_bad_input / defaults_flow_type | 12+7+22+3 | 同名 | 12+7+22+3 |
  | make_response / …carries_ecc_error_beside_state | 2+2 | `make_response_has_the_four_keys` / 同名 | 2+2 |
  | error_strings / state_strings | 11+18 | 同名 | 11+18 |
  | state_machine_happy_path / reset_walks_back… / configure_is_silently_skipped… / rejects_out_of_order | 18+31+20+11 | 同名 | 18+31+20+11 |
  | golden_transition_table(64 行 × 4 + 行数) | 257 | 同名(同じ `tests/fixtures/ecc_transitions.txt` を読む) | 257 |
  | json_min(自前 JSON 273 行の土台) | 27 | `the_json_substrate_behaves_like_json_min`(土台 = serde_json) | 27 |
  | **計** | **457** | | **457** |

  加えて golden 表の**形式検査** 3 assert/行 × 64 = 192(C++ では失敗時のみ実行される診断 `CHECK(false)` に対応)。
  `-2.5e1` だけ C++ `as_int()` = `-25` に対し serde_json は f64 のままなので `as_f64() == -25.0` で照合(値は同じ)。

- **変異(mutation)で噛み合いを実証** — 6 件すべて**期待どおりのテストだけ**が赤(他は緑のまま):

  | 変異 | 赤になったテスト |
  |---|---|
  | `xml_escape` が `"` を素通し | data_link_set_escapes_attributes / …round_trip |
  | `next_state`: Ready からの reset が Idle へ(**034 の事故**) | reset_walks_back… / golden_transition_table / rejects_out_of_order |
  | `Error::NoErr` の綴りを Ice 側(`NoErr`)に寄せる | error_strings / make_response… / wire_tables… |
  | `parse_request` が小文字 `tcp` を黙って直す | parse_rejects_bad_input |
  | `WIRE_STATES` の Idle と Described を入れ替え | wire_tables… / get_status_decodes_the_two_enums |
  | `ice_ping` を Normal(mode 0)で打つ | connecting_pings_the_servant_like_the_cpp_bridge |

- **受け入れ ①(`run_ecc_e2e.sh` を Rust bin で)**: green。
  `ECC_BRIDGE_BIN=<worktree>/target/debug/ecc_bridge ./run_ecc_e2e.sh` → **`ecc_e2e: 52 passed, 0 failed`**
  (C++ bin での同じ実行も 52 passed = 回帰なし)。fake_ecc / ecc_e2e_client は C++(Ice 3.8.2)のまま。
  **ブリッジの stderr ログは C++ 版と 1 行ずつ同一**(状態遷移・`ecc_error` の set/clear・
  `Could not establish data link.` の本文まで)。唯一の差は壊れた JSON の理由文字列
  (`parse error at offset 10: unexpected end of input` → `parse error: EOF while parsing a value at line 1 column 10`)
  —— json_min → serde_json の言い回しの差で、契約(`ok=false` + 理由が載る)は同じ。

- **受け入れ ②(`TPCDAQ_ECC_BRIDGE_BIN` を Rust bin に向けた既存 Rust 統合テスト)**: green。
  `tests/{ecc_bridge_intake, controller_ecc, p3_error_paths, p3_e2e, soak_smoke}.rs` = **2 / 2 / 2 / 5 / 3 = 14 本 green**
  (`TPCDAQ_ROOT_SINK_BIN` / `TPCDAQ_REAL_GRAW` / `TPCDAQ_REAL_GEOMETRY_MINI` も実物を渡した実走)。
  C++ bin を指した同じ 4 本(intake + controller_ecc)も従来どおり green。
  **`p3_e2e::e2e_h_consecutive_runs_need_no_manual_ecc_reset`(034 の歩き戻し)が Rust bridge で通る**のがこの段の要。
  なお §2.4 の「6 ファイル 26 本」のうち `controller_integration.rs` は `MockTransport` 経由で bin を起動しない
  (env を読んでいない)ため、実際に env で切り替わるのは上記 5 ファイル 14 本。

- **承認が必要な逸脱**(Fable レビュー待ち。いずれも SPEC §8.2 の JSON 契約と CLI は不変):
  1. `parse_request` の C++ `Request{valid,error,…}` → **`Result<Request, String>`**(`valid` = `is_ok`)。
     `next_state` の out 引数 → `Transition{step,next,error}` を返す形。意味論は 1:1。
  2. **`--ecc-proxy` の文字列を bind の前に検査して不正なら exit 2**(C++ は Ice の ParseException を
     `Result::fail` にして走り続け、全 action に `ok=false` を返していた)。誰も REQ を待っていない時点で落ちる方が安全、
     という判断。`category/name` 形式の identity も**拒否**する(黙って別オブジェクトに話しかけない)。
  3. **プロキシのログは bin 側の `eprintln!`**(C++ と同じ体裁)。`tracing` を初期化していないので
     `src/ice` の debug ログ(ハートビート)は出ない。stdout は `BIND …` 1 行のみ(契約)。
  4. **SIGINT/SIGTERM はハンドラを置かず既定動作で終了**(C++ は 200 ms poll + フラグ)。
     観測できる違いは「シャットダウン 1 行が出ないことがある」だけで、テストは SIGKILL / `kill -TERM` を使う。
  5. `getStatus` の out が **2 enum ちょうどでない**場合は `Err`(「読めた分だけ」で通さない)。
     C++ は Ice の生成コードが同じ検査をしていた位置。
  6. `Observed`(getStatus 失敗)の理由を **stderr に 1 行残す**(C++ `observe()` は捨てていた)。
     応答の `error` は従来どおり直前の操作のものが正。
  7. **connect タイムアウト = 10 s の定数**(`proxy::CONNECT_TIMEOUT`)。C++ は Ice 既定。
     controller の 60 s/REQ(SPEC §8.2 v1.20)より短いことだけが要件。CLI フラグは増やしていない。

- **未実施**(実 ECC が要る = 新 DAQ 機 / GDED。本レーンでは**触っていない**):
  - **③ 実 ECC 歩き**(041 デモ: describe → … → stop → breakup → reset → reset、audit の `ecc_walk_back` 同一性)。
  - **④ 034 の連続 run**(ACM アイドル切断 → 再接続の実証)。再接続規則は実装済み
    (`a_lost_connection_is_dropped_and_the_next_call_reconnects` でサーバ発切断は再現済み)だが、
    **実 ECC の ACM による切断そのものは未観測**(Phase 1 から持ち越し)。
  - sliced format の例外実バイトは未採取(読み飛ばしは実装済み)。
  - Phase 3(C++ 撤去・SPEC/CLAUDE.md 更新・GDED へのネイティブ配置)は**着手していない**。

### Fable レビュー(Phase 2、2026-09-22、マージ `worktree-agent-a446ecc3e5e8faec7` → main)
- 自分でも再実行: `cargo test` **652 passed / 0 failed / 1 ignored**(614 + 38)、fmt / clippy clean、`ECC_BRIDGE_BIN=<Rust bin> run_ecc_e2e.sh` → **52 passed, 0 failed**。
- 逸脱 1–7 は**すべて承認**(Result/Transition の型 = 意味論 1:1、`--ecc-proxy` 不正は起動失敗 = SPEC §3.2「半端な既定値で走らない」と同じ流儀、
  getStatus の厳格化と失敗理由のログは silent 禁止に合う、SIGINT 既定動作、connect 10 s)。
- **Phase 3 への持ち越し 2 点**: ①bin のログを他の Rust bin と同じ `tracing_subscriber`(env-filter)初期化にし、`src/ice` の debug が `RUST_LOG` で出るようにする(いまは `eprintln!`)。
  ②`controller_integration.rs` は MockTransport 経由で bin を使わない(§2.4 の「26 本」は実際には 5 ファイル 14 本 + mock 12 本)— 記述を訂正済みとして扱う。
- Phase 3 の前提(実 ECC + 実 zCoBo で Rust bridge の run 1 周、audit `ecc_walk_back` が C++ と同一)は新 DAQ 機で。それまで `TPCDAQ_ECC_BRIDGE_BIN` で切替可能な状態を維持。

## 結果(Phase 3 前提 — 実機確認、2026-09-23、GDED 遠隔、Fable)
- 構成: GDED(daq@192.168.144.101)で **実 getEccServer 20190315 をホストにネイティブ起動**(`-a 127.0.0.1:46002`、`--config-repo-url
  /home/daq/ELITPC/GetSoftware_config`、LD_LIBRARY_PATH = vm_ice : GET lib : stage(xenial deb 展開))+ **Rust `ecc_bridge`**(`target/release/ecc_bridge
  --bind tcp://*:47200 --ecc-proxy "Ecc:tcp -h 127.0.0.1 -p 46002"`、GDED 上でネイティブ、**SSH トンネル不要**)+ bench スタック(`bench_up.sh`:
  null_sink / graw_writer / decoder / receiver0 / controller)。VM の GET は全停止(ユーザー)。実 zCoBo 192.168.4.81、AsAd 内蔵パルサー
  (`getEccClient` で `asad-pulser-start -1000 1000`)。
- ブリッジ log: `connected to Ecc:tcp -h 127.0.0.1 -p 46002 (encoding 1.1)` → describe / prepare / configure / start / stop / breakup / reset / reset
  すべて `ok=1`、`ecc_error=NO_ERR`(Ice 3.7.2 サーバとの実機互換を初確認)。
- run 3(60 s): run/start **7.41 s**、`asad-pulser-start` 後 ~1 Hz、**60 frames / 16,727,040 B(= 60 × 278,784)**、malformed 0 / overflow 0 /
  extra_connections 0 / topology_frames 0、run/stop 3.66 s、`forced_eos:true / eos_closed:true`。
- run 4(30 s): audit `ecc_walk_back = ["status->Ready","breakup->Prepared","reset->Described","reset->Idle"]` = **C++ ブリッジ時(069 結果 #6)と同一**。
  run/start 7.98 s、30 frames / 8,363,520 B。
- → **Phase 3 の前提(実 ECC + 実 zCoBo で Rust bridge の run 1 周、walk-back 一致)は満たした。** C++ 撤去(Phase 3 本体)は次の起票で。
- 残懸念: bin のログは `eprintln!`(tracing 未初期化、Phase 2 レビューの持ち越し①)。ACM のアイドル切断は 2 run の間隔 20 s では未観測。

## Phase 3 発注(C++ ecc_bridge の撤去 — Sonnet、2026-09-23。前提 = 上の「Phase 3 前提 — 実機確認」節で充足、ユーザー裁定 3)

着手前に worktree で `git merge --ff-only main`。発注書は本体の絶対パスで読んでよい。reference/ は worktree に無い(本票では不要)。

### 消すもの / 残すもの(tools/ecc_bridge)
- **削除**: `ecc_bridge.cpp`、`ecc_core.hpp`、`test_ecc_bridge.cpp`、`json_min.hpp`、`check.hpp`(test_ecc_bridge 専用なら)、ビルド生成物(`ecc_bridge`、`test_ecc_bridge`、`*.dSYM`)。
- **残す**: `fake_ecc.cpp`(Ice servant のテストダブル)、`ecc_e2e_client.cpp`(Rust bin の e2e 相手)、`run_ecc_e2e.sh`(既定を Rust bin に:
  `ECC_BRIDGE_BIN` の既定 = `<repo>/target/release/ecc_bridge` があればそれ、無ければ `target/debug/ecc_bridge`、無ければエラー文言)、`tap_*.py`、`slice/`。
  `Makefile` は fake_ecc / ecc_e2e_client だけを作る形に縮める(`TPCDAQ_ICE_DIR` は残る)。
- `tests/*.rs` の `TPCDAQ_ECC_BRIDGE_BIN` は残す(既定を Rust bin に倒す: env 無しなら `env!("CARGO_BIN_EXE_ecc_bridge")` を使う。`TPCDAQ_FAKE_ECC_BIN` は従来どおり)。
  → **env 無しでも統合テストが Rust bin で走る**(skip しない)。fake_ecc が無いときだけ skip(理由を出力)。
- `reference/_spike/demo/start_demo.sh`: `ECC_BRIDGE` を Rust bin に。`docs/VIRTUAL_ZCOBO_ja.md` §4.1(Ice 3.8 の記述を「ブリッジは Rust、Ice ライブラリ不要」に)、
  `docs/NEW_DAQ_SETUP_ja.md` §7(C++ 併存の段落を削除)、`README.md` の該当行。**SPEC §1.1 / §8.2 と CLAUDE.md の Tech Stack 行は Fable が直す(触らない)。**
- `src/bin/ecc_bridge.rs`: Phase 2 レビューの持ち越し①を同時に消化 — 他の Rust bin と同じ `tracing_subscriber`(env-filter)初期化にし、
  `eprintln!` を `tracing` に置き換える(stdout の `BIND …` 1 行は不変)。

### テスト
- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(現 838。env 無しで `ecc_bridge_intake` 等が skip でなく走ること)。
- `make -C tools/ecc_bridge -j TPCDAQ_ICE_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched` が fake_ecc / ecc_e2e_client を作り、
  `run_ecc_e2e.sh` が Rust bin で **52 passed**。`bash reference/_spike/demo/start_demo.sh` → `run_once.sh 12` → `stop_demo.sh` が通る(Mac、ユーザーの
  demo が動いていないときに)。
- 結果節: 削除した行数、テスト数、e2e 出力、demo の run 1 本の結果、承認が要る逸脱。

## 結果(Phase 3 — C++ 撤去、2026-09-23、Sonnet レーン)

- **前提**: worktree で `git merge --ff-only main` → `05e2f90` 着手前に一致。着手前 `cargo test`
  **838 passed / 0 failed / 1 ignored**(発注書どおり)。

- **削除**: `tools/ecc_bridge/ecc_bridge.cpp`(361 行)、`tools/ecc_bridge/test_ecc_bridge.cpp`
  (558 行)= **計 919 行**。`git diff --stat HEAD` = 15 files changed, 160 insertions(+), 1028
  deletions(-)(残り 109 行は Makefile/README/docs/tests の整理)。

- **残した(発注書の削除リストと 1 点だけ食い違う — 下記「承認が要る逸脱 1」)**:
  `fake_ecc.cpp`(Ice servant のテストダブル)、`ecc_e2e_client.cpp`(Rust bin の e2e 相手)、
  `run_ecc_e2e.sh`(既定を Rust bin に付け替え)、`tap_*.py`、`slice/` 生成の仕組み。
  `Makefile` は fake_ecc / ecc_e2e_client の 2 つだけを作る形に縮めた
  (`all: fake_ecc ecc_e2e_client`、`ecc_bridge` / `test_ecc_bridge` / `test`(ECC_TRANSITIONS 経由の
  golden 表照合)ターゲットを削除)。`.gitignore` から `ecc_bridge` / `test_ecc_bridge` の
  ビルド成果物エントリも削った。

- **`run_ecc_e2e.sh` の既定ブリッジ**: `ECC_BRIDGE_BIN` 未指定なら
  `<repo>/target/release/ecc_bridge` → 無ければ `<repo>/target/debug/ecc_bridge` → 無ければ
  「先に `cargo build --release --bin ecc_bridge` すること」を stderr に出して exit 1。

- **`tests/*.rs` の `TPCDAQ_ECC_BRIDGE_BIN`**: `tests/controller_ecc.rs` / `tests/ecc_bridge_intake.rs`
  の `bins()` と `tests/common/mod.rs` の `e2e_env()` を、env 未設定なら
  `env!("CARGO_BIN_EXE_ecc_bridge")` にフォールバックする形へ変更(`TPCDAQ_FAKE_ECC_BIN` だけが
  真の skip ゲート)。実測(env を一切設定せず `cargo test --test controller_ecc --test
  ecc_bridge_intake --test p3_error_paths --test p3_e2e --test soak_smoke`):
  - `controller_ecc` / `ecc_bridge_intake`: 「skip: TPCDAQ_FAKE_ECC_BIN が無い」に変わり(以前は
    `TPCDAQ_ECC_BRIDGE_BIN` も一緒に不足と出ていた)、**env-gated テスト自体は 2/2, 2/2 green**
    (fake_ecc 無しでも走る非 gated テストの分)。
  - `p3_error_paths` / `p3_e2e` / `soak_smoke`: skip 理由の一覧から `TPCDAQ_ECC_BRIDGE_BIN` が
    消え、`TPCDAQ_ROOT_SINK_BIN, TPCDAQ_FAKE_ECC_BIN, TPCDAQ_REAL_GRAW,
    TPCDAQ_REAL_GEOMETRY_MINI` の 4 本だけが残ることを確認(意図どおり)。
  各ファイルの module doc の起動例も `TPCDAQ_ECC_BRIDGE_BIN` を省いて更新。

- **`src/bin/ecc_bridge.rs` のログ**: `eprintln!` を全部 `tracing`(`error!`/`warn!`/`info!`)に
  置換。`Options` 解析エラー・`--ecc-proxy` 不正・ZMQ bind 失敗・getStatus 失敗・action ごとの
  結果・接続確立/切断・shutdown まで、意味を変えずフィールド化(例:
  `info!(action = %.., ok, state = %.., ecc_error = %.., error = %.., "action")`)。
  `println!("BIND {endpoint}")` はそのまま(契約)。

- **テスト**: `cargo test` **838 passed / 0 failed / 1 ignored**(着手前と同数 —
  Phase 3 はリファクタのみで新規テストは追加していない)。`cargo clippy --all-targets -- -D
  warnings` / `cargo clippy --tests -- -D warnings` / `cargo fmt`(diff なし)すべて clean。
  `cargo test --bin ecc_bridge` = 7 passed(既存の CLI/契約テストが tracing 移行後も無変更で green)。

- **e2e 出力**(実測、`make clean && make -j TPCDAQ_ICE_DIR=.../reference/20190315_patched` の
  クリーンビルドから):
  ```
  run_ecc_e2e: bridge = <repo>/target/release/ecc_bridge
  run_ecc_e2e: fake ECC proxy = Ecc:tcp -h 127.0.0.1 -p 59234
  run_ecc_e2e: bridge REP = tcp://127.0.0.1:59235
  ecc_e2e: 52 passed, 0 failed
  ```
  fake_ecc / ecc_bridge の stderr ログも全遷移一致(describe→prepare→configure→start→stop→
  breakup→reset→reset、`WHEN_START` の `Could not establish data link.` まで Phase 2 と同じ)。

- **demo(`reference/_spike/demo/`)**: **未実施**。理由 2 点:
  1. `ps` / `lsof -iTCP -sTCP:LISTEN` で確認 —— controller / monitor / vcobo_daq / ecc_bridge /
     fake_ecc のいずれも稼働しておらず、47xxx 系ポートの listen も無い(ポート衝突の懸念は無い)。
  2. にもかかわらず未実施なのは、**このセッションが worktree に隔離されており
     `reference/`(メインチェックアウトのみに存在、.gitignore 済みで worktree には無い)を書けない
     ため**(Edit ツールが `reference/_spike/demo/env_demo.sh` への書き込みを
     「worktree の外」として拒否するのを実測)。`git merge --ff-only` の前提説明どおり
     `reference/` は worktree に無いが、今回はその**書き込み側**にも同じ制約がかかった。
     メインチェックアウト側 `/Users/aogaki/Workspace/tpcdaq-rs/reference/_spike/demo/env_demo.sh`
     の 27 行目を次のように直し、`bash start_demo.sh` → `run_once.sh 12` → `stop_demo.sh` を
     確認するのは、worktree の外で行える人(施主 or メインセッション)に持ち越す:
     ```diff
     -ECC_BRIDGE=${TPCDAQ_ECC_BRIDGE_BIN:-$REPO/tools/ecc_bridge/ecc_bridge}
     +ECC_BRIDGE=${TPCDAQ_ECC_BRIDGE_BIN:-$BIN/ecc_bridge}
     ```
     (`$BIN` はその 3 行上で `$REPO/target/release` として既に定義済み。`start_demo.sh` 自体は
     `$ECC_BRIDGE` を参照するだけで変更不要 —— 発注書は `start_demo.sh` を名指ししていたが、
     実際に直すのは `env_demo.sh` 側だった)。

- **承認が要る逸脱**:
  1. **`ecc_core.hpp` / `json_min.hpp` / `check.hpp` を削除しなかった**(発注書は
     「削除」と明記、`check.hpp` だけ「test_ecc_bridge 専用なら」と条件付き)。実際に grep して
     確認した結果、3 つとも **`test_ecc_bridge.cpp` 専用ではなかった**:
     - `fake_ecc.cpp` は `ecc_core.hpp` の `ecc::State` / `ecc::Error` / `ecc::next_state` /
       `ecc::parse_data_link_set` / `ecc::to_string` を状態機械の本体として使っている
       (~20 箇所)。
     - `ecc_e2e_client.cpp` は `check.hpp` の `CHECK` マクロ(52 件の合否そのもの)と
       `json_min.hpp` の `jsonmin::parse`(応答パース)を使っている
       (`ecc_core.hpp` の include はこのファイルでは実は不使用 —— 削らずに残した)。
     削除すると「残す」と明記された `fake_ecc.cpp` / `ecc_e2e_client.cpp` が**ビルド不能**になり、
     `### テスト` 節の受け入れ条件(`run_ecc_e2e.sh` 52 passed)を満たせなくなるため、
     この 3 ヘッダは残す判断をした(`make clean && make -j ...` のクリーンビルドで実証済み)。
     ecc_bridge.cpp・test_ecc_bridge.cpp 自体の削除は発注書どおり実施。
  2. **`src/bin/ecc_bridge.rs` は `bin_support::init_tracing()` を使わず、専用の
     `init_tracing_to_stderr()` を実装した**(発注書は「他の Rust bin と同じ
     tracing_subscriber(env-filter)初期化」と指示)。`bin_support::init_tracing()` は
     既定で **stdout** に書く(`tracing_subscriber::fmt()` の既定 writer)ことを実測で確認 ——
     このままだと `println!("BIND {endpoint}")` の直後に `info!` 等が同じ stdout に混ざり、
     「stdout は BIND 1 行のみ」という契約(この bin 自身のドキュメントコメント、Phase 2
     承認済み逸脱 3)を壊す。試験プロセス側(`Proc::spawn` 系)は stdout の最初の 1 行しか
     読まないため、以後 tracing が stdout に書き続けると読まれないまま OS パイプに溜まり、
     長時間 e2e/soak で書き込みブロック(デッドロック)の恐れもある。実測で確認
     (`init_tracing()` のままだと `BIND ...` の次の行に `INFO ... starting` が stdout に出る、
     `.with_writer(std::io::stderr)` を足すと stdout は `BIND` 1 行だけになる)。
     `RUST_LOG` フィルタの意味論(既定 `info`)は共有ヘルパーと同一。

- **未実施(このレーンでは触っていない)**: `docs/SPEC_ja.md` §1.1/§8.2、`CLAUDE.md` の
  Tech Stack 行(発注書どおり Fable の担当)。demo 実走(上記、worktree 外の理由)。

環境: macOS 26.6.2(Darwin 25.6.0)、rustc 1.97.1、Ice 3.8.2(Homebrew)、2026-09-23。

### 追補 — SIGINT 対応(2026-09-23、Sonnet レーン、レビュー承認後)

**発端**: 076 Phase 3 レビューで、実機(GDED)で `ecc_bridge` が **SIGINT で止まらず SIGTERM
でしか止まらなかった**(ZMQ REP の `recv` でブロック中と推定)ことが報告された。他の Rust bin
は SIGINT で止まる運用(`daq_down.sh`)なので揃える追補が発注された。逸脱 1–3 は承認済み。

- **実装**: `src/bin/ecc_bridge.rs`(+66 行)。
  - 依存を 1 つ追加: **`signal-hook = "0.3"`**(解決 `0.3.18`)。`tokio` は使わない
    (この bin は同期の REP ループで、`bin_support::spawn_sigint` は tokio ランタイム前提のため
    そのまま使えない)。`signal_hook::flag::register(SIGINT, Arc<AtomicBool>)` /
    `register(SIGTERM, ...)` で「フラグを立てるだけ」の async-signal-safe な既製ハンドラを登録
    (自前のシグナルハンドラは書いていない —— signal-hook 自体がその安全性を担保する対象)。
  - REP ソケットに `set_rcvtimeo(500)`(`RECV_POLL_MS` 定数)を追加。ループは
    `Err(zmq::Error::EAGAIN) | Err(zmq::Error::EINTR)` を「タイムアウトまたはシグナル起因の
    中断」として扱い、その時だけ停止フラグを見る(`should_stop_after_timeout`)。フラグが
    立っていれば `info!("stopping (signal)")` を出してループを抜け、その後の
    `info!("shutting down")` を経て `ExitCode::SUCCESS`(= exit 0)。REP ソケットは明示 close
    せず main 終了時の RAII(`set_linger(0)` 済みなので即座)に任せた(発注書は「ソケットを
    閉じて exit 0」だが、close 相当を担保する drop は既存コードのままで足りると判断)。
  - 純粋関数 `should_stop_after_timeout(&AtomicBool) -> bool` を切り出し、ZMQ/signal-hook
    無しで「フラグを見て 1 周で抜ける」判断だけを単体テストできるようにした。

- **実機事象の再現条件についての補足**: EINTR も EAGAIN と同列に扱った理由は、
  シグナル配送がブロッキング中の `recv` システムコールを中断して `EINTR` を返す経路が
  ある(libzmq が内部で吸収せずに伝播するケースがある)ため —— `src/decoder.rs` /
  `src/receiver.rs` の既存 EINTR 処理と同じ流儀。今回の macOS 実測では実際に **EINTR 経路で
  ほぼ即座(約 35 ms)に止まった**(下記)ので、500 ms のタイムアウト待ちは「保険」として
  機能している(signal-hook のフラグ登録だけでは `recv` 自体を割り込ませられない環境でも、
  最悪 500 ms 以内には必ず止まる)。

- **テスト(実行コマンド / green・red)**:
  - `cargo test --bin ecc_bridge`: **8 passed**(既存 7 + 新規
    `tests::should_stop_after_timeout_reads_the_flag_each_cycle`)。
  - `cargo test --test ecc_bridge_intake`(`TPCDAQ_FAKE_ECC_BIN` 未設定): 3 passed
    (新規 `sigint_stops_the_bridge_within_one_second` は「TPCDAQ_FAKE_ECC_BIN が無い」で
    明示 skip、他 2 本は従来どおり実行)。
  - 同(`TPCDAQ_FAKE_ECC_BIN=<worktree>/tools/ecc_bridge/fake_ecc` を渡して実行): **3 passed**
    (`sigint_stops_the_bridge_within_one_second` も実走し green。ログに
    `INFO ecc_bridge: stopping (signal)` → `INFO ecc_bridge: shutting down` を確認)。
  - `cargo test`(フル): **840 passed / 0 failed / 1 ignored**(追補前 838 → 840、+2 =
    上記 2 本の新規テスト)。
  - `cargo fmt`(diff 無し)/ `cargo clippy --all-targets -- -D warnings` /
    `cargo clippy --tests -- -D warnings`: すべて clean。
  - `run_ecc_e2e.sh`(`target/release/ecc_bridge` を再ビルドしてから): **52 passed, 0 failed**
    のまま(回帰なし)。
  - **実機挙動の直接確認(macOS、`kill -INT` / `kill -TERM` を実プロセスへ)**:
    SIGINT で **35 ms** で exit 0、SIGTERM で **33 ms** で exit 0(いずれも 1 秒の受け入れ
    基準を大きく下回る)。stderr の最後の 2 行が `stopping (signal)` → `shutting down`
    であることも確認。

- **承認が要る逸脱**: 無し(発注書の仕様どおり実装。「ソケットを閉じて」を明示 `drop`/
  `close` 呼び出しではなく既存の RAII に任せた点は実装細部であり、契約(exit 0 + 1 行ログ)は
  満たしている)。

- 環境: macOS 26.6.2(Darwin 25.6.0)、rustc 1.97.1、signal-hook 0.3.18、2026-09-23。

