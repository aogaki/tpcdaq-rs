# tpcdaq-rs 仕様書(SPEC)

- **status**: **v1.18(2026-08-16 — 実装の正本)**
- **改訂履歴**: v1.0(2026-08-12 ユーザーレビュー通過)/ v1.1(2026-08-13)graw-writer の
  ファイル分割単位を CoBo 毎 → **AsAd 毎**へ訂正、命名を**実機 DataRouter 形式に完全一致**へ変更
  (§1.1・§6.5・§7・§12-2。ユーザー指示 — オフライン解析の既存 bash 資産を無改造で使うため。
  run 番号管理はログブックと ROOT ファイル側が担う。mini は 1 AsAd で実質不変、
  ELITPC は 2 CoBo × 2 AsAd = 4 ファイル)/ v1.2(2026-08-13)**非 AsAd 制御フレームの保全**を規定
  (§6.5・§7・§12-2。007 実装の E2E で実 2025 run 先頭に frameType 7・12 B の制御フレームを確認。
  実機 FrameStorage は警告して捨てるが、絶対ルール「保存系は意図的ドロップ禁止」を優先し
  `ctrl/` サブディレクトリへ保全 — ユーザー決定)。§3.2 のコンポーネント REP 採番も固定。
  / v1.3(2026-08-13)§6.3 に**イベント内フラグメント順 = (cobo, asad) 昇順**を追加
  (010 実装レビューで確定 — 到着順は run 毎に揺れるため、§12-4 の 2 ソースビルド一致と
  TTree 比較の再現性を成立させるには決定的順序が要る)。
  / v1.4(2026-08-13)ロスレス PUSH に **ZMQ_IMMEDIATE を必須化**(§1.2 — 009 実装で発見:
  libzmq 既定は connect() 時点でローカルキューを作り、**未接続の相手にも HWM 分 send が成功を
  返す**ため、下流不在時の喪失が不可視になる)。
  / v1.5(2026-08-13)run.root の圧縮既定を 505(ZSTD-5)→ **101(ZLIB-1、設定可能)** に変更
  (§6.4 — Warsaw ヒアリング: オフライン解析も DAQ 計算機の旧 ROOT(ZSTD 非対応)で行うため。
  docs/WARSAW_PLAN_ja.md)。
  / v1.6(2026-08-13)§1.3 に**異常中止(abort)の正規経路**を追加(P2 レビュー R3 の解消 —
  中止も必ず EOS で閉じる。run クローズ前に decoder を Reset しない)。
  / v1.7(2026-08-13)**ELITPC 実データ(2022/2026)取得・実測による訂正 3 点**(TODO/019):
  ①ELITPC のワイヤ実態は **1 論理 CoBo × 4 AsAd**(coboIdx=0・asadIdx=0..3、
  `CoBo0_AsAd{0..3}_{TS}_0000.graw`)— v1.1 の「2 CoBo × 2 AsAd」を訂正(Aogaki の解釈:
  2 枚の zCoBo を 1 CoBo として扱っている)。§13-7 の 2-CoBo ジオメトリ問題は**解消**
  (実 ELITPC .dat の COBO 0・4 ASAD と完全整合)。
  ②両年とも **frameType 2(compact)rev 5** — 2022 時点で既に compact。frameType 1 は
  実機オラクル対象外(合成フィクスチャのみ)と確定。
  ③**ローテーションを実機一致へ訂正**(§7): 書き込み**後**に size > max(strict)で次ファイル
  即時オープン(FrameStorage.cpp 実装 + 実データの「各 _0000 = 3852 フレーム = 2^30 超過」で
  確認)。v1.6 までの「書く前判定」は実機と 1 フレームずれる誤りだった。
  §12-1/2 に ELITPC オラクル(`TPCDAQ_REAL_GRAW_DIR`)を追加。§6.4 に用語注意を追記:
  ELITPC オフライン実運用の変換器は TPCReco **grawToEventTPC**(→ PEventTPC)であり、
  「graw2root」(GET 付属 → GDataFrame)とは別物(ユーザー確認 2026-08-13)。
  / **v1.8(2026-08-13)run.root のイベント形式を GDataFrame → PEventTPC(TPCReco 互換)へ
  変更**(§6.4 全面改訂 — **ユーザー裁定: GDataFrame 出力は瑕疵**。オフライン解析は TPCReco で
  行われており、Warsaw も grawToEventTPC 変換で PEventTPC を使っている。我々の出力を
  変換不要でそのまま解析に使える形式に合わせる)。ツリー `TPCData` / ブランチ `Event`
  (PEventTPC、bufsize 128000・splitlevel 2)、充填意味論は grawToEventTPC と同一
  (normal ch のみ・strip 射影・signal 窓・FPN ベースのペデスタル減算既定 ON)。
  GDataFrame は**内部充填表現 + テスト専用出力**へ降格(P2 の mini 実データ全値一致
  オラクルを持つ唯一の回帰のため、PEventTPC の同 run 実データオラクルが閉じるまで維持)。
  / **v1.9(2026-08-14)§5 モニタ PUB の実装確定**(TODO/022 起票に伴う具体化):
  ①§3.2 source_id 空間に **root-sink = 101** を追加(PUB Batch の source_id)。
  ②§5.3 に PUB ワイヤ形式を確定(エンベロープは §2.2 と同形、payload は map 形式 msgpack の
  `status` / `hist_snapshot` / `built_event` 3 種。ヒストのビン値は f64 LE の Bin で運ぶ)。
  ③モニタ PUB リンクの sequence_number は **run リセットなしの単調増加**(§2.2 の
  run 開始 0 リセットはロスレスリンクの連続性検証の規則。モニタリンクの用途はギャップ =
  ドロップ数の可視化のみで、リセットは巻き戻り誤検出の特例を生むだけのため)。
  ④§5.2 の「波高」の集計単位を明確化: **ジオメトリで Strip に割り付いたチャンネル毎**
  (= 物理ストリップ毎。同一ストリップ番号の複数セクションは同一ビンへ別エントリ)、
  そのイベントでサンプルが 1 個以上あったチャンネルのみ計数。incomplete イベントは
  届いた分で fill、emit 済みイベントへの遅延フラグメントは fill しない(late_fragments
  カウンタのみ)。
  / **v1.10(2026-08-14)P2 批判的レビュー(TODO/P2_REVIEW.md)+ 016 レビューの
  処置確定(ユーザー承認)による改訂 8 点**:
  ①§6.2-5 を拡張 — root-sink は **run 中の run_number 食い違い(Data/EOS とも)を
  fatal(exit 6)に昇格**(R-P2-1。decoder は単一ストリームで run を混ぜない契約 —
  混在に「正しく続ける」方法はなく、旧挙動はプロトコル違反 run が完成 run 名に化けた)。
  ②§1.4 に検出応答の役割非対称を明文化(R-P2-6)。
  ③§6.3 に重複フラグメントの意味論を明記(R-P2-4 — 計数のうえ全部 fill)。
  ④§5.3 status に `pending_events` を追加(R-P2-5 — ビルダ組み上げ中の瞬間値の可視化)。
  ⑤§9.2 run_stop counters の **取得不能項目は null**(016 逸脱③ — 0 と「不明」を
  混同させない。events_built/events_incomplete/late_fragments は root-sink が REP を
  持たない間は null)。
  ⑥§8.1 — comment 投稿は token 免除の明文化(016 逸脱⑧ — 記録系。シフト全員が書ける)。
  ⑦§8.1 — run 番号手動設定 REST の形を確定(`POST /api/run/next`)。
  ⑧§12-8 に run 境界跨ぎのスループット測定を追加(R-P2-14)。
  / **v1.11(2026-08-14)§13-7 のデータリンク本数を解消**(Aogaki 情報): 2 枚の zCoBo は
  同一筐体内で内部ネットワークを 1 つに束ねてから PC 接続 = **データリンクは 1 本**
  (DataSender 1 エントリ・receiver 1 台。v1.7 のワイヤ実態と整合)。P5 初日の目視確認のみ残す。
  / **v1.12(2026-08-14)実 GET ソース調査(TODO/032・033)+ 連続 run のユーザー裁定(TODO/034)**:
  ①**§1.3 run 開始に「ECC を Idle へ戻す」段を追加**(ユーザー決定 — 実運用が「毎 run 完全
  リセットして一からやり直す」作法。かつ `ecc stop` 後は `Ready` で `describe` が通らないため
  2 本目以降の必須条件。TODO/030 の跨 run 実測で判明、TODO/034)。
  **同日中に訂正**: 実 ECC の `reset` は `EV_UNDO` = **1 段戻す**意味論で `Active` からは**無音で
  無視**され、`configure` も `ST_PREPARED` ガードで黙ってスキップされる(TODO/034 が実 ECC
  ソースで発見)。よって `Ready → Idle` は **`breakup → reset → reset` の歩き戻し**が必要。
  実装は TODO/036。
  ②**§1.3 停止シーケンスの事実修正**(TODO/032 — 実 GET の `daqStop` は**データリンクを
  close しない**。close は breakup か次 configure。**データリンクを張るのは CoBo 自身**で ECC は
  probe を張らない)+ 第一段を**受信静止検出に置換**(TODO/033、`eos_quiesce_ms` 新設 =
  既定 500 ms。`eos_timeout` はハード上限として存置)。強制 EOS を「例外」から**正規経路**へ。
  ③**§1.3 異常中止の機序を実装実態に訂正**(TODO/033 — decoder Reset は同一 run 内 seq ギャップを
  作らない(打ち切りは末尾バッチでスレッドは直後に終了)。実害は **EOS バリア喪失 → 次 run 冒頭の
  run_number 食い違い fatal(exit 6)として遅発**。誤りの起源は P2 レビュー R3 の仕様合成)+
  **終端条項**(強制 EOS も流れ切らなければ畳んでよい / 次 run 前に root-sink を再起動)を追加。
  ④**§9.2 run_stop に `forced_eos` / `eos_closed` を追加**(TODO/033 — 実機 TCP flow では
  `forced_eos=true` が常態で、**異常の印は `eos_closed=false` のみ**)+ reason の `"abort:..."` を明文化。
  ⑤**§1.4-6 receiver 単一リンク規約**(先勝ち + 余分接続は即 close + `extra_connections` /
  0 バイト接続は EOS を構成しない)+ **§12-13** + **§13-7 に P5 の機械確認手段**を追記(TODO/032)。
  / **v1.13(2026-08-15)ECC ConfigId の 3 相化(TODO/038 レーン A の発見、docs/VIRTUAL_ZCOBO_ja.md
  §6-R9 裁定)**: 実 ECC の ConfigId は **describe / prepare / configure の 3 組**で、実運用は
  別名を使う(実例: `describe=zCobo-ZC706, configure=pulser`。ECC の設定リポジトリは
  `describe-<id>.xcfg` / `configure-<id>.xcfg` / `hardwareDescription_<id>.xcfg` のフラット
  ディレクトリで、**prepare の読み込み実装も `configure-` プレフィクスを読む**)。
  ①§3.1 controller 設定の `config_id` を「文字列(3 相同値の略記)または
  `{describe, prepare, configure}` テーブル」の両対応に ②§8.1 run シーケンスは相ごとの id を
  ecc-bridge へ渡す(ecc-bridge JSON は元よりアクション毎 `config_id` — 変更なし)
  ③§9.2 run_start に **非同値時のみ** `config_ids` オブジェクトを追加(nullable 規律に整合)。
  あわせて §3.1 設定例の `ecc_proxy` の identity を実 servant id **`Ecc`** に訂正
  (`GetEcc` は stale — 038 実測。fake_ecc は元より正しい)。実装は TODO/042。
  / **v1.14(2026-08-15)統合デモ(TODO/041 — 実 ECC + vcobo-daq での run 実走)の発見 2 点**:
  ①**§8.2 ecc-bridge の status 応答に `ecc_error` を追加**(GET の error フィールド =
  `NO_ERR`/`WHEN_DESCRIBE`/… を運ぶ。実測: describe 失敗後の実 ECC は `IDLE/WHEN_DESCRIBE` を
  抱えるが、現行 status は state しか返さず UI から不可視。実装は TODO/043)。
  ②**§9.2 run_stop の意味論に注記追加**: 実機 TCP flow では `forced_eos:true` が常態
  (§1.3 — stop はリンクを閉じない)であるため、**`forced_eos:false` は「stop より前に
  データリンクが死んだ」ことの強い印**(D-2 実測: CoBo 突然死 = OS の正常 FIN → 自然 EOF →
  run が normal として閉じ、他に痕跡が残らない)。「唯一の異常の印は `eos_closed:false`」の
  記述を「`eos_closed:false` に加え、`forced_eos:false` も要注意の印」に改める
  (033-A の実装に織り込む — TODO/033 に追記済み)。
  / **v1.15(2026-08-15)§12-5 連続負荷の二段化(ユーザー裁定)**: 自宅ソフト soak =
  一晩(≥8 h、既定 12 h)のトレンド駆動(RSS/fd/カウンタ平坦 + ロスゼロ)、**フル 24 h は
  ハード込みで ELI-NP に移管**。根拠: ソフト単体で捕まるのはリーク・成長・ドリフトで、
  一晩 + トレンド外挿で足りる。24 h 固定の固有価値は実データ・実タイミング・実温度が乗る
  ハード込みでのみ生じる。ソフト soak は「実機 soak をハード起因の問題だけに純化する
  フィルタ」という位置づけ。実装 = TODO/031。
  / **v1.16(2026-08-15)§12-6 瞬発負荷の定義改訂(Fable 裁定 — 031 実測起因)**:
  「ペーシングなし全速」→ **「ペーシング付き ≥3× 目標(mini ≈ 672 Mbps)10 分 drop 0」**。
  loopback 全速は 216× 目標(6.06 GB/s)に達し物理的対応物が無い。副産物の設計検証:
  216× 条件で receiver の never-stop + counted drop(§1.4)が silent でなく働くことを実測。
  あわせて 031 の soak サニティが root-sink の実欠陥 2 件(RSS +0.55 MB/event 成長 /
  天井 ≈30 events/s)を捕獲 — 修正 = TODO/053、soak 本走行はその後。
  / **v1.17(2026-08-15)GDataFrame の全撤去(ユーザー裁定)**: GDataFrame は
  graw2root(GET 付属の別ツール — §6.4 v1.7 の用語注意で「混同注意」とした、まさにその
  別系統)の形式であり我々のチェーンに不要。v1.8 の削除条件(PEventTPC 実データオラクルの
  成立)は 021 で満了済み。§6.4 = 中間表現撤去(Filler が Fragment 直読)+
  `--format gdataframe` モード・専用回帰の撤去、§12-3 = 「旧 GDataFrame 比較を維持」の
  文言を撤去。等価性の担保は内容一致オラクル(3852 events / 0 differences)が担う。
  性能上も中間表現は 21 ms/event の 15% を占めていた(053 計測)。実装 = TODO/054。
  / **v1.18(2026-08-16)§5.2 モニタ 2D 表示の改訂(ユーザー裁定 — ELITPC デモ
  実走レビュー起因)**: ①UI の 2D は縦 = strip / 横 = time bucket(TPCReco 慣習。
  ワイヤ・monitor.root は不変、転置は UI 描画層のみ)②Event Display に表示専用の
  ベースライン減算(strip 毎に先頭 25 cell 平均、負値保持。オフラインは自前で再計算する
  ため・経験則をデータに焼き込まないため、ワイヤと保存系には適用しない)
  ③Waveform は生 ADC のまま / StripTime は Σ ADC 積算のまま(per-event ビューは
  Event Display が担う)。実装 = TODO/058(UI のみ、+6 テスト)。
  **同日追補(調査 3 レーンの確定事項)**: ①GDataFrame の出自 = GET **CoBoFrameViewer**
  パッケージのオフライン ROOT 永続化モデル(graw2root 変換器 + root2disp ビューア専用。
  ライブビューアすら CoBoEvent 直読)②TPCReco では `fillEventFromFrame(GET::GDataFrame&)`
  の**入力アダプタ層**(WITH_GET ガード内のみ、ROOT 出力に 1 バイトも入らない、解析/GUI は
  不知)③よって撤去は grawToEventTPC 互換に**無影響** — Filler 直読の意味論の正本は
  `EventSourceGRAW.cpp:301-323` と `PedestalCalculatorGRAW.cpp`(054 発注書に指針明記)
  ④**注意(記録)**: TPCReco latest スナップショットの `EventSourceGRAW.cpp:262` は
  runId を eventId で**上書きする**(本家のバグと思われる)。実機配備版の実出力
  (`PEventTPC_2026-08-11...root` の runId=20260811074737)は run 開始時刻であり、
  **我々の実装は実機出力側に一致**(§6.4 の既定を維持。BuildIndex の意味からも正)。
- **正本性**: 本書が実装の正本。PROPOSAL v0.4 と食い違う場合は本書が勝つ(差分は §14 に列挙。
  PROPOSAL v0.5 への反映は Warsaw フィードバックと併せて判断 — 未実施)。
- **入力**: PROPOSAL_ja.md v0.4 / delila-rs 実装調査 / C++ 版 tpcdaq 実装調査 /
  TPCReco `GeometryTPC` 調査 / delila-rs `tools/root_sink` 調査(いずれも 2026-08-12 実施)。
- **原則**: 迷ったら KISS(CLAUDE.md 原則 1)。本書に「未決」は残さない — 実測が要るものは
  「P5 実測で決める」を決定として明記する。

---

## 0. 決定サマリ(TODO/000 の 9 決定事項 → 節対応)

| # | 論点 | 決定(一行) | 節 |
|---|------|------------|-----|
| 1 | ヒスト集計の持ち主 | **root-sink(C++)に一元化**。スナップショットを PUB、monitor/UI は表示に徹する | §5.1 |
| 2 | ZMQ メッセージ形式 | delila-rs 方式(MessagePack positional + JSON 制御)を踏襲、**ただし全バッチに run_number を載せる** | §2 |
| 3 | JSONL スキーマ | 5 レコード型(run_start / run_stop / audit / comment / psu)、controller 単一ライタ | §9 |
| 4 | WS プロトコル | C++ 版の 13 バイトヘッダ・バイナリ枠組みを継承し型を再定義。適合性テスト方式も踏襲 | §10 |
| 5 | コンポーネント境界・設定 | 7 コンポーネント(+P6 で psu)。設定は **TOML**、CoBo は `[[cobo]]` 配列 | §1, §3 |
| 6 | root-sink 手術範囲 | 骨格(スレッド/Channel/RunState/Recorder)流用。**eventIdx ビルダとヒスト書き出しは新規**、SUB→PULL 反転 | §6 |
| 7 | R4 波高定義 | 生 ADC 時間方向最大値、x = 0–4096 固定 512 ビン、**飽和率表示を採用** | §5.2 |
| 8 | 受け入れ数値 | **連続 24 時間 / 100 Hz 相当 / 保存系 drop 0** ほか(全数値 §12) | §12 |
| 9 | 検証項目リスト | MTU 9000 / NIC 構成 / AGET デッドタイム / 2-CoBo .dat 入手 ほか | §13 |

---

## 1. システム構成

### 1.1 コンポーネント一覧と責務

| コンポーネント | 言語 | 個数 | 責務 |
|---|---|---|---|
| receiver | Rust | CoBo 毎に 1 | TCP listen(CoBo 毎ポート)、MFM フレーミング、drain のみ(never-stop)。生フレームを graw-writer と decoder へ PUSH |
| graw-writer | Rust | 1 | 生フレームを **AsAd 毎ファイル**(ヘッダ asadIdx で振り分け)へバイト一致 append(§7) |
| decoder | Rust | 1 | frameType 1/2 デコード → Fragment(§2.4)を root-sink へ PUSH。内部ワーカー並列(delila-rs reader 方式) |
| root-sink | C++ | 1 | **イベントビルダ + PEventTPC(TPCReco)互換 TTree(v1.8)+ ヒスト集計 + run\<N\>_monitor.root**。ヒスト/最新イベント/状態を PUB(§5, §6) |
| monitor | Rust | 1 | root-sink の PUB を購読し WS へ変換する**ゲートウェイ**(集計しない)。ジオメトリで UVW グリッド化(§5.4) |
| controller | Rust | 1 | run 制御オーケストレーション、REST API、操作権トークン、run 番号採番、**JSONL 単一ライタ**(§8, §9) |
| ecc-bridge | C++ | 1 | Ice クライアント(encoding 1.1 固定)。JSON REQ/REP ↔ ECC(§8.2) |
| psu | Rust | 1 | (P6)HiVolta/HMP2020 のポーリングと制御。TRIP を controller へ投稿 |

- ドメイン核(framer / decode / geometry / ビルダ判定)は IO 非依存の純ロジックとして分離
  (root_sink の `eb_core.hpp`/`sink_core.hpp` 純ヘッダ方式、C++ 版 tpcdaq の pure-core と同じ流儀)。
- リポ配置: Rust は単一 cargo ワークスペース(`src/bin/` にコンポーネント毎バイナリ + `src/` 共有ライブラリ)。
  C++ サテライトは `tools/root_sink/`、`tools/ecc_bridge/`。GET 由来(CeCILL)は `third_party/get/` に隔離(§6.6)。

### 1.2 配線図とソケット種別

```
CoBo k ──TCP:46005+k──▶ [receiver k] ──PUSH──▶ (PULL bind) [graw-writer]      ロスレス
                             │
                             └──PUSH──▶ (PULL bind) [decoder]                   ロスレス
                                             │
                                             └──PUSH──▶ (PULL bind) [root-sink] ロスレス
                                                              │
                                                              └──PUB──▶ (SUB) [monitor] ──WS──▶ ブラウザ
                                                                  ヒスト snapshot / 最新 built event / status   最新優先
[controller] ──REQ/REP(JSON)──▶ 各 Rust コンポーネント(状態遷移コマンド)
[controller] ──REQ/REP(JSON)──▶ [ecc-bridge] ──Ice(1.1)──▶ ECC:46002
[各コンポーネント] ──PUSH──▶ (PULL bind) [controller]  ログブック投稿(§9)
```

- **ロスレス系 = PUSH/PULL**(有限 HWM の背圧)、**モニタ系 = PUB/SUB**(間引き可・ドロップはギャップ検出で可視化)。
- **ロスレス PUSH は `ZMQ_IMMEDIATE` 必須**(v1.4): libzmq 既定は connect() 時点でローカル
  キューを作り、未接続の相手にも HWM 分の send が成功を返す — 下流不在で「送れたことになる」のは
  ロスレス契約違反(喪失の不可視化)。IMMEDIATE で**接続確立済みの相手にだけ積む**。
  適用リンク: receiver→graw-writer / receiver→decoder / decoder→root-sink。実装は zmq_helper の
  PUSH ヘルパに集約(焼き込み分散禁止)。モニタ系 PUB には適用しない(落として良いリンク)。
  delila-rs の「全リンク PUB/SUB + HWM=0 無制限バッファ」からの**意図的差分**(§14-4):
  ELITPC の ~111 MB/s では無制限バッファ = メモリ暴走リスクのため、有界 + 可視エラーを選ぶ。
- **PULL 側が bind**(安定エンドポイント)、PUSH 側が connect。ZMQ の per-peer FIFO により
  ソース毎の順序は保存される(graw-writer のファイル順序正しさの根拠 — AsAd 毎ファイルは
  ソース列の部分列なので、この保証がそのまま AsAd 毎の順序保証になる)。
- decoder → monitor の直結リンクは**設けない**(PROPOSAL 図からの変更、§14-2)。イベント表示・波形
  ビューの供給元は root-sink の built-event publish に一元化する。理由: (a) 2 CoBo 時に「表示イベント」
  の断片が食い違う問題が構造的に消える、(b) 集計と表示が同一のビルド結果を見る、(c) リンクが 1 本減る。
  root-sink が死ぬとモニタも止まるが、root-sink はロスレス保存系であり死んだ時点で run は続行不能。

### 1.3 プロセスモデル・状態機械・run シーケンス

- コンポーネント状態機械は delila-rs をそのまま採用:
  `Idle -Configure→ Configured -Arm→ Armed -Start→ Running -Stop→ Configured`、`Reset` で Idle、
  異常時 `Error`(Reset でのみ脱出)。コマンドは JSON over REQ/REP(§2.6)。
- **root-sink と ecc-bridge は REP コマンドソケットを持たない**。root-sink はデータ駆動
  (最初の Data で run open、全ソース EOS で close — delila-rs root_sink の RunState 方式)。
  ecc-bridge は controller 専用の REQ/REP のみ。
- run 開始シーケンス(controller が実行):
  1. run 番号採番(§8.1)→ 各 Rust コンポーネントへ `Configure`(下流から: graw-writer, monitor → decoder → receivers)
  2. `Arm` — **receiver はここで bind + listen**(listen-before-start の実装点)
  3. `Start{run}` — 書き手はファイル準備、receiver は accept 開始
  4. ecc-bridge 経由で **`reset`(= ECC を `Idle` へ戻す)→** `describe → prepare →
     configure(DataLinkSet XML)→ start`
     - **毎 run 先頭で必ず ECC を `Idle` へ戻す(v1.12、ユーザー決定 2026-08-14)**: 実運用
       (ワルシャワ大学)が「**毎 run 完全にリセットして一からやり直す**」作法であり、それに合わせる。
       **1 本目でも同じ経路を通す**(状態による分岐を作らない)。
     - **⚠ 実 ECC の `reset` は「1 段戻す(`EV_UNDO`)」意味論であり、`Idle` への直行ではない
       (v1.12 訂正 — TODO/034 が実 ECC ソースで発見、発注側で裏取り済み)**:
       `EV_UNDO` の遷移は `Described→Idle` と `Prepared→Described` の 2 本のみで、
       **`Active`(= Ready/Running/Paused)からの `EV_UNDO` は存在しない**
       (`GetBench/src/get/rc/BackEnd.cpp:924` / `:250-270`)。未定義遷移は例外も出さず
       **`Ignored` で無音**(`StateMachine/src/dhsm/Engine.cpp:344`)。さらに
       `BackEnd::configure` は **`if (state == ST_PREPARED)` ガードで黙ってスキップ**する
       (`BackEnd.cpp:955-962`)。
       → `ecc stop` 後の `Ready` から `reset` を 1 回打つだけでは **何も起きず、続く
       describe / prepare も無音、configure もスキップされ、`start` だけが成功する**。
       **`Ready → Idle` には `breakup`(Active→Prepared)→ `reset`(→Described)→
       `reset`(→Idle)の歩き戻しが要る**。controller は**現在の ECC 状態を見て必要な段数だけ
       歩き戻す**(§8.2 の状態を使う)。
       **`ecc stop` 後の ECC は `Ready`** で、`describe` は `Off / Idle / Described` からしか
       許されないため、この歩き戻しが無いと **2 本目以降の run が成立しない**
       (TODO/030 の跨 run 実測 + TODO/034 の実 ECC 解析)。
       **オペレータに手で `POST /api/ecc/reset` を挟ませない**。
     - `configure` で **CoBo がデータリンクを張り直す**(下記 v1.12 注記)ので、receiver の
       「run 毎に接続 1 本」という意味論とも整合する。**歩き戻しが不完全なまま `start` すると
       `configure` がスキップされ、CoBo がリンクを張り直さないまま run が始まる**
       (実機では前 run のソケットが receiver 側で閉じられているため
       「Could not establish data link.」= 可視な失敗になるが、いずれにせよ run は成立しない)。
     - controller は **`reset` 直後の ECC 申告状態を audit(`ecc_state_after_reset`)に必ず残す**
       (silent にしない。実機で `"Ready"` が出たら歩き戻しが効いていないシグナル)。
     - `Arm` は前 run のソケット解放待ちで一時的に bind 失敗しうる(libzmq の close は非同期)。
       controller は**指数バックオフでリトライ**し、**粘った回数と所要を audit
       (`arm_retries`)に必ず記録**する(実測: 最悪 4 試行 / 157 ms。上限 6 試行 / 620 ms)。
     - **run 開始シーケンスが失敗したら `next_run` を巻き戻す**(controller は単一書き手)。
       ただし `run_start` をログブックへ書いた後は巻き戻さない。「番号は飛んでよい」(§12-11)は
       維持するが、**運用でふつうに使って飛ぶ状態は無くす**。
  5. run_start レコードを JSONL へ(§9)
- run 停止シーケンス:
  1. ecc `stop`(CoBo が送信停止。**実 GET は stop ではデータリンクを close しない** — close は
     breakup(`daqDisconnect`)または次の `configure` の再接続時。20190315_patched
     `DaqCtrlNodeI::daqStop` / `disconnect` で確認、v1.12。**データリンクを張るのは CoBo 自身**で、
     ECC は Ice で指示するだけ。接続確立は `configure` の時点)
  2. receiver: EOF が届いた場合(breakup 先行・リプレイ等)は `EndOfStream` を下流全リンクへ。
     **実機の通常経路では ecc stop 後に EOF は届かない**(TODO/032 調査)ため、controller は
     「自然 EOS の完了」または「全 receiver の受信静止(受信バイト数が `eos_quiesce_ms`
     (既定 **500 ms**、設定可)のあいだ不変。不達の receiver は静止とみなす)」の早い方まで待ち、
     receiver への `Stop` コマンドで **EOS を注入する(これが正規経路)**。`eos_timeout`
     (既定 **5 秒**、設定可)は両段のハード上限。静止検出は ecc stop の flush 済み在飛データを
     飲み切ってから畳むための待ちであり、ロスレス規約の一部である(v1.12)
  3. root-sink: 全ソース EOS 到達 = in-band バリア → TTree finalize + **run\<N\>_monitor.root 書き出し**(R10)
  4. コンポーネント `Stop`(上流から)、run_stop レコードを JSONL へ
- **異常中止(abort)の正規経路(v1.6、v1.12 で機序を実装実態に訂正)**: 中止も必ず
  「EOS を流して閉じる」: ecc `stop`(不達でも続行)→ receiver へ `Stop`(強制 EOS)→
  EOS がチェーンを流れて root-sink が run をクローズ(incomplete は可視カウント)→
  その後に各コンポーネントの `Stop`/`Reset`。
  **run がクローズする前に decoder を Reset しない**。理由(v1.12 訂正): Reset の送出打ち切りが
  作るのは「同一 run 内の seq ギャップ」ではない(打ち切られるのは末尾バッチで、以後の送出
  なしにスレッドが終了するため §6.2-5 の Gap 検出は発火しない)。実害は **EOS バリアの喪失**で
  あり、root-sink は run を開いたまま残り、**次の run の最初の Data で run_number 食い違いの
  fatal(exit 6、§6.2-5)として遅発する**。`Reset` は run クローズ後の Error 復旧専用。
  例外: root-sink 自体が死んでいる場合のみ上流の Reset は無条件に可(下流不在への abandon は
  可視カウントされ無害)。
- **EOS が強制でも流れ切らなかったとき(v1.12 追加 — 終端条項)**: 強制 EOS 後も `eos_timeout`
  内に伝播を観測できなければ、controller はそれ以上待たずにコンポーネントを畳んでよい
  (`run_stop` に `ok: false` と `eos_closed: false` を記録 — §9.2)。このとき root-sink の run は
  開いたままである。**次の run を開始する前に root-sink を再起動(または fatal 死を回収)する
  こと** — さもなくば次 run の最初の Data が上記の遅発 fatal を踏む(これは正しい検出であり、
  抑止しない)。run_stop は `ok: false, reason: "abort:..."` または `"error:eos-timeout"`。
- 起動順(プロセス起動そのもの)は任意(ZMQ connect はリトライされる)。起動スクリプトの推奨順は
  bind 側から: graw-writer → decoder → root-sink → monitor → controller → receivers → ecc-bridge。

### 1.4 過負荷時の縮退規約(ロスレス系の限界の扱い)

「保存系は絶対に落とさない」「receiver はソケットへ逆圧しない」は、ディスクが恒常的に入力より遅い
状況では同時には満たせない。その場合の規約を仕様として固定する:

1. receiver の受信タスクは常にソケットを drain する(never-stop)。送信タスクとは有界キューで分離。
2. ロスレスリンクは有限 HWM(既定 1000 メッセージ)+ 受け側有界内部キュー。一時的な停滞
   (ディスクスパイク等)はバッファが吸収する(既定サイズは目標レート **2 秒分**以上)。
3. receiver 内部キューが満杯に達したら、それは**システム過負荷 = Error 状態**。以降のフレームは
   `overflow_frames` としてカウントし(silent 禁止)、コンポーネントは Error を報告、controller は
   run を異常停止して JSONL に記録する。「静かに間引いて run を続ける」ことはしない。
4. モニタ系(root-sink → monitor の PUB)は常時間引き可。ドロップは sequence_number ギャップとして
   受信側で検出し、UI に累積数を表示する。
5. **契約違反検出時の応答の役割非対称(v1.10 明文化、R-P2-6)**: REP を持つコンポーネント
   (receiver / decoder / graw-writer)は **Error 状態を報告して drain は継続**し、止める判断は
   controller が行う(§1.3)。REP を持たない root-sink は **即 fatal 終了**が唯一の可視な
   失敗表明であり、黙って走り続けるより正しい(§6.2-5/6)。
6. **receiver のデータリンクは同時に 1 本(先勝ち、v1.12)**: 接続保持中に到着した余分な接続は
   accept して即 close し、`extra_connections` としてカウント + 初回 warn(黙って backlog に
   滞留させない — silent stall 禁止)。**1 バイトも運ばなかった接続の終了(EOF / エラー)は
   run 境界(EOS)を構成しない** — `empty_connections` としてカウントする(迷い込み接続の
   即断が偽 EOS で run を閉じるのを防ぐ。§1.3 の強制 EOS 経路は不変)。現接続 peer と
   最終受信時刻(`last_read_unix_ns`)は GetStatus で可視。
   根拠: 実機のデータリンクは CoBo が configure 時に張る 1 本のみで、ECC は probe を張らない
   (TODO/032 調査)。GET 純正 DataRouter も単一接続(確立後は listen 自体を閉じる)であり、
   本規約はそれを可視化強化した同型。

## 2. ZMQ メッセージ仕様

### 2.1 直列化方式

- **データ面 = MessagePack**(rmp-serde、compact/positional — フィールド名はワイヤに乗らない)。
  1 ZMQ メッセージ = 単一フレーム。トピックフレームなし(SUB は空プレフィックス購読)。
- **制御面 = JSON**(serde_json)。REQ/REP。
- この分離は delila-rs と同一。C++(root-sink)側のデコードは delila-rs `tools/delila2root/TDelila.hpp`
  の MessagePack リーダを流用する。

### 2.2 エンベロープと Batch

```
Message(enum, fixmap(1) {variant 名: ペイロード}):
  Data(Batch)
  EndOfStream { source_id: u32, run_number: u32 }
  Heartbeat   { source_id: u32, run_number: u32, counter: u64 }

Batch(positional array(5)):
  [ source_id: u32,         // 送り手の一意 ID(§3.2 の表)
    run_number: u32,        // ★全バッチに必ず載せる
    sequence_number: u64,   // ソース毎単調増加。run 開始で 0 リセット
    created_ns: u64,        // 送信時 unix ns(レイテンシ計測用)
    payload ]               // リンク別(§2.3)
```

- **run_number を全バッチに載せるのは delila-rs からの意図的差分**(§14-3)。delila-rs は run 番号を
  コマンドと EOS のみに載せ、コンシューマが latch する設計だが、そこから事故が 2 件記録されている
  (stale EOS / EOS 消失で root_sink が writing のまま固まる)。u32 一個の追加コストで latch を消す。
- **EndOfStream はいかなる間引き・破棄規則からも除外**(delila-rs の実地事故の教訓)。
- Heartbeat は各ソースがアイドル時 1 Hz で送出(コンシューマの死活判定用。データ到着中は不要)。
- sequence_number はロスレスリンクでは受信側で**連続性を検証**し、ギャップ = プロトコル違反として
  Error(§1.4-3)。モニタリンクではギャップ = ドロップ数として可視化。

### 2.3 リンク別ペイロード

| リンク | payload | 中身 |
|---|---|---|
| receiver → graw-writer | `RawFrames` | `Vec<Bin>` — MFM フレームのバイト列そのまま(フレーム境界 = 要素境界) |
| receiver → decoder | `RawFrames` | 同上(同一バッチ内容。送信は独立ソケット・独立 seq) |
| decoder → root-sink | `Fragments` | `Vec<Fragment>`(§2.4) |
| root-sink → monitor(PUB) | `Snapshot` | §5.3(ヒスト)/ §5.4(built event)/ status。形式は map 形式 msgpack(`to_vec_named` 相当)で自己記述にする(Rust↔C++ 双方向でないため進化容易性優先 — delila-rs の EbMessage と同じ理屈) |
| 各 → controller(PUSH) | `LogPost` | JSONL 1 レコード分の JSON 文字列(§9。controller が ts/seq を付与) |

- バッチ詰め: receiver は「8 MiB 到達 or 10 ms 経過」の早い方でバッチを閉じる(設定可)。
  ホットパスで per-frame の ZMQ send / heap 確保をしない(CLAUDE.md)。
- **decoder のソース性(2026-08-12 明確化)**: decoder は Fragments を `source_id = 100`・自前
  sequence_number の**単一ストリーム**として送出し、上流全 CoBo の EOS 受領 + 対応 Fragment の
  送出完了後に**自分の EOS を 1 本**送る。CoBo の識別は Fragment.cobo が担う。したがって
  root-sink の期待ソース集合は {decoder} のみ(RunState が単純化)。graw-writer は receiver 直結
  なので期待ソース = 設定の CoBo 集合のまま。

### 2.4 Fragment(デコード済みフレーム)

1 CoBo フレーム(= 1 AsAd 分)→ 1 Fragment。GDataFrame(§6.4)を再構成できる全ヘッダを運ぶ。

```
Fragment(positional array):
  [ event_idx: u32, event_time: u64(48bit 有効), cobo: u8, asad: u8,
    frame_type: u8, revision: u8, read_offset: u16, status: u8,
    mult: [u16;4], window_out: u32, last_cell: [u16;4],
    items: Bin ]   // u32 LE 配列。1 item = 1 サンプル
```

item の 32bit パック(frameType 1 のビット割りを正規形として両 frameType をこれに正規化):

| ビット | 内容 |
|---|---|
| [31:30] | aget(0–3) |
| [29:23] | chan **raw 0–67**(FPN 込み。ジオメトリ変換はしない — 電子回路空間のまま運ぶ) |
| [22:14] | bucket(0–511) |
| [13:12] | 予約(0) |
| [11:0] | ADC 値(生、減算なし) |

- hitPat(36 バイト)は運ばない(C++ 版 RootWriter も fHitPatterns 未使用 — 生 graw が可逆バックストップ)。
- ワイヤコストは生フレームの約 2 倍(2025 compact 比)だが localhost/LAN では問題にならず、
  デコードは 1 回だけ・全コンシューマ共有という単純さを優先する。

### 2.5 スキーマ漂流ガード

delila-rs `src/common/delila_schema.rs` の方式を踏襲する:
- フィールド順序と型タグを Rust の定数表として一度だけ宣言し、実構造体を直列化して表と突き合わせる
  ユニットテストを置く(表と実装がズレたらテストが落ちる)。
- C++ 側(root-sink)のデコーダは delila 方式の「先頭 N フィールドを読み、以降は skip」で前方互換。
- ZMQ ストリーム自体にはスキーマヘッダを載せない(ファイルには載せる必要が出た時に検討)。

### 2.6 コマンドチャネル(JSON REQ/REP)

- `Command`: `{"Configure": {"run_number": N, "comment": "...", "config": {...}}}` /
  `"Arm"` / `{"Start": {"run_number": N}}` / `"Stop"` / `"Reset"` / `"GetStatus"`
  (serde enum の JSON 表現。delila-rs `command.rs` と同形)。
- `CommandResponse`: `{ "success": bool, "state": "...", "message": "...", "run_number": N|null,
  "metrics": {...}|null }`。metrics は run_stop レコードの材料(frames/bytes/drops 等)。
- REP ソケットはエラー時に再 bind してコンポーネントが制御不能に陥らないこと(delila-rs TODO 58 の教訓)。
- **UI からの直接 ZMQ は禁止**。操作は常に controller の REST(§8.1)経由。

## 3. 設定スキーマ(TOML)

### 3.1 例

```toml
# config/mini.toml — mini eTPC(1 CoBo)
[system]
experiment = "mini_eTPC"
output_root = "/data/tpcdaq"          # 出力・logbook・状態ファイルの根
geometry = "config/geometry_mini_eTPC.dat"

[[cobo]]
id = 0                                 # source_id にもなる(§3.2)
listen = "0.0.0.0:46005"              # CoBo からの TCP 着信
data_sender_id = "CoBo[0]"            # ECC の NodeId 表記そのまま(大文字 TCP と並ぶ実機の罠)

[decoder]
workers = 1                            # 予約(P5 実測まで単線 — 009。>1 は受理するが未使用)

[root_sink]
snapshot_hz = 1.0                      # ヒストスナップショット配信周期
event_publish_hz = 20.0                # 最新 built event の配信上限
build_timeout_ms = 1000                # フラグメント待ちタイムアウト(壁時計)

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"               # 操作権取得時の共有パスフレーズ(事故防止であり認証ではない)
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"  # servant identity は "Ecc"(v1.13 訂正)
config_id = "default"                        # 略記 = 3 相同値。相ごとに変える場合(v1.13):
# config_id = { describe = "zCobo-ZC706", prepare = "pulser", configure = "pulser" }
```

ELITPC(2 CoBo)は `[[cobo]]` を 2 ブロック書くだけ(`id = 0/1`、`listen` ポートを分ける)。
receiver プロセスは `tpcdaq-receiver --config x.toml --cobo-id K` で CoBo 毎に起動する。

### 3.2 ポート・ID 規約(既定値、すべて設定で上書き可)

| 用途 | 既定 |
|---|---|
| CoBo データ着信 | `46005 + cobo_id`(DataLinkSet XML が CoBo 毎に向け先を指定) |
| graw-writer PULL bind | `tcp://*:47001` |
| decoder PULL bind | `tcp://*:47002` |
| root-sink PULL bind | `tcp://*:47003` |
| root-sink PUB bind | `tcp://*:47004` |
| controller ログ投稿 PULL bind | `tcp://*:47005` |
| コンポーネント REP | graw-writer = `47100`、decoder = `47101`、root-sink = `47102`、receiver k = `47110+k`(v1.2 で固定) |
| ecc-bridge REP | `tcp://*:47200` |
| controller REST / monitor WS | `8080` / `9000` |
| source_id | receiver = `cobo_id`、decoder = 100、root-sink = 101(モニタ PUB、v1.9)、psu = 200(Batch の source_id 空間) |

- ch 数・CoBo 数・ポートを**コードに焼き込まない**。すべて TOML とジオメトリ(§4)から来る。
- 設定パースエラーは起動失敗(半端な既定値で走らない — C++ 版 app_config の全リセット方式より厳しくする)。

## 4. ジオメトリ仕様

### 4.1 入力 = TPCReco `.dat`(2 レイアウトとも受理)

- ヘッダ行(順不同、`KEY: 値`): `ANGLES`(3)/ `DIAMOND SIZE`(1)/ `REFERENCE POINT`(2)/
  `DRIFT VELOCITY` / `SAMPLING RATE` / `TRIGGER DELAY` / `DRIFT CAGE ACCEPTANCE`(2)。
  **DRIFT CAGE ACCEPTANCE も含め全部パースして保持**する(rust_reference は取りこぼしている)。
- ストリップ行:
  - **NEW(10 欄)**: `DIR SECTION STRIP COBO ASAD AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS`
  - **LEGACY(7 欄)**: `DIR STRIP AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS`(section/cobo/asad = 0)
  - 判別はトークン数(TPCReco `GeometryTPC.cpp` と同じ)。`AGET_CH` は**信号番号 0–63**。
- AUX 行(5 欄): `NAME COBO ASAD AGET AGET_CH` — パースして Aux として登録(ヒストからは除外、
  波形ビューでは表示)。**AUX の `AGET_CH` も信号番号 0–63**(実 ELITPC .dat の列コメント
  `AGET_ch[0-63]` 準拠、2026-08-12 確定)で、ストリップ行と同じ FPN リオーダを適用する。
- `#` コメント・空行は無視。重複 `(cobo,asad,aget,ch)` は警告 + 先勝ち。
- 実ジオメトリ .dat はリポに入れない。**テストは合成フィクスチャ**(mini 縮小版・架空 2-CoBo 版)のみ。

### 4.2 チャンネルキーと ChannelRole

- ルックアップキーは **`(cobo, asad, aget, raw_ch 0–67)`**。rust_reference の 3 タプル(cobo 欠落)は
  単一 CoBo 前提のショートカットなので継承しない。
- 実装はフラット配列(累積 AsAd オフセット方式の稠密インデックス — TPCReco `Global_raw2normal` と同じ
  考え方)。1088 エントリ程度なので HashMap を使う理由がない。
- 引いた結果は役割を明示する:

```rust
enum ChannelRole {
    Strip { plane: Plane, section: u8, strip: u16 },  // 信号ストリップ
    Fpn { index: u8 },                                 // FPN(1..4)
    Aux { name: ... },                                 // AUX 入力
    Unmapped,                                          // .dat に記載なし(要警告カウント)
}
```

FPN は「ハッシュミス」ではなく明示的な役割として返す(TPCReco の FPN_CH 擬似ストリップと同じ契約。
rust_reference はこの情報を落としている)。`Unmapped` の出現は `info!` 以上でカウント可視化(silent 禁止)。

### 4.3 FPN リオーダ

- FPN = raw {11, 22, 45, 56}(AGET 毎)。信号 0–63 ↔ raw 0–67 の変換は
  rust_reference の 64 要素定数表(`REORDER_FROM_GEOMETRY_TO_GRAW`)を採用し、**パース時に一度だけ**
  適用する(ホットパスは配列引きのみ)。この表は TPCReco `Aget_normal2raw` と全 64 入力で一致することを
  確認済み。両実装(ループ版)との一致ユニットテストを置く。

### 4.4 mm 非依存の原則

- DAQ・モニタのホットパスは**ストリップ番号と time bucket のみ**を使い、mm へ変換しない
  (R5 でオンライン再構成は非スコープ。ヒスト軸も strip/bucket)。
- これにより strip_pitch 規約の不一致(TPCReco: `pad_size×1.5` vs rust_reference: `pad_size×√3/2`)は
  DAQ 層では**判断不要**になる。ヘッダスカラ(ANGLES 等)はメタデータとして保持し、UI の meta(§10.3)と
  JSONL(§9)へ素通しする。将来 mm が要る場合は TPCReco 規約を採る(オフライン整合が正)。

### 4.5 二重実装の一致テスト

ジオメトリパーサは Rust(monitor)と C++(root-sink、C++ 版 tpcdaq のパーサを移植)の 2 実装になる。
双方に「全チャンネル → (role, plane, section, strip) の表を TSV ダンプ」する機能を持たせ、
同一 .dat 入力でのダンプ一致を CI で機械検証する(WS 適合性テストと同じ発想)。

## 5. モニタ・ヒストグラム仕様

### 5.1 集計の持ち主 = root-sink(決定)

- **決定**: R3/R4 の全ヒストグラムは root-sink(C++)が集計する。monitor(Rust)と UI は表示専用。
- 根拠:
  1. **完全性**: モニタ系 PUB/SUB は負荷時に間引かれる。monitor 側集計だと `run<N>_monitor.root` が
     「見えた分だけの部分集計」になる。root-sink はロスレス系で全イベントが通る。
  2. **一致性**: 集計が一つなので「UI で見た形 ≠ monitor.root」の二重管理リスクが構造的に消える
     (C++ 版 §12.7 の同一オブジェクト保証と同じ性質)。
  3. R10 の書き出しが同一プロセスで完結する。
- **条件(C3「保存と可視化の分離」との折り合い)**: root-sink 内で
  - ヒスト fill は builder 出力を処理するスレッドで行う(配列加算のみ、O(items))。
  - スナップショットの直列化と PUB 送信は**専用スレッド + 二重バッファ**で行い、Writer(TTree)
    スレッドとロックを共有しない。
  - 受け入れ時に「スナップショット配信を止めても/倍にしても保存スループットが変わらない」ことを
    計測で確認する(§12-8)。

### 5.2 ヒストグラム定義(全 9 枚 + 飽和率)

すべて run start でリセット、run 中は全イベント積算、run stop で `run<N>_monitor.root` へ(R10)。
「波高」= 各ストリップ波形(生 ADC、減算なし)の時間方向最大値(R4 確定義)。

| id | 名前 | 型 | ビン | 軸 | フィル規則 |
|---|---|---|---|---|---|
| 1–3 | `StripTime{U,V,W}` | TH2D | Nstrip × 512 | x=strip(1..N)、y=bucket | 全サンプルを weight=ADC で加算(R3) |
| 4–6 | `Charge{U,V,W}` | TH1D | 512、[0,4096] 固定 | x=波高 | イベント毎・面内**全ストリップの波高**を各 1 エントリ(R4①) |
| 7–9 | `ChargeMax{U,V,W}` | TH1D | 512、[0,4096] 固定 | x=波高 | イベント毎・面内**最大波高 1 エントリ**(R4②) |

- Nstrip はジオメトリから(mini: U72/V92/W92、ELITPC: U132/V225/W226)。焼き込み禁止。
- x レンジ 0–4096 固定。オートレンジ禁止(飽和天井 4095 が常に見えること — UI 側も同様)。
- 同一ストリップ番号の複数セクションは同一ビンに合算する(制限として明記。セクション別ビューは将来課題)。
- FPN・Aux・Unmapped チャンネルはヒストに入れない(波形ビューには出す)。
- **表示の向き(v1.18)**: UI の 2D 表示(StripTime と Event Display)は
  **縦(Y)= strip、横(X)= time bucket**(TPCReco の慣習)。ワイヤ(§5.3 / §10.2 の
  格子 = strip が遅い軸)と `run<N>_monitor.root` の TH2D 軸定義は**不変** — 転置は
  UI 描画層だけで行う。
- **Event Display のベースライン減算(v1.18、表示専用)**: UI は `0x02 Uvw` の各 strip
  について**先頭 25 cell(0..24)の平均**をベースラインとして全 cell から引いて描く。
  **負値は保持**(0 に切り上げない)。ワイヤは生 ADC のまま・保存系(FPN ペデスタル
  算法 = grawToEventTPC 互換)には一切適用しない。理由: ①オフライン解析は自前で
  ベースラインを再計算する ②「先頭 25 サンプル」は経験則の決め打ちにすぎず、
  データに焼き込むと後段の自由度を奪う(ユーザー裁定 2026-08-16)。
  **Waveform タブは生 ADC のまま**(同日裁定 — 再提案不要)。StripTime(Σ ADC 積算)
  には減算を適用しない。
- **波高の集計単位(v1.9 明確化)**: ジオメトリで Strip に割り付いたチャンネル毎
  (= 物理ストリップ毎)。Charge{U,V,W} は該当チャンネル 1 本につき 1 エントリで、
  同一ストリップ番号の複数セクションは同一ビンへ**別エントリ**として入る。
  そのイベントでサンプルが 1 個以上あったチャンネルのみ波高を計上する(部分読み出しで
  サンプル 0 のチャンネルは飽和率の分母にも入れない)。incomplete イベントは届いた分で
  fill する(全イベント積算 = R3。捨てない)。emit 済みイベントへの遅延フラグメントは
  fill しない(§6.3 の late_fragments カウンタが可視化を担う)。
- **飽和率(採用)**: 面毎に `波高 ≥ 4095 のストリップ数 / 波高を数えたストリップ総数`(run 積算)。
  ヒストではなくカウンタ 2 個 × 3 面。status(§5.3)に載せ UI に % 表示。

### 5.3 スナップショット・status 配信(root-sink PUB)

- ヒストスナップショット: 全 9 枚を `snapshot_hz`(既定 1 Hz)で publish。ビン値は f64 のまま運ぶ
  (ワイヤは §2.3 の map 形式 msgpack。W 面 2D で ~1 MB/s 程度 — 問題ない)。
- status(1 Hz、run 外でも常時): `{ run, state, events_built, events_incomplete, late_fragments,
  pending_events, frames_per_cobo, bytes_written, saturation: {U,V,W}, publish_drops }`。
  `events_built` = emit 済みイベント総数(complete + incomplete)。`publish_drops` =
  built event の間引き数(§5.4 の「モニタ取りこぼし数」とは別 — こちらは送り手側の意図的間引き)。
  `pending_events`(v1.10、R-P2-5)= ビルダ組み上げ中の瞬間値。pending のメモリ上界は
  レート × build_timeout に比例するため常時可視化し、実装は警告閾値(定数 1000)超過で
  warn を一度出す(hard limit はロスレス契約と衝突するので設けない)。
- 最新 built event: `event_publish_hz`(既定 20)を上限に、**最新優先**で publish(全ストリップ生波形
  込み = 波形ビュー R13 と同一ペイロード)。イベント ID(run / event_idx)必携(R9)。
- **ワイヤ形式(v1.9 確定)**: エンベロープは §2.2 と同形
  (`{"Data": [source_id=101, run_number, sequence_number, created_ns, payload]}`)。
  sequence_number は **run リセットなしの単調増加**(§2.2 の 0 リセット規則の例外 —
  モニタリンクの用途はギャップ = ドロップ数の可視化のみ)。payload は map 形式 msgpack
  (`to_vec_named` 相当の自己記述)で、`kind` キーで 3 種を判別する:

  | kind | 内容 |
  |---|---|
  | `"status"` | 上記 status のフィールドをそのまま map で。`state` は文字列、`frames_per_cobo` は `{cobo(10進文字列): u64}`、`saturation` は `{"U": {"saturated": u64, "counted": u64}, "V": …, "W": …}` |
  | `"hist_snapshot"` | `{ kind, run: u32, hists: [ { id: u8(1..9), name: str, nx: u32, ny: u32(1D は 1), bins: Bin } ×9 ] }`。`bins` = f64 LE の生バイト列(長さ nx*ny*8)。2D の添字 = `(strip-1)*512 + bucket`(strip が遅い軸)、1D の添字 = ビン番号 0..511。under/overflow ビンは持たない(ADC 0..4095・bucket 0..511・strip 1..N は構造上レンジ内) |
  | `"built_event"` | `{ kind, run: u32, event_idx: u32, complete: bool, fragments: [ §2.4 の Fragment positional array, … ] }`(フラグメント順 = (cobo,asad) 昇順 = §6.3 v1.3) |

### 5.4 monitor(Rust)の責務

- root-sink PUB を購読 → WS メッセージ(§10)へ変換して全クライアントへ配る。それだけ。
- built event(電子回路空間)からの表示用変換は monitor が行う(可視化 CPU を保存プロセスから隔離):
  - UVW グリッド(面毎 strip×bucket)への変換 — ジオメトリ適用はここ。
  - 波形ビュー用の (cobo,asad) 毎 68ch×512 dense 行列。
- 表示間隔・freeze は**クライアント側**(R9 確定義)。freeze は表示のみ停止し、DAQ・積算・保存に一切
  影響しない。UI 上も run Stop と明確に区別する。
- PUB のギャップ(sequence_number)を数え、UI に「モニタ取りこぼし数」を表示(silent 禁止)。

## 6. root-sink 仕様(C++、delila-rs root_sink 流用)

### 6.1 手術範囲(調査で確定)

| 区分 | 対象 |
|---|---|
| **そのまま流用** | スレッド骨格(Receiver/Builder/Writer/Publisher + in-band 制御マーカー)、`Channel<T>`(有界化)、`SeqReorder<T>`、`RunState`(全ソース EOS で close)、Recorder ライフサイクル(inprogress → rename、AutoSave 30 s、MaxTreeSize + 名前比較 rollover)、ROOT 作法一式(`SetBatch` / `AddDirectory(kFALSE)` / `SetScanGlobalDir(kFALSE)` / `EnableThreadSafety` / 圧縮 505 = ZSTD-5)、TH1/TH2 生成・登録機構、純ヘッダ + g++ 単体テスト + リプレイ publisher + 順序非依存比較という試験方式 |
| **改造** | ZMQ 取り込みの **SUB → PULL ロスレス反転**(§6.2)、ワイヤデコードを TPC スキーマ(§2.2/2.4)へ、ファイル命名(§6.5) |
| **新規** | **eventIdx イベントビルダ**(§6.3 — eb_core の timestamp Sorter は TPC には使えない: ソース毎 FIFO も eventIdx キーも実在しない。PROPOSAL の想定より流用度は低い、§14-1)、**ヒストのファイル書き出し**(現 root_sink はヒストを一切ファイルに書かない — THttpServer ライブのみ)、スナップショット/status/built-event の PUB、GDataFrame TTree フィル(C++ 版 tpcdaq RootWriter から移植)、ジオメトリ .dat パーサ(C++ 版 tpcdaq から移植) |
| **削除** | Δt モニタ(CoincidenceMatcher/PositionMatcher)、hist_config.hpp の実験固有 DSL(TPC は固定 9 枚で足りる — KISS)、THttpServer(UI は WS 経由に一本化。ヒスト描画は UI 側 JSROOT — §11。骨格に残るためデバッグ用再有効化は容易)、`--operator` HTTP fetch(run 境界のブロッキング副作業は背圧化後は許されない) |

### 6.2 ロスレス化チェックリスト(実装ユニットの受け入れ項目)

1. `ZMQ_SUB` → `ZMQ_PULL`、SUBSCRIBE 削除。sink 側 bind。
2. RCVHWM を**有限**に(HWM=0 のままでは背圧が発生しない — ここが要)。
3. 内部 Channel(受信→ビルダ→Writer)を有界化(push は cap>0 でブロックする実装が既にある)。
4. モニタ tee(Publisher 行き)だけは有界 + 落とし可のまま(背圧に参加させない)。ドロップはカウント。
5. sequence_number 連続性チェックを追加(現状は明示的に skip している)。ギャップ = Error。
   **v1.10 拡張(R-P2-1)**: run 中の run_number 食い違い(Data/EOS とも)も同格の
   **fatal(exit 6)**。旧「カウンタのみ」は混在 run が完成 run 名のファイルに化ける経路を
   残していた。idle 中の stale EOS(計数して無視)と期待外 source_id(計数して混ぜない)は
   従来どおり fatal にしない。
6. malformed バッチ: warn+skip → **Error 停止**(ロスレス契約では黙って捨てない)。
7. closed channel への push が silent discard になっている箇所を明示化(assert)。
8. run 境界(RunOpen/RunClose)でのブロッキング外部 IO 禁止。

### 6.3 イベントビルダ(新規、eventIdx ベース)

- 期待フラグメント集合 = 設定 + ジオメトリから导出した `{(cobo, asad)}` の集合(mini: {(0,0)}、
  ELITPC: **{(0,0),(0,1),(0,2),(0,3)}**(v1.7 実データ確認 — 1 論理 CoBo × 4 AsAd)。
  .dat の実配置に従う)。
- キー = `(run_number, event_idx)`。全期待フラグメント到達で complete、`build_timeout_ms`
  (壁時計、既定 1000 ms)超過で **incomplete フラグ付きで emit**(捨てない)。
- emit は event_idx 昇順。**TTree は 1 エントリ = 1 ビルド済みイベント(PEventTPC、v1.8)** —
  「到着順連結で順序が狂う」現行 GET 問題(PROPOSAL Q3)の構造的回避はここで実現される。
- **イベント内のフラグメント順は (cobo, asad) 昇順で決定的にする**(v1.3。到着順は run 毎に
  揺れるため、§12-4 の 2 ソースビルド一致・TTree 比較が順序で偽陰性にならないように)。
- **遅延到着(emit 後)の扱い(v1.8 改訂)**: PEventTPC は eventId 毎に 1 エントリで書き切る
  (grawToEventTPC の eventId 重複排除と同じ意味論)ため、emit 済みイベントへの遅延
  フラグメントは **TTree に書かず `late_fragments` としてカウント + warn 可視化**。
  データ自体は生 graw(ロスレス保存系)に必ず在る — run.root は v1.8 から「解析用の変換出力」
  であり(FPN 落とし・窓切り・ペデスタル減算を含む)、ロスレス保証の担い手ではない。
  (旧 GDataFrame テストモードに関する注記は v1.17 のモード撤去により削除。)
- **重複フラグメント(同一イベント内の同一 (cobo,asad)、v1.10 明文化 — R-P2-4)**:
  `duplicate_fragments` で計数のうえ、**イベントに全部載せて全部 fill する(加算)** —
  「捨てない」を優先する。実機で重複は発生しない想定であり、PEventTPC の chargeMap /
  モニタヒストが二重加算になることは計数で可視。実機オラクルとの差異が観測されたら再訪。
- 単一 CoBo 構成では実質素通し(期待集合が 1 要素)。多ソースの検証は graw_replay ×2 並走(§12-4)。

### 6.4 TTree(PEventTPC / TPCReco 互換 — v1.8 全面改訂)

**決定(v1.8、ユーザー裁定)**: run.root のイベント形式 = **TPCReco `grawToEventTPC` の出力と
同一**。オフライン解析は TPCReco であり(mini/ELITPC 共通)、変換ステップなしで我々の出力を
そのまま解析に使えることが価値。v1.7 までの GDataFrame 出力は瑕疵と裁定。

- **ツリー/ブランチ(TPCReco EventSourceROOT がハード期待する形)**: ツリー名 `TPCData`
  (タイトル空文字)、単一ブランチ `Branch("Event", &pevent, 128000, 2)`(splitlevel 2)。
  **1 エントリ = 1 イベント(全 AsAd ビルド済み)**。出自 = `ConvertGrawFile.cpp:40-45` +
  `EventSourceROOT.cpp:25/74`(HIGS2026_online、2026-08-13 実ソース確認)。
- **クラス**: `PEventTPC` + `eventraw::EventInfo`(TPCReco 由来)。**TPCReco はライセンス
  無指定(= all rights reserved)のためコピーをリポにコミットしない** — 017 の .ice と同じ
  **ビルド時参照**方式: `TPCDAQ_TPCRECO_DIR`(既定 `reference/TPCReco/TPCReco-HIGS2026_online`)
  のヘッダを include し、我々の LinkDef(pragma: PEventTPC+ / eventraw::EventInfo+ /
  同::global_properties+ / nestedclasses)で rootcling 辞書を生成。依存は 6 ファイルで閉じる
  (PEventTPC.h/.cpp・EventInfo.h(.cpp は要否実装時判定)・StripTPC.h(GeometryTPC は前方宣言
  のみ)・CoBoClock.h(boost ヘッダのみ))。Warsaw の再配布許諾が得られたら third_party/tpcreco
  へ昇格。**streamer checksum 一致を受け入れテストで固定**(実測: PEventTPC v1 0xf71c32cf /
  eventraw::EventInfo v1 0xfea093e4 / global_properties v1 0x49e6428c — コピー元は
  HIGS2026_online 固定。myChargeArray の出入りした他スナップショットと混ぜると割れる)。
- **chargeMap キー** = `{dir(U=0/V=1/W=2), section(0="-"/1=A/2=B), number(1 始まり), cell(0..511)}`、
  値は `AddValByStrip` の **`+=` 加算**(tuple 版 setter を使う — shared_ptr<StripTPC> 版と同値)。
- **myChargeArray ブランチは既定で無効**(`SetBranchStatus("TPCData.myChargeArray*", false)` —
  実運用既定 `disabledBranches=["TPCData.myChargeArray*"]` と同一。float[3][3][256][512] ≈
  4.7 MB/イベントの節約。streamer には残るのでクラス互換に影響なし)。
- **充填意味論(`EventSourceGRAW::fillEventFromFrame` = HIGS2026_online の実装と同一)**:
  - **normal ch(0..63)のみ**走査(`Aget_normal2raw` で FPN リオーダ)。FPN・非 strip ch は
    捨てる(ロスレスは生 graw が担う — 絶対ルールと矛盾しない: ここは「変換出力」)。
  - strip = ジオメトリ lookup(geo.hpp、018)。`AddValByStrip(strip, cell, 値)` で
    chargeMap(key = tuple<int,int,int,int>)へ。
  - **signal 窓**: cell ∈ [minSignalCell, maxSignalCell](既定 5..506)以外は捨てる。
  - **ペデスタル減算(既定 ON)**: TPCReco `PedestalCalculator(GRAW)` と同一算法 —
    ①FPN 4ch を cell 毎に平均(pedestal 窓 5..25 と signal 窓それぞれ)
    ②normal ch の pedestal 窓で `raw − FPN平均` をチャンネル毎に平均(= オフセット、
    TProfile 256 ビン整数中心と同値の純算術)
    ③補正 = オフセット + FPN_ave_signal[cell]、格納値 = raw − 補正。
    per (cobo,asad) フレーム毎にリセット・再計算(イベント内で完結、run 状態なし)。
  - **EventInfo**: eventId = eventIdx、timestamp = eventTime、
    runId = **run 開始 TS の `%Y%m%d%H%M%S`**(TPCReco `RunIdParser` と同じ導出 —
    v1.7 §6.4「run_number を載せない」の代替がここに自然に存在する)、
    pedestalSubtracted = 減算フラグ。
    **実装注記(020)**: ワイヤに run 開始時刻が無いため、runId は root-sink が run を
    開いた瞬間のローカル時刻から生成する。graw ファイル名の TS(graw-writer が独立に
    採るローカル時刻)と数秒ずれ得る — 対応付けの正はログブック(§9)。P4 で controller が
    Start コマンドに正式な run TS を載せて全コンポーネントで統一する(016 の設計入力)。
- **設定**(config `[root_sink]` → CLI): `pedestal_remove`(既定 true)/
  `min_pedestal_cell` 5 / `max_pedestal_cell` 25 / `min_signal_cell` 5 /
  `max_signal_cell` 506(TPCReco `allowedOptions.json` の既定と同値)。
- 圧縮は **101(ZLIB-1)を既定とし設定可能**(v1.5。実機 grawToEventTPC 出力が ZLIB level 1
  であることを 2026 実ファイルで直接確認済み — WARSAW_PLAN §4)。
- **GDataFrame の扱い(v1.8 降格 → v1.17 全撤去)**: **ユーザー裁定(2026-08-15):
  GDataFrame は graw2root(GET 付属の別ツール)の形式であり、我々のチェーンに全く不要。**
  v1.8 が定めた削除条件「PEventTPC の同 run 実データオラクルが閉じたら」は 021
  (`compared 3852 events, 0 differences`)で**成立済み**。よって:
  ①中間表現(Fragment → GDataFrame → PEventTPC)を撤去し、**Filler は Fragment を直接読む**
  (等価性の担保は中間表現の共有ではなく **§12-3 の内容一致オラクル**が担う)
  ②`--format gdataframe` テスト専用モードと専用回帰を撤去
  ③不要化した third_party/get の GDataFrame 系クラスを整理。実装 = TODO/054。

### 6.5 ファイル命名・ライフサイクル

| 出力 | 命名 | 備考 |
|---|---|---|
| イベント TTree | `run{run:04}.root`(rollover 時 `_0001` 付加) | 書き込み中は `run_inprogress_<unixtime>.root`、finalize で rename。異常終了は inprogress のまま残す(完全 run に化けない) |
| モニタヒスト | `run{run:04}_monitor.root` | 全ソース EOS 後に書き出し。**EOS から 10 秒以内**(R10「速やかに」の数値化) |
| 生 graw | `CoBo{K}_AsAd{A}_{TS}_{idx:04}.graw` | **実機 DataRouter 命名に完全一致**(§7)。run 番号は含まれない — 対応はディレクトリとログブックが持つ |
| 非 AsAd 制御フレーム | `ctrl/CoBo{K}_{TS}_{idx:04}.graw` | v1.2。run ディレクトリ配下のサブディレクトリ(オフラインの glob からは見えない)。§7 |

出力先: `<output_root>/run{run:04}/` に run 毎ディレクトリを切り、全出力をまとめる。

- run 番号 ↔ graw 実機命名の対応は **run ディレクトリと JSONL ログブック(§9 のファイル実績記録)が
  持つ**(graw 名に run 番号が無いことの補償。ログ・UI・運用の管理単位は常に run 番号)。
- **0 イベントの run は ROOT ファイルを作らない**(011 レビューで明文化 — 遅延オープン。
  「run はあったがイベント 0」の実績はログブックとカウンタが持つ。空 TTree ファイルを置かない)。
- イベント TTree は**全 CoBo/AsAd マージ済みの run 毎単一ファイルが理想形**(2026-08-13 ユーザー確認。
  rollover はサイズ保護のみ)。オフライン側が ROOT にも graw 同様の命名を要求した場合は
  **シンボリックリンクで対処**する(リネーム・複製はしない。実装物ではなく運用手順)。

### 6.6 third_party 隔離

GDataFrame 系(CeCILL)は `third_party/get/` に置き、ライセンス文と出自(GET/CoBoFrameViewer 由来)を
README で明示。root-sink のビルドは tools/ 内で完結し、Rust 側に一切リンクしない(境界は ZMQ のみ)。

**TPCReco 系(v1.8)**: ライセンス無指定のため**コミットしない**(§6.4 — ビルド時参照
`TPCDAQ_TPCRECO_DIR`)。Warsaw の再配布許諾が得られたら `third_party/tpcreco/` へ
昇格し、出典 URL + コミットハッシュ + 改変内容を NOTICE として付す。

## 7. graw-writer 仕様

- receiver からの RawFrames を **(cobo, asad) 毎のファイル**へバイトそのまま append(v1.1 訂正)。
  1 フレーム = 1 AsAd 分(§2.4)なので、振り分けはフレームヘッダの asadIdx を読むだけ。
  リシリアライズ・変換なし(AsAd 毎連結 = 入力を asadIdx で分別した列と同一 =
  grawToEventTPC(TPCReco)/ graw2root(GET)/ CoBoFrameViewer で読めるファイル)。
- **振り分け対象は frameType 1/2 のフレームのみ**(v1.2)。asadIdx の読み出しは decode モジュールに
  集約した `peek_asad`(**frameType 1/2 かつヘッダ 28 B 以上のときだけ Some**)を使う — frameType を
  見ずにオフセット 27 を読むと、28 B を超える制御フレームが来たとき**誤った AsAd ファイルに混入**する。
- **非 AsAd フレーム(peek_asad = None。例: 実 2025 run 先頭の frameType 7 トポロジー 12 B)は
  `run{run:04}/ctrl/CoBo{K}_{TS}_{idx:04}.graw` へバイトそのまま保全**し、`ctrl_frames` カウンタ +
  info ログで可視化(v1.2 ユーザー決定)。**Error 状態にはしない**(run 先頭に毎回来る正常な制御
  フレームで Error に落ちない。Error は write 失敗・seq ギャップ・EOS 前 run 変更のみ)。
  実機 FrameStorage は同フレームを警告して捨てる(`"Dumping frame"`)が、tpcdaq-rs は絶対ルール
  (意図的ドロップ禁止)を優先する。ctrl/ はサブディレクトリなのでオフラインの `CoBo*` glob には
  見えず、per-AsAd ファイルは実機と完全一致のまま。TS・idx・ローテーション規則は per-AsAd と同一。
- AsAd 数は設定にもコードにも焼き込まない。観測した (cobo, asad) 毎にファイルを遅延作成
  (mini = 1 ファイル、ELITPC = **1 CoBo × 4 AsAd = 4 ファイル**が自然に出る —
  実 2022/2026 データで確認、v1.7)。
- **命名 = 実機 DataRouter 完全一致**(v1.1): `CoBo{K}_AsAd{A}_{TS}_{idx:04}.graw`。
  TS = 当該 (cobo, asad) ストリームの最初のファイル作成時刻、**localtime** の ISO 8601 拡張 +
  ミリ秒 3 桁(例 `2022-04-12T08:03:44.531` — コロン入り、Linux 前提)。K/A はゼロ埋めなし 10 進。
  出自 = GET `GetBench/src/get/daq/FrameStorage.cpp`(`"CoBo"<<K<<"_AsAd"<<A<<'_'<<TS<<'_'
  <<setw(4)<<idx<<".graw"`)+ `utl::buildTimeStamp()`(localtime + %03d ms — 2026-08-13 実ソース確認)。
- **ローテーションは TS 据え置き・idx++ のみ**(FrameStorage `createNewFile(newTimeStamp=false)` と
  同一挙動)。新 run では新 TS + idx=0000。AsAd 間で TS が ms 単位でずれるのは実機も同じ
  (実データ例: AsAd0–3 = .531/.533/.536/.540)で、TPCReco `RunIdParser` はこれを許容する。
- ファイルハンドルは run 中開きっぱなし(per-frame open/close 禁止 — 旧 DataBloc の失敗)。
  flush は 1 秒毎、fsync はローテーションと close 時。
- ローテーション(**v1.7 実機一致に訂正**): フレームを書いた**後**、ファイルサイズが
  `max_file_size(既定 1 GiB = 2^30 B)` を **strict に超えていたら**次 seq のファイルを
  即時オープン(FrameStorage.cpp:190-197 `write → tellp() > 1024 MiB → createNewFile` と同一)。
  **境界を跨いだフレームは現ファイルに残る**(実データ確認: 各 _0000 = 3852 フレーム =
  1,073,875,968 B > 2^30。v1.6 までの「書く前判定」は実機と 1 フレームずれる誤り)。
  **フレームはファイル間で分割しない**。巨大単発フレームもそのまま丸ごと書かれてから
  ローテーション。直後に run が終わると空の次ファイルが残る(実機と同一挙動)。
- 書き込み失敗は Error 状態 + カウント(silent 禁止)。run 中のディスクフルは §1.4-3 の異常停止経路。
- run バウンダリ: Batch の run_number が変わったら新ファイル群、EOS で flush+close。

## 8. run 制御仕様

### 8.1 controller

- **REST API(axum)** — UI の唯一の操作面。主要エンドポイント:
  - `GET /api/status` — 全コンポーネント状態 + run 情報 + ECC 状態(delila-rs SystemStatus 相当)
  - `POST /api/control/acquire {operator, passphrase}` → `{token}` / `POST /api/control/release`
    — 操作権は常に 1 クライアント。取得は常に横取り可(C++ 版 CommandRouter 方式)、横取りは監査ログへ
  - `POST /api/run/start {token, comment?}` / `POST /api/run/stop {token}` — §1.3 のシーケンスを実行
  - `POST /api/ecc/{describe|prepare|configure|start|stop|breakup|reset} {token}` — 段階操作
    (R6: GET controller と同じ操作感)
  - `GET /api/logbook?since_seq=N` / `POST /api/logbook/comment {author, text}`(R11)
  - 状態変更系はすべて token 必須 + 監査ログ(audit レコード)。閲覧系は認証なし(二層アクセス制御)。
    **例外(v1.10 明文化)**: `POST /api/logbook/comment` は **token 不要** — DAQ 状態を変えない
    記録系であり、R11(シフト全員が書けるログブック)を操作権 1 名モデルより優先する。
    author は自己申告(レコードに残る)。
- run 番号: `<output_root>/tpcdaq_state.json` に `next_run` を永続化(controller 単一書き手)。
  手動設定は **`POST /api/run/next {token, next_run}`**(v1.10 で形を確定): run 実行中は拒否、
  正整数のみ、audit 記録。次の run/start から有効。
- Web UI(Angular)静的ファイルは controller が配信(delila-rs と同じ「Rust だけでデプロイ」方式)。

### 8.2 ecc-bridge(C++)

- C++ 版 `tpcdaq::control::EccController`(Ice pimpl 済み)を JSON REQ/REP サーバに被せただけの
  薄いプロセス。状態: `Off/Idle/Described/Prepared/Ready/Running/Paused/Unknown`。
- リクエスト `{"action": "configure", "config_id": "...", "links": [{"sender": "CoBo[0]",
  "router_ip": "...", "router_port": 46005, "type": "TCP"}, ...]}` → `{"ok", "state", "error"}`。
  `config_id` は**元よりアクション毎** — controller が §3.1 の設定から**当該相の id** を渡す
  (v1.13。describe/prepare/configure で別名の実運用に対応。ecc-bridge 側は変更なし)。
- **status 応答の `ecc_error`(v1.14 追加、実装 = TODO/043)**: `{"action":"status"}` の応答に
  **GET の error フィールド**(`NO_ERR`/`WHEN_DESCRIBE`/`WHEN_PREPARE`/…)を `ecc_error` として
  追加する。既存の `error` は輸送・ブリッジ層のエラー文字列のままで意味を変えない。
  controller `/api/status` はこれを素通しで載せる(UI 表示は P4)。
  根拠: 実 ECC は失敗後に state と別のエラーフラグを抱える(例: `IDLE / WHEN_DESCRIBE`)が、
  現行 status では UI から不可視(041 D-1 実測)。
- DataLinkSet XML は links から生成(CoBo 毎に DataLink 1 本)。**実機の罠を仕様として固定**:
  DataSender id は `CoBo[k]` 形式、flowType は大文字 `TCP`、Ice encoding 1.1 固定。
  router_port は receiver が**実際に bind したポート**を controller が Arm 応答から取って渡す。
- 例外は全部 Result 化(never throw)。ECC 不達は `state: "Unknown"`。

### 8.3 検証

fake-ECC servant(C++ 版のテストハーネス)相手の e2e を CI に置く(§12-7)。listen-before-start の
負性テスト(listen 前 start → "Could not establish data link")を含む。実 ECC は実験使用中と同一版
(20190315_patched、`reference/` に確保済み)でコンテナ検証 → 実機(P5)。

## 9. JSONL ログブック仕様(Q1 確定)

### 9.1 ファイルと書き手

- パス: `<output_root>/logbook.jsonl`(単一ファイル、追記のみ。行量は高々 数百行/日 なので
  ローテーション不要 — 必要になったら年次で切る)。
- **書き手は controller ただ一人**。他コンポーネント(psu、将来の投稿者)は §2.3 の LogPost
  (PUSH/PULL)で投稿し、UI は REST で投稿する。controller が `ts`(RFC3339、ミリ秒、オフセット付き)
  と `seq`(単調増加 u64)を付与して 1 行 = 1 write(2) で追記、行単位 flush。
- クラッシュ耐性: 末尾行の破損は起こり得る(唯一の許容)— リーダは「最終行のみ parse 失敗を許容し
  警告表示」と定める。

### 9.2 レコードスキーマ(共通: `ts`, `seq`, `type`, `actor`)

| type | 追加フィールド |
|---|---|
| `run_start` | `run`, `config_id`(3 相同値ならその文字列。非同値なら configure 相の id)、**`config_ids`(v1.13 追加、非同値時のみ**: `{describe, prepare, configure}`。同値時は省略 — nullable 規律)、`geometry: {path, sha256}`, `cobos: [{id, listen}]`, `operator`, `comment`, `expected_fragments`(期待 (cobo,asad) 集合) |
| `run_stop` | `run`, `duration_s`, `ok: bool`, `reason`(**"normal" / "error:eos-timeout" / "abort:<原因>"** — abort は停止開始時点の起因。EOS の顛末は次の 2 フィールドが持ち、reason には合成しない)、**`forced_eos: bool`**(EOS を receiver `Stop` で注入したか。**実機 TCP flow では通常 true**(§1.3 v1.12 — stop はデータリンクを close しない)。EOF 由来の自然 EOS のみ false)、**`eos_closed: bool`**(EOS がチェーンを流れ切ったことを観測できたか。**false は異常の印**であり、reason が abort でも eos-timeout の事実はここで読める)(v1.12 追加。**v1.14 注記**: 実機 TCP flow では `forced_eos:true` が常態なので、**`forced_eos:false` は「stop 前にリンクが死んだ」ことの強い印** — CoBo 突然死は OS の正常 FIN により自然 EOF として normal クローズし、他に痕跡が残らない(041 D-2 実測)。リーダ・UI は `eos_closed:false` と並んで `forced_eos:false` にも注意を向けること)、`counters: {events_built, events_incomplete, late_fragments, frames: {cobo: n}, overflow_frames, malformed}`(**v1.10: 各項目は nullable — null = 「その時点で取得不能」であり 0 と混同しない**。root-sink が REP を持たない間、events_built/events_incomplete/late_fragments は null。取れる分は GetStatus 実測)、`files: [{path, bytes}]`(graw 群 + root + monitor.root 実績) |
| `audit` | `action`(REST エンドポイント名), `params`(要約), `operator`, `ok`, `error` |
| `comment` | `author`, `text`(自由記述、R11) |
| `psu` | `device`, `channel`, `event`("TRIP"/"ON"/"OFF"/"VSET"/...), `values: {vmon, imon, vset}`(P6 で詳細化) |

例:

```json
{"ts":"2026-08-12T15:04:05.123+03:00","seq":1042,"type":"run_start","actor":"controller","run":57,"config_id":"default","geometry":{"path":"config/geometry_mini_eTPC.dat","sha256":"ab12..."},"cobos":[{"id":0,"listen":"0.0.0.0:46005"}],"operator":"aogaki","comment":"gas test","expected_fragments":[[0,0]]}
{"ts":"2026-08-12T16:10:00.001+03:00","seq":1043,"type":"comment","actor":"ui","author":"aogaki","text":"beam tuned, 4 Hz"}
```

- UI の「ログ」タブは `GET /api/logbook` でこのタイムラインをそのまま表示 + comment 追記(R11)。
- run メタデータの二重管理はしない(JSONL が唯一の run 台帳。ROOT/graw ファイル自体には run 番号は
  ファイル名でのみ載る)。

## 10. WS プロトコル仕様

### 10.1 バイナリ枠組み(C++ 版継承)

全バイナリメッセージ共通 13 バイトヘッダ(リトルエンディアン):

| off | size | 内容 |
|---|---|---|
| 0–1 | 2 | マジック `'T' 'P'` |
| 2 | 1 | msgType |
| 3 | 1 | version = 2(型再定義のため版上げ) |
| 4 | 1 | flags(bit0 = incomplete event) |
| 5 | 4 | u32 runNumber |
| 9 | 4 | u32 eventNumber(ヒストでは 0) |

### 10.2 バイナリメッセージ型

| type | 名前 | ボディ |
|---|---|---|
| 0x02 | `Uvw` | `u8 plane(0=U,1=V,2=W)`, `u16 nStrips`, `u16 nBuckets`, `u16 ADC × nStrips×nBuckets`(strip-major: `idx=(strip-1)*nBuckets+bucket`) |
| 0x03 | `Waveforms` | `u8 cobo`, `u8 asad`, `u8 nAget(=4)`, `u8 nCh(=68)`, `u16 nBuckets`, `u16 ADC × nAget×nCh×nBuckets`(aget-major、raw ch 順、FPN 込み・減算なし — R13) |
| 0x10 | `Histo1d` | `u16 id`, `u32 nbins`, `f32 xmin`, `f32 xmax`, `f32 × nbins` |
| 0x11 | `Histo2d` | `u16 id`, `u16 nx`, `u16 ny`, `f32 xmin,xmax,ymin,ymax`, `f32 × nx×ny`(iy 外側 row-major) |

- 旧 0x01 Event(3D 点群)は**廃止**(R5 非スコープ)。
- ヒストの id は §5.2 の表と一致(1–3 が 2D、4–9 が 1D)。ビン値は f32 に落とす(表示専用。正値は
  monitor.root 側にある)。

### 10.3 JSON テキストメッセージ

| type | 方向 | 内容 |
|---|---|---|
| `meta` | S→C(接続時・run 変化時) | `{nBuckets, planes: {U,V,W}, geometry, anglesDeg, detector, cobos, run}` |
| `status` | S→C(1 Hz) | §5.3 の status + `{monitorGaps, clients, wsDropped}` |
| `run` | S→C(遷移時) | `{state, run, ts}` |
| `subscribe` | C→S | `{"streams": ["uvw","waveforms","histos","status"]}` — 既定は waveforms **以外** ON(帯域制御。波形ビューを開いたクライアントだけが 0x03 を受ける) |

- 操作(cmd/ack)は WS に**載せない**(C++ 版からの変更)。操作は controller REST(§8.1)に一本化。
- サーバの送信キューは live(0x02/0x03/0x10/0x11)= drop-oldest + カウント、JSON 制御 = reliable
  (C++ 版 SendQueue の方針を今度は実配線する)。

### 10.4 クロス言語適合性テスト(方式踏襲)

1. **生成器**(Rust bin `tools/ws_proto_sample`): 本番エンコーダで全メッセージ型を既知値で
   エンコードし、`u32 長さ + ペイロード` の連結ストリームをファイルへ書く。
2. **検証器**(TypeScript、Angular UI の**本番デコーダ**を import するテスト): 同ファイルを分解・
   デコードし、既知値と突き合わせ(float は ε=1e-5)。
3. CI 配線: cargo test 内で生成 → node/vitest で検証(フィクスチャは毎回再生成、コミットしない —
   陳腐化が構造的に起きない C++ 版 ctest FIXTURES 方式)。
4. 各言語側の独立レイアウトテスト(Rust: バイトオフセット assert / TS: デコーダ単体)も併置。
5. ライブ経路(実 WS 接続で meta→uvw→histo→status を受けて機械検証)は P3 で probe を用意。

## 11. Web UI(範囲宣言のみ — 詳細は P3 起票時)

Angular + Angular Material(delila-rs operator UI と同一スタック)。ビュー:
**Run 制御**(R6、token 取得 UI 込み)/ **モニタ**(9 ヒスト、R9 イベント表示: interval・freeze・
イベント ID 常時表示)/ **波形ビュー**(R13、面/AsAd/AGET 単位、クライアント側間引き)/
**ログブック**(R11、タイムライン + コメント追記)/ **Power**(P6)。

**描画スタック(2026-08-12 決定)**:
- **9 ヒストグラムの描画 = JSROOT**。§10.2 の WS バイナリスナップショットを受けて、クライアント側で
  `createHistogram` により TH1D/TH2D 相当を組み立てて描く(colz・log 切替・軸ズーム・stats box が
  ROOT の作法のまま使え、オンライン表示と `run<N>_monitor.root` のオフライン表示が同じ描画系になる —
  §5.1 の集計一元化と対をなす)。**ワイヤに ROOT シリアライズ形式は載せない**(THttpServer も復活
  させない — root_sink 骨格に残るためデバッグ用の再有効化は容易だが、既定は削除のまま)。
  JSROOT は monitor ページで遅延ロードし、初期バンドルを太らせない。
- **その他の可視化(波形ビュー・レート/PSU トレンド・status)= ECharts**。波形ビューの独自
  インタラクション(重ね描き/グリッド、チャンネル選択、間引き)は ECharts/自前 canvas が適する。
- P3 起票時の確認項目: ①1 Hz redraw でのズーム状態保持(painter `updateObject` + redraw)、
  ②JSROOT ダークモードの Grafana 風ページへの馴染み、③ライセンス最終確認(MIT のはず)。

デザイン規律: Atlassian Design 準拠、モニタは Grafana 風ダーク。破壊的操作は確認パターン。
freeze は表示のみで、run Stop と視覚的に混同させないこと(§5.4)。

## 12. 受け入れ基準(数値確定)

| # | 項目 | 基準 |
|---|---|---|
| 1 | デコーダオラクル | 実 2025 run graw(ローカル、`TPCDAQ_REAL_GRAW` 環境変数)で **events=108 / items=15,040,512 / malformed=0**。実 ELITPC graw(`TPCDAQ_REAL_GRAW_DIR`、2022/2026 各 4 ファイル、v1.7)で **各ファイル frames=3852 / items=536,444,928 / malformed=0 / unsupported=0 / eventIdx 0..=3851 連続 / eventTime 単調**。CI は合成フィクスチャで frameType 1/2 両方 green |
| 2 | graw バイト一致 | frameType 1/2 を asadIdx で分別した列 = per-AsAd 出力、残り全フレームの列 = ctrl/ 出力、**全出力の合計 = 入力の完全ロスレス分割**(v1.2)。mini 実 graw オラクル: AsAd ファイル 30,108,672 B + ctrl 12 B(frameType 7 ×1)= 30,108,684 B。ローテーション跨ぎも連結一致。ELITPC 実ファイル(1,073,875,968 B > 2^30)を既定 max でリプレイすると **_0000 が入力と完全バイト一致 + 空 _0001**(ローテーション境界の実機一致、v1.7) |
| 3 | TTree 互換 | **v1.8: PEventTPC 互換** — ①構造一致: 実機 grawToEventTPC 出力(2026 実ファイル)とツリー名/ブランチ/クラス streamer バージョン/圧縮が一致 ②値一致(単体): 既知入力 → chargeMap 期待値(strip 射影・signal 窓・ペデスタル算法の手計算オラクル)③値一致(実データ): 同一 run の graw 4 本組と grawToEventTPC 変換済み .root のペアで全イベント全 key の値一致(env-gated)— **2026-08-14 達成: `compared 3852 events, 0 differences`**(TODO/021、tests/elitpc_pevent_e2e.rs)。~~旧 GDataFrame 比較の維持~~ **v1.17 で撤去**(GDataFrame は graw2root の形式で不要 — ユーザー裁定。§6.4) |
| 4 | 2 ソースビルド | graw_replay ×2 並走(異なる CoBo id を模す)→ 全イベント complete、eventIdx 昇順、incomplete=0、CoBo 毎フレーム数一致 |
| 5 | 連続負荷 | **v1.15 で二段化(ユーザー裁定 2026-08-15 — ソフト単体の 24 h は過剰、フル 24 h はハード込みでこそ意味がある)**: **(a) 自宅ソフト soak = 一晩(≥ 8 h、既定 12 h)のトレンド駆動** — 100 Hz 相当ペース(mini ≈ 28 MB/s)のループリプレイ、保存系 drop 0(全カウンタ 0: overflow / gap / malformed / late)、各プロセスの RSS/fd/全カウンタを定期サンプリングし**後半半分で単調増加なし**(上昇トレンドが出たら延長 or 修正)。ディスク節約のため「書いて検証して消す」ハーネス可。**(b) フル 24 h(100 Hz 相当連続)は ELI-NP 実機テストでハード込みで実施**(P5 前段 — §13-7 の現地項目に併合。ソフト soak は実機時間をハード起因の問題だけに純化するためのフィルタという位置づけ) |
| 6 | 瞬発負荷 | **v1.16 で定義を明示レート形に改訂**: **ペーシング付き ≥3× 目標レート(mini ≈ 672 Mbps / 84 MB/s)で 10 分、保存系 drop 0**(バッファ設計の証明)。旧「ペーシングなし全速」は loopback で **216× 目標(6.06 GB/s)** に達し物理的対応物が無い(031 実測)。なおその 216× 条件で receiver の never-stop + counted drop(§1.4)が設計どおり働き silent でないことは実測確認済み — これは §12-6 の合否とは別の設計検証として記録 |
| 7 | run 制御 e2e | fake-ECC 相手に describe→…→start→データ→stop→JSONL 記録まで全通し green。listen-before-start の負性テスト含む |
| 8 | モニタ非干渉 | スナップショット配信を 0 Hz/2 倍にしても保存スループット変化が測定誤差内(±2%)。freeze 中も events カウンタが進む。**run 境界跨ぎ(連続 2 run)のスループットも測定し、境界での intake 停滞が無いことを確認**(v1.10、R-P2-14 = §6.2-8 の定量化) |
| 9 | R10 期限 | 最終 EOS から **10 秒以内**に `run<N>_monitor.root` が完成 |
| 10 | WS 適合性 | §10.4 の全メッセージ型 green |
| 11 | JSONL 耐性 | run 中 kill -9 後、最終行以外の全行が parse 可能。再起動後 next_run が重複しない |
| 12 | ジオメトリ | mini/ELITPC/合成 2-CoBo .dat のロード + Rust/C++ ダンプ一致(§4.5)。FPN 表の両参照実装一致 |
| 13 | receiver 余分接続(v1.12) | 現接続でデータ流通中に余分な接続を張っても、現接続のフレーム列はバイト一致で無影響 + 余分接続は即 close + `extra_connections` 加算 + warn 1 回。0 バイト接続の終了で EOS が出ない(偽 run 境界なし) |

リプレイに使う graw_replay(Rust 版新規、005 で実装済み)は `--rate-mbps`(**Mbit/s 単位**の
ペーシング。例: mini 100 Hz 相当 ≈ 28 MB/s = 224 Mbps)と `--loop` を持つ(C++ 版はペーシング
なし全速のみ)。**複数ファイル指定(021)**で per-AsAd 4 本組を **eventIdx 昇順に
インターリーブ**して 1 本の TCP で送る(= 実機ワイヤの再現。同 idx 内は引数順、
制御フレームは遭遇時に即時送出。単一ファイルは従来どおりバイトそのまま)。

## 13. 実機検証項目リスト(P5/P6 で実測 — 「実測で決める」を決定として固定)

1. **MTU 9000**: CoBo → スイッチ → 受信 NIC の全経路でジャンボフレーム疎通確認(P5 初日項目)。
2. **NIC 構成**: mini は 1GbE で足りることの確認。ELITPC×100 Hz(合算 ~0.9 Gbit/s)向けに
   10GbE 化 or CoBo 毎 NIC 分離のどちらを採るかは **Warsaw 配備前の実測で決定**(receiver は CoBo 毎
   独立なのでどちらでも構造変更なし)。
3. **AGET 読み出しデッドタイム**: 概算 ~1.4 ms/イベント(100 Hz で ~14%)の実測。実効レート天井は
   ソフトではなく GET フロントエンドで決まる想定の裏取り。
4. **DataSender id / flowType 実測**: `CoBo[0]` は FW パッケージ(`reference/ZC706_20181031_ELINP`:
   `describe-zCobo-ZC706.xcfg` の `Node id="CoBo" / Instance id="0"`、`README_SCRIPTS.txt` の実
   DataLinkSet 例)で**文書確認済み**(2026-08-12)— 実機では最終確認のみ。なお同スクリプトの
   DataRouter は `type="FDT"`(NarvalActor 向け)であり、我々の `type="TCP"` は同 getHwServer が
   サポートする別 flowType — TCP 経路の実機疎通は P5 で必ず確認する。
5. **実 ECC**: 20190315_patched 版コンテナで e2e → 実機。
6. **ディスク持続書き込み**: 実運用ストレージで 28 MB/s(mini)/ 111 MB/s(ELITPC)持続の実測。
7. **2-CoBo ジオメトリ .dat**: **解消(v1.7)** — ELITPC 実データ(2022/2026)は
   coboIdx=0・asadIdx=0..3 の **1 論理 CoBo** であり、既存 ELITPC .dat(COBO 0・4 ASAD)が
   そのまま正。2-CoBo .dat は不要(合成 2-CoBo フィクスチャは多 CoBo 対応の能力テストとして維持)。
   **データリンク本数も解消(v1.11、Aogaki 情報 2026-08-14)**: 2 枚の zCoBo(FPGA ボード)は
   **同一筐体内**に収まり、**内部で 2 つのネットワークを 1 つに束ねてから PC へ接続**する構成。
   したがって PC から見えるデータリンクは **1 本**(DataLinkSet の DataSender 1 エントリ・
   receiver 1 台 — v1.7 のワイヤ実態「1 論理 CoBo × 4 AsAd」および README_SCRIPTS.txt の
   DataLinkSet 単一エントリ例と完全整合)。P5 初日に実機で接続数を目視確認するだけでよい
   (万一 2 TCP 接続が同一ポートに来る構成だった場合は receiver の accept ループが
   同時 2 接続対応を要する — 現設計は 1 接続ずつ drain。その際は要改修として扱う)。
   **P5 初日の接続数目視は receiver の `extra_connections` カウンタで機械確認できる(v1.12)**
   (run 中 0 のままなら 1 リンク構成の実機確認完了。> 0 なら 2 接続構成 = 上記「要改修」シグナル)。
8. (P6)HiVolta: LOCAL モードでのモニタ無干渉・単一 TCP 接続の専有確認。HMP2020: LAN オプション
   有無と `SYST:MIX` 動作(Q2)。

## 14. PROPOSAL v0.4 との差分(調査由来 — 正本の v0.5 更新候補)

1. **eb_core 流用度の下方修正**: delila-rs eb_core のマージは timestamp ベース・単一グローバル
   バッファであり、「ソース毎 FIFO → eventIdx でマージ」という PROPOSAL §5 の記述に合う実装は
   存在しない。eventIdx ビルダは新規実装(§6.3)。流用は骨格(§6.1)。
2. **monitor へのイベント供給元の変更**: decoder → monitor 直結をやめ、root-sink の built-event
   publish に一元化(§1.2)。PROPOSAL の構成図は要更新。
3. **run_number を全データバッチに搭載**(delila-rs 方式からの改良、§2.2)。
4. **背圧方式**: delila-rs の「HWM=0 無制限バッファ」ではなく有限 HWM + 有界キュー + 可視 Error
   (§1.4)。PROPOSAL の「PUSH/PULL の背圧で守る」の具体化。
5. **ELITPC 2-CoBo の .dat は現存しない**(TPCReco リポの全 13 変種が COBO 0 のみ・4 ASAD)。
   GeometryTPC 自体は 2 CoBo 対応。§13-7 の確認事項へ
   (v1.7: 実データにより「不要」と確定 — ワイヤ実態が COBO 0・4 ASAD そのもの)。
6. root_sink は**ヒストをファイルに書かない**(THttpServer ライブのみ)— R10 実装は新規コード。
   PROPOSAL §5「root_sink の TH1D/TH2D 書き出し機構を流用」は「登録・生成機構を流用、書き出しは
   新規」が正確。
7. C++ 版 tpcdaq は run 番号がハードコード 0 で run メタデータを一切書いていない(JSONL 設計 §9 が
   これを埋める新規部分であることの確認)。

## 15. 用語

| 用語 | 意味 |
|---|---|
| フレーム | MFM フレーム(CoBo が送る単位、1 フレーム = 1 AsAd 分) |
| Fragment | デコード済みフレーム(§2.4) |
| built event | 同一 (run, event_idx) の全期待 Fragment を束ねたもの(§6.3) |
| 波高 | ストリップ波形(生 ADC)の時間方向最大値(R4) |
| ロスレス系 / モニタ系 | 落とさない経路(PUSH/PULL)/ 最新優先の経路(PUB/SUB)(CLAUDE.md 絶対ルール) |
