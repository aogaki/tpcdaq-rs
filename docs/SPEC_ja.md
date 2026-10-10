# tpcdaq-rs 仕様書(SPEC)

- **status**: **v1.36(2026-10-10 — 実装の正本)**
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
  / **v1.19(2026-08-16)§6.4 ROOT 系統の位置づけ再定義(ユーザー裁定 — Warsaw 大
  Wojciech 教授の要望 3 点が起点)**: **生データは graw のみ**。ROOT は解析レディー
  プロダクトであり処理を含んでよい(基底 = grawToEventTPC 互換 PEventTPC は不変・
  §12-3 オラクル維持。将来 Unfolding / ストリップ毎ゲイン正規化等を追加しうる —
  WARSAW_PLAN §7、成立性調査 = TODO/059)。§14-8 に PROPOSAL R8 との差分を記録。
  / **v1.20(2026-08-17)§8.2 ECC タイムアウトの意味論明文化(057 調査)**:
  per-REQ 60 s は正しく発火する(実測固定)。シーケンス全体は意図的に無期限
  (非冪等な歩きを途中放棄しない)— クライアント側タイムアウト + status ポーリングが
  正規の回復。`ecc command applied` ログに elapsed_ms を常設。
  / **v1.27(2026-09-14)§13 に実機実測を反映(遠隔ベンチ: GDED + 実 zCoBo + AsAd 内蔵パルサー、
  TODO/069 結果節)**: §13-1 MTU 9000 疎通実測、§13-4 `CoBo[0]`/`TCP` 実機確認 + **TCP 経路に
  topology frame は来ない(decoder カウンタ 0)= 確定**、§13-5 実 ECC(20190315)をホストで
  ネイティブ起動し e2e 成立、§13-7 `extra_connections=0` 実測(mini)。初回 configure 7 s
  (261 s コールドスタートは再現せず)。データは frameType 2 rev 5(compact)278,784 B/frame。
  / **v1.30(2026-09-17)Wave 1 反映(TODO/080)**: §5.4 に **AsAd 電子回路座標パネル**(TODO/077 — monitor タブの Event Display の下、(cobo,asad) 毎 nAget×nCh × nBuckets の 2D、0x03 由来)を追加、§10.3 の 0x03 購読条件を「波形ビュー**または monitor タブ**」に改訂。§6.7 に **raw ツリー契約の共有**(ガス圧シミュレーション gas-pressure-test の Stage 4 が同一契約で `_raw.root` を書き macros を共有 — 契約変更は両リポ同時、合成 raw は §5.5 QA 再構成のフィクスチャ供給元、080 D-8)を明記。 / **v1.31(2026-09-22)slowctl 骨格の契約(TODO/079 Phase 2、監視のみ)**: §2.3 に slowctl → monitor の PUB ペイロード(kind=`slow`)、§3.1 に `[slowctl]` / `[[slowctl.device]]` / `[monitor] slow_sub` の例、§3.2 に slowctl status PUB `47006` と source_id 200 = slowctl(旧 psu)、§9.2 の `psu` レコードを **`slow`** に一般化(IDENT / VALUE / ALARM / RECOVER / ERROR、値は機器の単位文字列つき)し run_start / run_stop に `slow`(直近値、nullable)を追加、§10.3 の status JSON に `slow` を同梱。作動 API は引き続き無し(§11)。 / **v1.32(2026-09-22)パラメータ JSON の契約(TODO/078 レーン A)**: §3.3 を新設 — JSON が正・xcfg は生成物、11 ノブとスコープ、例外は「一様値 + 例外リスト」、**CompoundConfig の継承は粗い例外を発注どおりには解決しない(Search.hpp の backtrack、オラクル実測)ので粗い例外は実在 id の直積へ展開**、生成物名 `<stem>__<sha256 8>`、受け入れは CompoundConfig オラクル(tools/ccfg_resolve、env-gated)。 / **v1.33(2026-09-23)パラメータ REST と run/start 結線(TODO/078 レーン B)**: §8.1 に `GET/PUT /api/params`・`GET /api/params/render`・`/api/status.params`、§3.1 `[controller]` に `ecc_config_dir` / `params_dir`、§9.2 `run_start.params`(nullable)と `audit run/start.params_id`、§3.3 に id の正規化規則(serde_json 辞書順の JSON テキストの sha256)と REST の状態規則(Running 中の PUT は 409、ecc_config_dir 未設定は 503)。 / **v1.34(2026-09-23)UI 2 ビュー(TODO/078 レーン C・079 Phase 4)**: §11 に **Parameters ビュー**(ノブ 11 のフォーム + Exceptions (N) + Preview xcfg + Save、status バーに `params <id> · exceptions N`)と **Slow control ビュー**(旧 Power を置換: 最新値の表 + ECharts トレンド(UI 内 30 min リングバッファ)+ counters、status バーに alarm 数)を追加。作動 UI は無し(安全境界)。 / **v1.35(2026-09-23)ecc-bridge を Rust に(TODO/076 Phase 3)**: §1.1 の ecc-bridge 行を Rust(`src/ecc/`、`src/bin/ecc_bridge.rs`、Ice プロトコル 1.0 / encoding 1.1 を自前実装、Ice ライブラリ非依存)に、§8.2 を「ecc-bridge(Rust)」に改題し C++ 実装の記述を履歴化。JSON REQ/REP 契約は不変(C++ の 457 CHECK を Rust テストに移植、fake ECC の e2e 52 本、実 ECC + 実 zCoBo で walk-back 一致 — 076 結果節)。C++ に残るのは fake_ecc(テストダブル)と e2e クライアントのみ。 / **v1.36(2026-10-10)パラメータの初期値をテンプレートから抽出(TODO/111)**: §3.3 に **抽出(render の逆)** — `src/params/extract.rs`、テンプレートの `*` 連鎖から一様値、明示 id ノードから例外、要素が無い葉は `unset`、型・範囲外や合成ノブが写らないものは `unsupported` として**明示**(黙って既定値を入れない)。ノブを **12(葉 15)**に: `readout` は `isAllChannelRead` だけ(full/partial)、**`fpnRead`(bool、`isFPNRead`)を分離** — mini の現用 xcfg は `isAllChannelRead=false` + `isFPNRead=true` で、2 要素を 1 モードに束ねると表せなかった。§8.1 に `GET /api/params/templates`・`GET /api/params/defaults?template=`。§11 の Parameters ビューは UI の既定値直書き(`DEFAULT_KNOBS`、HIgS の 25 MHz)を廃止し、テンプレート一覧から選んで抽出値で埋める(unset / unsupported は空欄 + バッジ + Save 無効)。受け入れ: 実テンプレート 3 本(mini delay_test / VM pulser / HIgS physics)で抽出 → render がバイト一致、オラクル 4,423 インスタンス一致、ELITPC 実機で抽出 → 無変更保存 → 生成 xcfg がテンプレートとバイト一致 → その id で run(111 結果節)。
  / **v1.29(2026-09-15)安全境界を明文化(§11、ユーザー裁定 — TODO/079 の発端)**: web 操作面(controller REST / UI)は DAQ の run 制御と監視のみ。**物理的作動(HV・ガス圧 setpoint・バルブ)は web に経路を作らず、ローカル限定**。§1.1 の psu 行を「監視のみ」に改め、名称を slowctl(HV / ガス / 環境の汎用スローコントロール)へ。根拠 = token + passphrase は事故防止であって認証ではない(§3.1)、TLS/認証は自前実装しない。
  / **v1.28(2026-09-14)§6.7 を実装前に精度上げ(TODO/075 着手、Fable)**: adc ブロック数は
  「観測」ではなく**期待フラグメント集合(§6.3 `--expect`)から起動時に確定**(ブランチは最初の
  イベントより前に要る。incomplete 先頭でも縮まない)/ 期待集合外フラグメントは書かずに数える /
  inprogress 名 `run_raw_inprogress_<unixtime>.root` / **ロールオーバ無し**(下流 = 単一ファイル
  契約)/ `--raw-tree` は `--no-root` と独立(raw のみ = ジオメトリ不要)/ 専用スレッド +
  有界キュー(PEventTPC recorder と共有物なし)/ 終了 JSON の `raw_*` カウンタ / オラクル = 2025
  mini 実 .graw(compact — graw2root.C は同日 frameType 2 対応済み)。§3.1 に `raw_tree` キー。
  / **v1.26(2026-09-03)§6.7 raw ツリー出力を新設(オプション — Fable 裁定。発端 =
  ユーザー要求「macros/ 解析チェーンを下流無変更で DAQ 直結、graw2root.C を不要に」)**:
  root-sink に第 2 出力 `run{run:04}_raw.root`(TTree `raw` = macros/graw2root.C と同一契約、
  生 ADC・FPN 込み・減算なし)。既定 off・mini プロファイルで on。PEventTPC(§6.4)と
  §12-3 オラクルは不変(置き換えではなく追加)。受け入れ = 同一 run で graw2root.C 出力との
  全エントリ一致。§6.5 表に 1 行追加。graw 側は無変更で互換と監査済み(§7 のバイト一致 +
  frameType 1)。実装入口 = TODO/075。
  / **v1.25(2026-09-02)§5.5 オンライン QA 再構成を新設(R5 の部分改訂 — Fable 裁定。
  発端 = ユーザー提案「本番は α 単独 = 1 イベント 1 トラック決め打ちで reco 込み DAQ」)**:
  モニタ系限定・monitor(Rust)内・既存 built_event PUB(§5.3、20 Hz)のサンプリングで
  simplefit を Rust 移植(Python 実装が正、フィクスチャ一致)。正本 TrackTree はオフライン
  (minitpc-analysis)維持、run.root / run<N>_monitor.root は不変。§4.4 に mm 例外
  (TPCReco 規約・モジュール内限定)、§10.2 に 0x01 非復活を注記。判断材料 =
  minitpc-analysis 実測(269 ev/s、頭尻 ~50% = 決定不能、100 mbar で Bragg 不在)。
  実装入口 = TODO/074(DRAFT)。
  / **v1.24(2026-08-20)§13-7 データリンク本数を実機実測でクローズ(TODO/073)**:
  HIGS 2026 現地の run 中 netstat で **zCoBo→DataRouter のデータ接続は 1 本のみ**
  (`4.2:46005 ← 4.84:44951`、CoBo がエフェメラルポートから接続する側 = 「リンクを張るのは
  CoBo 自身」の実機初観測)。現地 dataRouter 引数 = `FDT FrameStorage`(2026 運用も FDT 確定。
  単一リンク → AsAd 毎 4 ファイルの分割は dataRouter 側)。geometry .dat は
  reference/TPCReco latest を無改変使用(ユーザー確認)。
  **同日追補: patched-v2 疑義も決着(073 A-1)** — 実機 GET ツリー
  (reference/20190315_patched_v2)との全数 diff で**ソース差分は FrameStorage.{cpp,h}
  のみ**(env `GET_SPLIT_OUTPUT_BY_ASAD` による AsAd 毎ファイル分割 hack、2021-04-29)。
  制御プレーン(ECC/MemRead/DataSender/getHwServer/MDaq/mfm)は同一 → **既存の
  プロトコル検証は全て有効**。graw 命名 3 系統の出自確定: `CoBo{K}_AsAd{A}` =
  dataRouter + env / `CoBo_ALL_AsAd_ALL` = 同 env 無し / 素の `CoBo_` = GetController
  内蔵 raw 保存(TestController.cpp:389 — pedestal/pulser の単一ファイル形式。
  topology frame 込みで書く理由もこれで説明)。
  / **v1.23(2026-08-18)実ストリーム 3 事実の取り込み(TODO/067 — 実 pedestal/pulser 全数照合)**:
  ①**frameType 1 を「実データ照合済み」へ格上げ**(v1.7 ②の「合成のみ」を上書き): 実 pulser
  (frameType 1 rev 5・itemSize 4)304 frames / 42,336,256 items を GET 純正 MFM ライブラリ
  (CoBoEvent::decodeSamples と同一経路)と突き合わせ全一致。item ビットパック
  (aget 30/2・chan 23/7・buck 14/9・sample 0/12)は CoboFormats-Rev-5.xcfg と一致。
  **hit pattern は FPN{11,22,45,56} を除くが item データは FPN 込み全 ch** と意味確定。
  ②**topology frame(frameType 7・12 B)**は FDT センダのみ・daqStart 時に送出
  (MemRead.cpp:362 の senderType ガード)。decoder は認識 + INFO + `topology_frames`
  カウンタ(unsupported の内数)でスキップ(欠落 0 をテスト固定)。
  ③**FDT ワイヤ差分を契約として記録し、flowType は TCP 維持を裁定**(§8.2)。
  §6.3 に実データ由来の到着順注記。§12-1 に pulser frameType 1 オラクル追加。
  graw_replay は 0 バイト .graw(中断 run)を警告付きスキップ(--loop 無限ループの副産物修正)。
  **同日追補(TODO/068 — pedestal 36 run / pulser 25 run 全数走査)**: §6.3 の到着順注記に
  単一ファイル形式の実測(混在は常に隣接 1 イベント)と run 末尾不完全イベントの裁定
  (既定の incomplete emit で足りる)を追補。ノイズ・ゲインの物理統計は
  reference/_spike/asset_survey/out/ と archive/068 結果節が正(SPEC 対象外)。
  / **v1.22(2026-08-18)§13-7 に制御プレーン注記(TODO/065)**: 実 ECC 設定の生コピー
  調査で判明した describe-elitpc(2018)の CoBo 2 インスタンス定義は zCobo1k 移行前の
  2 ボード時代の記述と整理。P5 現地確認に「使用中 describe の CoBo インスタンス数」を追加。
  受信系の複数 CoBo 前提は維持。
  / **v1.21(2026-08-18)Recorder 並列化 P1(TODO/064 — 裁定 = 055)**:
  `--recorder-workers`(既定 1 = 完全互換)で worker 毎 TTree の並列書き出し。
  **mini 100 events/s 達成**(N=4: 隔離 152 /s、30 分 @224 Mbps 持続 100.4 /s・
  10/10 run・全カウンタ 0)、**§12 受け入れ表 5(フルレート持続)/ 6(burst 672 Mbps
  drop 0)をクローズ**。過負荷 stop の eos-timeout(旧保留②)は並列化で解消 —
  EOS 予算 5 s は据え置きで成立。§6.5 に _w{k} 命名を追加。021 オラクルは N=1 無改変 +
  N=2 ユニオンの両方で 3852 events / 0 differences。
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
| ecc-bridge | **Rust**(v1.35、旧 C++) | 1 | Ice クライアント(プロトコル 1.0 / encoding 1.1 を `src/ice/` で自前実装、Ice ライブラリ非依存)。JSON REQ/REP ↔ ECC(§8.2) |
| slowctl(旧 psu) | Rust | 1 | (P6 / TODO/079)HiVolta/HMP2020・ガス圧(Bronkhorst)・環境(Arduino)の**監視のみ**(ポーリング、TRIP/ALARM を controller へ投稿、run 記録に直近値)。**setpoint 等の物理的作動は載せない**(§11 安全境界、v1.29) |

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
| slowctl → monitor(PUB) | `Snapshot` | **v1.31**: kind = `slow` — `{kind, ts_ms, samples: [{device, node, param, value, unit, ts_ms, alarm}], counters: {poll_ok, poll_err, parse_err, reconnects, identity_mismatch}}`(map 形式 msgpack、root-sink と同じ流儀。1 Hz に集約、最新値のみ。monitor は落としてよい = モニタ系) |

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
raw_tree = false                       # §6.7 raw ツリー(run{N}_raw.root)。既定 off、mini で on 想定(v1.28)

[monitor]
ws_listen = "0.0.0.0:9000"
slow_sub = "tcp://127.0.0.1:47006"     # slowctl の status PUB(v1.31)。省略 = 購読しない(status JSON に slow は出ない)

[slowctl]                              # v1.31(TODO/079)。監視・記録・アラームのみ — 作動 API は無い(§11)
status_pub = "tcp://*:47006"
log_push = "tcp://127.0.0.1:47005"     # controller のログ投稿 PULL(§3.2)。省略 = この既定(079 Phase 2 レビューで追加)
poll_hz = 1.0                          # 全デバイス共通の読み出し周期
log_every_s = 60                       # 値が変わらなくても logbook `slow` VALUE を書く間隔(変化 2 % でも書く)
[[slowctl.device]]
type = "bronkhorst_flowbus"            # | "arduino_env" | "hivolta" | "hmp2020" | "fake"(テスト)
name = "gas"                           # logbook / status の `device`
port = "/dev/serial/by-id/usb-FTDI_xxx-if00-port0"   # /dev/ttyUSBn は書かない(by-id、無ければ by-path)
# hivolta は addr = "192.168.x.y:1470"(TCP、既定)または port(USB VCP 9600)のどちらか。両方あれば addr 優先(v1.34、TODO/089)
nodes = "scan"                         # FLOW-BUS: "scan" = 128 + 3..120 を起動時に走査、または [3, 4, 5]
expect_serial = ""                     # 任意。空でなければ open 時に 113/3 と照合(不一致 = エラーとして数えて再試行)
alarms = [{ node = 4, param = "fmeasure", lo = 90.0, hi = 110.0 }]   # [lo, hi] 逸脱で ALARM、戻れば RECOVER。DAQ は止めない

[controller]
rest_listen = "0.0.0.0:8080"
passphrase = "change-me"               # 操作権取得時の共有パスフレーズ(事故防止であり認証ではない)
ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"  # servant identity は "Ecc"(v1.13 訂正)
config_id = "default"                        # 略記 = 3 相同値。相ごとに変える場合(v1.13):
# ecc_config_dir = "/opt/get/configs"        # v1.33(§3.3): ECC の config repo = テンプレート xcfg の置き場 = 生成 xcfg の出力先。省略 = params 系 REST は 503
# params_dir = "/data/tpcdaq/params"         # v1.33: パラメータ JSON(current.json / <id>.json)の保存先。省略 = <output_root>/params
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
| slowctl status PUB bind | `tcp://*:47006`(v1.31) |
| コンポーネント REP | graw-writer = `47100`、decoder = `47101`、root-sink = `47102`、receiver k = `47110+k`(v1.2 で固定) |
| ecc-bridge REP | `tcp://*:47200` |
| controller REST / monitor WS | `8080` / `9000` |
| source_id | receiver = `cobo_id`、decoder = 100、root-sink = 101(モニタ PUB、v1.9)、slowctl = 200(旧 psu、v1.31。Batch の source_id 空間) |

- ch 数・CoBo 数・ポートを**コードに焼き込まない**。すべて TOML とジオメトリ(§4)から来る。
- 設定パースエラーは起動失敗(半端な既定値で走らない — C++ 版 app_config の全リセット方式より厳しくする)。

### 3.3 パラメータ JSON(v1.32、TODO/078 — 正本は JSON、xcfg は生成物)

- **xcfg を直接編集しない。** 実験側の正典 xcfg を**テンプレート**として無改変で取り込み、`tpcdaq-params/1` の JSON(`template` + `knobs` + `exceptions`)から
  `configure-<stem>__<sha256 先頭 8>.xcfg` と同内容の `prepare-…` を生成する(同じ JSON → 同じファイル)。describe は不変。
- **ノブ = 12(JSON の葉 15、v1.36)**、定義は `src/params/schema.rs` の `KNOBS` が唯一(表と根拠 = `docs/PARAMS_KNOBS_ja.md` §7)。
  CoBo: `triggerDelay`(10 ns 単位)/ `writingClockFrequency_MHz` / `triggerMode` / `zeroSuppression.enabled` / `pulser.{enabled, voltage_mV, period_ms}`(`Generator.amplitudeStart` と同値、mV)。
  AGET: `globalThreshold` / `peakingTime_ns`(AGET 仕様 16 値 = xcfg `range`)/ `readout`(`isAllChannelRead` のみ、full/partial)/ **`fpnRead`(`isFPNRead`、bool — v1.36 で `readout` から分離。mini の現用は partial + fpnRead true)**(`enableWriteHittedregister` と `channel.Reading` はテンプレート由来)。
  channel: `lsbThreshold` / `gain` / `triggerInhibition` / `zeroSuppression.threshold`。**全ノブ必須**(省略不可)。管理外の要素はテンプレートを 1 バイトも変えない(往復はバイト一致)。
- **例外リスト**: `at` は (asad, aget, ch) の接頭辞、ノブのスコープと同じか粗いものだけ。**CompoundConfig の継承解決(`CCfg/View/Search.hpp`)は
  `<AsAd id="1"><Aget id="*">` のような粗い明示ノードを一様値より優先しない**(段を戻って `*` の連鎖を選び直す backtrack。オラクル実測、078 結果節)。
  よってレンダラは粗い `at` を**テンプレート実在 id の直積へ展開して明示ノードで書く**(JSON の意味は不変、生成物が大きくなるだけ)。宛先 id がテンプレートに無ければエラー。
- **除去規則**: 管理ノブは当該ノブの全インスタンスノード(`*` と明示 id)をテンプレートから除去してから一様値を `*` に、例外を明示 id に書く(除去を省くとテンプレートの明示ノードが勝つ — 負性テストで固定)。
- **受け入れ**: ①往復(例外なし + テンプレート値 = バイト一致)②**CompoundConfig オラクル** `tools/ccfg_resolve/`(C++11、reference/ の CompoundConfig をビルド時参照、コミットしない、
  env `TPCDAQ_CCFG_RESOLVE_BIN` + `TPCDAQ_XCFG_TEMPLATE` があるときだけ照合し、無ければ skip を明示)で全インスタンス一致(実テンプレート 4,423)③実 ECC で describe → prepare → configure(新 DAQ 機)。
- **id と保存(v1.33、レーン B)**: `<id>` = `<テンプレート stem(configure-/prepare- を除いた部分)>__<sha256 先頭 8 桁>`、sha256 は **serde_json で再直列化した JSON テキスト(キー辞書順)**に対して取る(整形の違いで id が変わらない)。
  JSON は `<params_dir>/current.json`(常に上書き)と `<params_dir>/<id>.json`、生成 xcfg は `<ecc_config_dir>/configure-<id>.xcfg` と `prepare-<id>.xcfg`(**既存なら書かない**)。
- **REST(v1.33、§8.1)**: `GET /api/params`(無ければ 404)/ `PUT /api/params {token, params}`(検証 400、**Running 中は 409**、`ecc_config_dir` 未設定は **503**、成功で render + 保存 + `audit params/put`)/
  `GET /api/params/render?template=…`(dry run、書かない)。**run/start は current.json があれば prepare / configure の config_id に `<id>` を使う**(describe は TOML のまま)。
  current.json があるのに render できないときは run を始めない(TOML へ黙って後退しない)。UI(Parameters ビュー、Exceptions (N) バッジ、status バーの `params <id> · exceptions N`)はレーン C(§11 に着手時追記)。
- **抽出(v1.36、TODO/111)**: `extract(template)` は render の逆。`*` の連鎖にある値 → `knobs`(`KnobKind` の型・範囲に収まるものだけ)、明示 id ノードの値 → `exceptions`(`at` は明示 id の接頭辞)、
  `*` 連鎖に要素が無い葉 → `unset`、要素はあるが型・範囲外 / 合成ノブが 1 モードに写らない / CoBo の明示インスタンス / `*` の下の明示 id → `unsupported {knob, found, why}`。
  **黙って既定値を入れない**(unset / unsupported は UI が空欄 + 理由表示 + Save 無効)。受け入れ = 実テンプレートで 抽出 → render がバイト一致 + 再抽出の冪等性 + オラクル。
  REST `GET /api/params/templates` → `{templates: [configure-*.xcfg の名前]}`(生成物 `*__<hash8>` と他相は除く)、`GET /api/params/defaults?template=<name>` → 上記の JSON(400 = 名前がパス外 / 不正、404 = 無い、503 = `ecc_config_dir` 未設定)。

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
  (R5: オンラインの**正本**再構成は非スコープ — QA 再構成の例外は §5.5/v1.25。ヒスト軸も strip/bucket)。
- これにより strip_pitch 規約の不一致(TPCReco: `pad_size×1.5` vs rust_reference: `pad_size×√3/2`)は
  DAQ 層では**判断不要**になる。ヘッダスカラ(ANGLES 等)はメタデータとして保持し、UI の meta(§10.3)と
  JSONL(§9)へ素通しする。将来 mm が要る場合は TPCReco 規約を採る(オフライン整合が正)。
- **(v1.25)将来条項の発動は §5.5 の QA 再構成モジュール(monitor 内)に限定**。同モジュールのみ
  TPCReco 規約で mm を使う。DAQ・保存・§5.2 ヒストの hot path は引き続き strip/bucket のみ。

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
- **AsAd 電子回路座標パネル(v1.30、TODO/077)**: monitor タブの Event Display(U/V/W)の下に、**(cobo,asad) 毎**の
  2D(y = aget×nCh + ch、x = time bucket、値 = 表示専用のベースライン減算後生 ADC — 先頭 25 cell 平均、負値保持)を
  4 枚目として並べる。データは既存の 0x03 Waveforms(ジオメトリ非適用なので FPN・未マップ ch・dead ch も見える)。
  枚数・ch 数・bucket 数はメッセージ由来。0x03 は drop-oldest なので、UVW 側と event# が違うときは **`≠ UVW` を明示**
  (silent 禁止)。刻み・freeze は Event Display と共通。

### 5.5 オンライン QA 再構成(v1.25 — R5 の部分改訂)

**決定(Fable 2026-09-02、発端 = ユーザー提案)**: 本番実験は α 単独(1 イベント = 1 トラック)の
見込みであることを受け、**モニタ系に限定したサンプリング・トラック再構成(QA 再構成)**を
スコープに入れる。R5「オンライン再構成非スコープ」は**正本再構成について維持** — 正本の
TrackTree はオフライン解析(minitpc-analysis、Higs 側リポ)が持ち、DAQ は QA 表示のみ。

- **目的**: 電場スキャン 4 指標(v_drift = z 端点 / 拡散 = 幅 vs z / 付着 = 電荷 vs z /
  ゲイン = charge/length)を run 中にライブ表示し、条件出しを「翌日の解析」から
  「その場の作業」に変える(mini eTPC コミッショニングが直接の駆動要件)。
- **配置 = monitor(Rust)内の純関数モジュール。root-sink には入れない。**
  §5.1 がヒスト集計を root-sink に置いた根拠は完全性(全イベント積算)であり、QA 再構成は
  統計量で足りて完全性を要求しない → C3「保存と可視化の分離」が優先する。
- **入力 = 既存の built_event PUB(§5.3、`event_publish_hz` 既定 20 Hz、最新優先)**。
  新規配線なし。欠落は既存 sequence ギャップで可視化済み。20 Hz × 1 h ≈ 7.2 万トラックで
  QA 統計は十分。
- **アルゴリズム = minitpc simplefit の Rust 移植**(ビュー毎 PCA → ±3σ バンド再選択 →
  z スライス三角測量 → 3D PCA。ミニマイザなし = per-event コストが決定的)。
  **Python 実装が正**とし、§4.5 の二重実装パターンでフィクスチャ一致テストを置く
  (検証記録 = minitpc-analysis docs/simplefit-validation.md: 269 ev/s、
  残差中央値 5.6 mm、TPCReco 同等以上)。
- **拘束条件**:
  1. **1 トラック仮説は設定値**(例 `hypothesis = "single_alpha"`)。コードへの焼き込み禁止
     (ch 数と同格の不変条件。ELITPC は多分岐を含む — 該当イベントは reject でよいが、
     多分岐を構造的に排除する設計は不可)。
  2. **仮説違反は reject + カウント + status 表示**(多重ブロブ / z スライス < 3 /
     残差 RMS 閾値超過)。silent 禁止。
  3. **頭尻(head/tail)推定は実装しない** — 実測でどの手法も ~50%(100 mbar で α が
     止まらず Bragg ピーク不在のため原理的に決定不能)。方向はオフラインが線源位置の
     事前情報で決める。
  4. **mm 変換は本モジュール内に限定**、規約は TPCReco(§4.4 の将来条項を発動)。
  5. ベースライン: built_event は生 ADC(R13)。reco 用ベースラインの選択(per-channel 表
     (068)か先頭 25-cell 平均か)はチケット時に決定。保存系・§5.2 ヒストへは不適用
     (v1.18 裁定を維持)。
- **出力**: QA 集計は WS 0x10/0x11 の **id 20 番台**(§5.2 の id 1–9 = root-sink
  全イベント積算とは別空間。UI に「sampled」明示)。トラック重畳型(0x04)は必要時に起票。
- **run.root は不変**(§6.4 の grawToEventTPC 互換 drop-in 等価は不可侵。friend tree も不可)。
  **`run<N>_monitor.root` にも QA ヒストは入れない**(全イベント積算のみの原則を守る。
  サンプル集計の永続化が欲しくなったら JSONL run サマリ(§9)で別途裁定)。
- **受け入れ**: reco on/off で保存スループット(§12-8)と WS 配信(ドロップ・遅延)が
  不変であることを計測で確認。事前スパイクは不要(20 Hz サンプリングでは Python 実測
  3.7 ms/event ですら ~7% コア — 律速になり得ない)。

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
- **到着順への非依存は実データで裏付け済み(v1.23、TODO/067)**: 実 pedestal
  (467 event)で AsAd 到着順の回転(`2,3,0,1` 等)2 件、**eventIdx の後退 40 箇所
  (後退幅は必ず 1 = 隣接 event の混在のみ)**、1 event の到着幅最大 6 フレームを実測。
  ビルダは **AsAd 順にも eventIdx 単調性にも依存してはならない**(現行 eventIdx
  グルーピング実装はこの条件を満たす。到着幅 6 ≪ build_timeout)。
  **同日追補(TODO/068 全ラン走査)**: 単一ファイル形式(GetController 経由)でも
  イベントは連続塊でない箇所が pedestal 36 run で 25,438 箇所 — ただし**混在は常に隣接
  1 イベント分**で eventIdx は単調(pulser/physics は 0 箇所 = レート依存)。並べ替え
  「窓」の固定深さは設けない(eventIdx キー + build_timeout の現行方式が正で、実測上限
  1 イベントに対し十分)。**run 末尾の切断による不完全イベント**(全データで 15 個、
  AsAd の欠け順 3→0→1→2 と規則的 = ソース側の run stop 由来)は既定の
  「incomplete フラグ付き emit(捨てない)」がそのまま適用される — 追加ポリシー不要と裁定。
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

**位置づけの再定義(v1.19、ユーザー裁定 2026-08-16 — Wojciech 要望起点)**:
**生データは graw のみ**。ROOT は「生データの別形式」ではなく**解析をすぐ始めるための
解析レディープロダクト**であり、処理を含んでよい(graw から原理的に作り直せることが保険。
現に PEventTPC はペデスタル減算済みで、既にこの位置づけにある)。
- **並列書き出し(v1.21、TODO/064)**: `--recorder-workers N`(既定 1)。**N=1 は
  従来と完全同一**(コードパス・出力名とも)。N>1 は worker 毎に TTree 一式を専有し
  round-robin 分配(有界キュー・背圧・ロスレス)。worker 内は eventIdx 単調、
  ファイル横断の全順序は保証しない(イベント自立・照合は eventId キーの内容一致)。
  実測: mini N=4 = 152 events/s(隔離)/ 100.4 events/s(30 分 @224 Mbps 持続)。
- **基底は現行どおり grawToEventTPC 互換の PEventTPC**(§12-3 内容一致オラクルは不変)。
- 将来、**Unfolding(ストリップ毎の信号復元)・ストリップ毎ゲイン正規化**等の処理済み
  派生物を**追加**しうる(WARSAW_PLAN §7。基底互換ツリーを壊さない追加が既定形 —
  具体形は実装時に本節を改訂して定義する)。

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

- **worker パート(v1.21)**: `--recorder-workers` N>1 のとき worker k の出力は
  `run{run:04}_w{k}.root` + ローテーション `run{run:04}_w{k}_{part:04}.root`
  (1 GiB ローテーションは worker 毎に独立)。N=1 は従来命名のまま。

| 出力 | 命名 | 備考 |
|---|---|---|
| イベント TTree | `run{run:04}.root`(rollover 時 `_0001` 付加) | 書き込み中は `run_inprogress_<unixtime>.root`、finalize で rename。異常終了は inprogress のまま残す(完全 run に化けない) |
| モニタヒスト | `run{run:04}_monitor.root` | 全ソース EOS 後に書き出し。**EOS から 10 秒以内**(R10「速やかに」の数値化) |
| raw ツリー(オプション) | `run{run:04}_raw.root` | v1.26 §6.7。既定 off。書き込み中は `run_raw_inprogress_<unixtime>.root`、finalize で rename、異常終了は inprogress のまま(規則はイベント TTree と同一、名前空間だけ分ける)。**ロールオーバ無し**(v1.28) |
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

### 6.7 raw ツリー出力(v1.26 — mini 解析チェーン直結、オプション)

**決定(Fable 2026-09-03、発端 = ユーザー要求)**: mini eTPC のスタンドアロン解析チェーン
(`macros/` — graw2root.C → analyzeUVW.C → makeTracks.C。素の ROOT のみ・TPCReco 不要)を
**下流無変更**で DAQ に直結するため、root-sink に**オプションの第 2 出力**
`run{run:04}_raw.root` を追加する。有効時、graw2root.C(生 .graw → ROOT の変換ステップ)は
不要になる。

- **形式 = macros/graw2root.C の出力と同一契約**(= 下流 analyzeUVW.C の入力契約):
  TTree `raw`(1 エントリ = 1 ビルド済みイベント、エントリ順 = emit 順 = event_idx 昇順 §6.3)
  - `eventId/i` = ビルダの event_idx(= フレームヘッダ eventIdx)
  - `eventTime/l` = イベント先頭フラグメント((cobo,asad) 昇順の先頭)の 48-bit CoBo タイムスタンプ
  - `adc[4][68][512]/S` = **生 ADC(FPN 込み・減算なし — R13 と同じ生値)**。
    リーフリストは**期待フラグメント集合(§6.3 `--expect`)の要素数 nAsad から起動時に生成**
    (`adc[{4×nAsad}][68][512]/S` — 焼き込み禁止。mini = `0:0` の 1 要素で macros 契約と完全一致。
    v1.28: 旧「観測 AsAd 数」は撤回 — ブランチは最初のイベントより前に要り、先頭が incomplete
    でも配列が縮んではならない)。AsAd ブロックの順 = 期待集合の (cobo,asad) 昇順の順位。
    **期待集合外の (cobo,asad) は書かずに数える**(`raw_unexpected_fragments`)。chan ≥ 68 の
    item も同様(`raw_items_out_of_range`、Filler と同じ防御)。
- **既定 off、設定で on**(mini プロファイルで on を想定。CLI `--raw-tree`、TOML `raw_tree`)。
  **PEventTPC(§6.4)は不変** — 置き換えではなく追加。grawToEventTPC 互換オラクル(§12-3)に
  触れない。**`--no-root` と独立**(v1.28): `--no-root --raw-tree` = raw のみ(ジオメトリ不要 —
  macros チェーンだけを使う mini 運用の最小形)。
- **保存系**として扱う(意図的ドロップ禁止。open / mkdir 失敗は PEventTPC と同じ
  `fatal_reason` → exit 5 = 黙って書けないまま走り続けない。カウンタは終了 JSON の `raw_*`)。
  incomplete イベントは届いた分で fill、欠損は 0(graw2root.C の memset 既定と同じ意味論)。
  書き込みは**単一ライタ = 専用スレッド + 有界キュー(in-band RunClose)**(mini レート
  ≈ 28 MB/s @100 Hz は 1 スレッドで十分。worker 分割(§6.5)には載せない — 下流が単一ファイル
  前提。**ロールオーバも無し**(v1.28、同じ理由。10 分 run @100 Hz ≈ 16 GB 非圧縮 — ROOT は
  単一ファイルで問題ない)。PEventTPC recorder スレッドとはロックもキューも共有しない
  (§5.1 と同じ流儀)。有効時のコストは集計スレッドでの BuiltEvent 1 コピーのみ。
- **なぜ root-sink か**: PEventTPC はペデスタル減算済み・ジオメトリ適用済みで**生 ADC を
  復元できない**(graw2root.C を PEventTPC 読みへ「適応」させる道は原理的に不成立)。
  生波形とビルド済みイベントを同時に握るのは root-sink のみ。
- **受け入れ(オラクル = graw2root.C そのもの)**: 同一 mini run について root-sink の
  `run{N}_raw.root` と graw2root.C(同じ .graw)の tree `raw` が**全エントリ一致**
  (eventId / eventTime / adc 全要素 — 比較器 `tools/root_sink/compare_raw`、負性対照つき)。
  オラクル入力 = 2025 mini 実 .graw(`TPCDAQ_REAL_GRAW`、compact — graw2root.C は
  2026-09-14 に frameType 2 対応済み。**2026 pulser は ELITPC 4 AsAd 単一ファイルなので
  graw2root.C の 1 フレーム = 1 エントリと合わず、オラクルに使えない**)。
  加えて §12-8 の変形(raw 出力 on/off で PEventTPC 書き出しスループット不変 ±5%)。
- **実装・実測済み(2026-09-14、TODO/075 結果節が正)**: 2025 mini 実 run で graw2root.C と
  `compared 108 events, 0 differences`(負性対照 1 difference)。§12-8 変形: N=4 で 224 Mbps /
  672 Mbps とも raw on/off の差 +0.3% / +0.2%(ロスレスカウンタ全 0)。raw ファイルは 192 KB/event
  (パルサー、ZLIB-1)= 100 Hz で 19 MB/s。CLI `--raw-tree`、TOML `raw_tree`。
- **契約の共有(v1.30、TODO/080 D-8)**: ガス圧シミュレーション `~/test/gas-pressure-test`(Geant4 + Garfield++ + AGET
  応答、別リポ)の Stage 4 は**この節と同一契約**(`eventId/i`、`eventTime/l`、`adc[4][68][512]/S`)で `_raw.root` を書き、
  `macros/` を `external/tpcdaq-macros/` として共有する。**契約を変えるときは両リポを同時に直す**。シミュレーションの
  合成 raw(真値付き)は §5.5 QA 再構成のフィクスチャ供給元であり、順方向応答(Magboltz 拡散 ⊗ AGET 整形)は
  Unfolding(WARSAW_PLAN §7)の応答カーネルの出所になる。

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
  - **パラメータ(v1.33、§3.3、TODO/078)**: `GET /api/params` → `{id, sha256, exceptions, params, files: {configure, prepare}}`(無ければ 404)/
    `PUT /api/params {token, params}` → 同形(検証失敗 400 + `ParamsError` 文言、Running 中 409、`ecc_config_dir` 未設定 503)/
    `GET /api/params/render?template=<name>` → `{id, exceptions, configure_xcfg}`(dry run、書かない)。`GET /api/status` に `params: {id, exceptions} | null`(失敗は null + `notes`)。
    **v1.36(TODO/111)**: `GET /api/params/templates` → `{templates: [string]}`(`ecc_config_dir` 直下の `configure-*.xcfg`、辞書順)/ `GET /api/params/defaults?template=<name>` →
    `{template, knobs, exceptions, unset, unsupported}`(テンプレートからの抽出、書かない。400 / 404 / 503 は §3.3)。
  - 状態変更系はすべて token 必須 + 監査ログ(audit レコード)。閲覧系は認証なし(二層アクセス制御)。
    **例外(v1.10 明文化)**: `POST /api/logbook/comment` は **token 不要** — DAQ 状態を変えない
    記録系であり、R11(シフト全員が書けるログブック)を操作権 1 名モデルより優先する。
    author は自己申告(レコードに残る)。
- run 番号: `<output_root>/tpcdaq_state.json` に `next_run` を永続化(controller 単一書き手)。
  手動設定は **`POST /api/run/next {token, next_run}`**(v1.10 で形を確定): run 実行中は拒否、
  正整数のみ、audit 記録。次の run/start から有効。
- Web UI(Angular)静的ファイルは controller が配信(delila-rs と同じ「Rust だけでデプロイ」方式)。

### 8.2 ecc-bridge(Rust — v1.35。〜v1.34 は C++、TODO/076 Phase 3 で撤去)

- `src/bin/ecc_bridge.rs` = `src/ecc/{core,proxy}.rs`(状態機械・JSON 契約・DataLinkSet XML の移植)+ `src/ice/`(Ice 1.0/1.1 のワイヤ)を
  JSON REQ/REP サーバに被せた薄いプロセス(同期、SIGINT/SIGTERM で停止、ログは stderr)。旧 C++ 版と契約・状態遷移・ログ行は同一(457 CHECK 移植、e2e 52 本、実機 walk-back 一致)。状態: `Off/Idle/Described/Prepared/Ready/Running/Paused/Unknown`。
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
  **flowType は TCP 維持を裁定(v1.23、TODO/067)— FDT は非対応と明文化**。FDT のワイヤは
  plain TCP と異なる(GetBench 一次資料): 接続直後に 4 B の IMALIVE `00 00 00 00`
  (FdtDataSender.hpp:125)、イベント 3 s 途絶毎に同 4 B(heartBeatPeriod_ms=3000)、
  daqStop で 4 B の GOODBYE `FF FF FF FF` + **ソケット close(= run 毎に再接続)**、
  daqStart で topology frame(frameType 7)送出。これらの 4 B 制御語は MFM フレームでは
  ないため現行 framer はバッファ破棄で反応する。**将来 FDT が必要になった場合は
  ①IMALIVE/GOODBYE の除去 ②run 毎 connect/disconnect ③vcobo への同挙動の追加、の
  3 点セットを新ユニットとして起票すること**(なお実 2025 mini run の graw 先頭にも
  frameType 7 が実在 — 当時のリンクが FDT だった可能性を示唆。TCP 経路の topology 有無は
  069 Q1 で現地確定)。decoder の frameType 7 防御(v1.23)はどちらの経路でも無害に働く。
- 例外は全部 Result 化(never throw)。ECC 不達は `state: "Unknown"`。
- **タイムアウトの意味論(v1.20 明文化 — 057 調査で確定)**: controller の ECC REQ は
  **1 リクエストあたり 60 s**(rcvtimeo。発火することは遅延 fake ECC で実測固定 —
  tests/controller_ecc_timeout.rs)。**run/start 等のシーケンス全体には意図的に期限を
  設けない** — 歩き戻し込みで最大 9 REQ = 最悪 540 s。ECC 歩きは状態を持つ列であり、
  途中で放棄する方が危険(非冪等)。クライアントは自前のタイムアウト + `/api/status`
  ポーリングで回復する(UI は既にこの形)。実測の含意: 実 ECC の configure は
  コールドスタートで分単位かかりうる(056 で 261 s 観測)— 各 REQ の所要は
  controller ログの `ecc command applied` に `elapsed_ms` として常時記録される。

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
| `run_start` | `run`, `config_id`(3 相同値ならその文字列。非同値なら configure 相の id)、**`config_ids`(v1.13 追加、非同値時のみ**: `{describe, prepare, configure}`。同値時は省略 — nullable 規律)、`geometry: {path, sha256}`, `cobos: [{id, listen}]`, `operator`, `comment`, `expected_fragments`(期待 (cobo,asad) 集合)、**`slow`(v1.31、nullable — slowctl が動いていなければ省略)**: `{"<device>/<node>/<param>": {value, unit, ts}}` = controller が受け取った `slow` レコードの直近値(VALUE/ALARM/RECOVER 由来)、**`params`(v1.33、nullable — current.json が無ければ省略)**: `{id, sha256, exceptions}`(§3.3。prepare / configure の config_id がこの `id`) |
| `run_stop` | `run`, `duration_s`, `ok: bool`, `reason`(**"normal" / "error:eos-timeout" / "abort:<原因>"** — abort は停止開始時点の起因。EOS の顛末は次の 2 フィールドが持ち、reason には合成しない)、**`forced_eos: bool`**(EOS を receiver `Stop` で注入したか。**実機 TCP flow では通常 true**(§1.3 v1.12 — stop はデータリンクを close しない)。EOF 由来の自然 EOS のみ false)、**`eos_closed: bool`**(EOS がチェーンを流れ切ったことを観測できたか。**false は異常の印**であり、reason が abort でも eos-timeout の事実はここで読める)(v1.12 追加。**v1.14 注記**: 実機 TCP flow では `forced_eos:true` が常態なので、**`forced_eos:false` は「stop 前にリンクが死んだ」ことの強い印** — CoBo 突然死は OS の正常 FIN により自然 EOF として normal クローズし、他に痕跡が残らない(041 D-2 実測)。リーダ・UI は `eos_closed:false` と並んで `forced_eos:false` にも注意を向けること)、`counters: {events_built, events_incomplete, late_fragments, frames: {cobo: n}, overflow_frames, malformed}`(**v1.10: 各項目は nullable — null = 「その時点で取得不能」であり 0 と混同しない**。root-sink が REP を持たない間、events_built/events_incomplete/late_fragments は null。取れる分は GetStatus 実測)、`files: [{path, bytes}]`(graw 群 + root + monitor.root 実績)、**`slow`(v1.31、nullable)**: run_start と同形の直近値(run 平均は後続で追加) |
| `audit` | `action`(REST エンドポイント名), `params`(要約。**v1.33**: `run/start` は `params_id`(無ければ null)、`params/put` は `{id, exceptions}`), `operator`, `ok`, `error` |
| `comment` | `author`, `text`(自由記述、R11) |
| `slow` | **v1.31、旧 `psu` を一般化(TODO/079)**。`device`(TOML の `name`)、`node`(機器内アドレス: FLOW-BUS ノード番号 / HV ch 番号 / 無ければ null)、`event`(`"IDENT"` 起動時の識別 / `"VALUE"` 計測値 / `"ALARM"` / `"RECOVER"` / `"ERROR"` 読み出し失敗の集計)、`values`: VALUE/ALARM/RECOVER は `{<param>: {value: f64, unit: "<機器が返した単位文字列>"}}`、IDENT は `{<field>: "<string>"}`、ERROR は `{poll_err, parse_err, reconnects, identity_mismatch}` の増分。**1 Hz の全サンプルは書かない**: 前回書いた値から 2 % 以上動いたか `log_every_s` 経過で VALUE、ALARM/RECOVER は即時、ERROR は 1 分毎の集計(0 なら書かない)。actor = `slowctl` |

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

- 旧 0x01 Event(3D 点群)は**廃止**(R5 非スコープ)。**§5.5(v1.25)の QA 再構成でも
  復活させない** — 中身が点群ではない。QA 集計は 0x10/0x11 の **id 20 番台**(発生源 = monitor、
  sampled 明示)で運び、イベントディスプレイへのトラック重畳が必要になった時点で
  0x04 `Track` を起票する。
- ヒストの id は §5.2 の表と一致(1–3 が 2D、4–9 が 1D)。ビン値は f32 に落とす(表示専用。正値は
  monitor.root 側にある)。

### 10.3 JSON テキストメッセージ

| type | 方向 | 内容 |
|---|---|---|
| `meta` | S→C(接続時・run 変化時) | `{nBuckets, planes: {U,V,W}, geometry, anglesDeg, detector, cobos, run}` |
| `status` | S→C(1 Hz) | §5.3 の status + `{monitorGaps, clients, wsDropped}` + **`slow`(v1.31、nullable)**: slowctl の最新 `slow` ペイロード(§2.3)を同梱。`[monitor] slow_sub` 未設定なら省略(従来と同一) |
| `run` | S→C(遷移時) | `{state, run, ts}` |
| `subscribe` | C→S | `{"streams": ["uvw","waveforms","histos","status"]}` — 既定は waveforms **以外** ON(帯域制御。**波形ビューまたは monitor タブ**(v1.30、§5.4 AsAd パネル)を開いたクライアントだけが 0x03 を受ける) |

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

- **Parameters ビュー(v1.34、TODO/078 レーン C。v1.36 で初期値をテンプレート抽出に — TODO/111)**: §3.3 の JSON を編集する唯一の画面。ノブ 12(葉 15)のフォーム(型・範囲は UI の表 = `src/params/schema.rs` の写し、
  過不足は spec で検知、最終検証はサーバの 400)、折りたたみ「Exceptions (N)」(件数は常に見える)、「Preview xcfg」(`GET /api/params/render` の本文、管理ノブの行をハイライト。
  テンプレートとの diff は出さない)、「Save」(`PUT /api/params`、操作権 token 必須、Running 中は無効)。status バーに `params <id> · exceptions N`(15 s poll、無ければ `—`)。
  **初期値(v1.36)**: UI に既定値を焼き込まない。「Load from template」で `GET /api/params/templates` の一覧から選び(読めなければ名前の手入力に落として理由を出す)、
  `GET /api/params/defaults` の抽出値でフォームと Exceptions 表を埋める。`unset` / `unsupported` / 手で空にした葉は空欄 + バッジ(`unset in template` 等)+ 警告一覧、
  値の無い葉がある間は Save 無効(理由を表示)。下書きに変更があるまま読み直すときは画面内で確認(捨てる / Cancel)。
- **Slow control ビュー(v1.34、TODO/079 Phase 4、旧 Power を置換)**: WS `status.slow`(§10.3)の最新値の表(device / node / param / value unit / age / alarm)、
  ECharts トレンド(UI 内リングバッファ 1 Hz × 30 min、最大 4 系列)、`counters` 表。`slow` 無しは「No slow-control data」(モック値なし)。status バーに alarm 数。
  **作動 UI は置かない**(下の安全境界)。
**安全境界(v1.29、ユーザー裁定 2026-09-15)**: web 操作面(controller REST / UI)が持てるのは **DAQ の run 制御と監視だけ**。**物理的に壊せる作動 — HV の電圧設定、ガス圧の setpoint、バルブ — は web に経路を作らない**(controller にも UI にも API を置かない)。それらは機器側の手段(ベンダー GUI・フロントパネル)か、DAQ 機上の **localhost / Unix ドメインソケット限定の CLI** で、物理的にその場にいる人だけが行う。根拠: 操作権 token + passphrase は事故防止であり認証ではない(§3.1)、TLS/認証は自前実装しない方針。DAQ 側のスローコントロール(§1.1 slowctl)は読み取り・記録・アラームに徹する。

デザイン規律: Atlassian Design 準拠、モニタは Grafana 風ダーク。破壊的操作は確認パターン。
freeze は表示のみで、run Stop と視覚的に混同させないこと(§5.4)。

## 12. 受け入れ基準(数値確定)

| # | 項目 | 基準 |
|---|---|---|
| 1 | デコーダオラクル | 実 2025 run graw(ローカル、`TPCDAQ_REAL_GRAW` 環境変数)で **events=108 / items=15,040,512 / malformed=0**。実 ELITPC graw(`TPCDAQ_REAL_GRAW_DIR`、2022/2026 各 4 ファイル、v1.7)で **各ファイル frames=3852 / items=536,444,928 / malformed=0 / unsupported=0 / eventIdx 0..=3851 連続 / eventTime 単調**。CI は合成フィクスチャで frameType 1/2 両方 green。**frameType 1 実データオラクル(v1.23)**: 実 pulser(`TPCDAQ_REAL_GRAW_PULSER`)で **data_frames=304 / items=42,336,256 / topology=1 / malformed=0 / reset_count=0**、実 pedestal(`TPCDAQ_REAL_GRAW_PEDESTAL`)で **data_frames=1868 / items=260,145,152 / topology=1 / malformed=0**(tests/decoder_real_stream_compat.rs、環境変数未設定時 skip) |
| 2 | graw バイト一致 | frameType 1/2 を asadIdx で分別した列 = per-AsAd 出力、残り全フレームの列 = ctrl/ 出力、**全出力の合計 = 入力の完全ロスレス分割**(v1.2)。mini 実 graw オラクル: AsAd ファイル 30,108,672 B + ctrl 12 B(frameType 7 ×1)= 30,108,684 B。ローテーション跨ぎも連結一致。ELITPC 実ファイル(1,073,875,968 B > 2^30)を既定 max でリプレイすると **_0000 が入力と完全バイト一致 + 空 _0001**(ローテーション境界の実機一致、v1.7) |
| 3 | TTree 互換 | **v1.8: PEventTPC 互換** — ①構造一致: 実機 grawToEventTPC 出力(2026 実ファイル)とツリー名/ブランチ/クラス streamer バージョン/圧縮が一致 ②値一致(単体): 既知入力 → chargeMap 期待値(strip 射影・signal 窓・ペデスタル算法の手計算オラクル)③値一致(実データ): 同一 run の graw 4 本組と grawToEventTPC 変換済み .root のペアで全イベント全 key の値一致(env-gated)— **2026-08-14 達成: `compared 3852 events, 0 differences`**(TODO/021、tests/elitpc_pevent_e2e.rs)。~~旧 GDataFrame 比較の維持~~ **v1.17 で撤去**(GDataFrame は graw2root の形式で不要 — ユーザー裁定。§6.4) |
| 4 | 2 ソースビルド | graw_replay ×2 並走(異なる CoBo id を模す)→ 全イベント complete、eventIdx 昇順、incomplete=0、CoBo 毎フレーム数一致 |
| 5 | 連続負荷 | **v1.21 追記: フルレート持続を宅内で実証** — 30 分 @224 Mbps・`--recorder-workers 4` で 10/10 run 合格(100.4 events/s・全カウンタ 0・eos-timeout なし・RSS 平坦)。一晩トレンド soak は 031(45 Mbps × 8.4 h)で実証済み。**v1.15 で二段化(ユーザー裁定 2026-08-15 — ソフト単体の 24 h は過剰、フル 24 h はハード込みでこそ意味がある)**: **(a) 自宅ソフト soak = 一晩(≥ 8 h、既定 12 h)のトレンド駆動** — 100 Hz 相当ペース(mini ≈ 28 MB/s)のループリプレイ、保存系 drop 0(全カウンタ 0: overflow / gap / malformed / late)、各プロセスの RSS/fd/全カウンタを定期サンプリングし**後半半分で単調増加なし**(上昇トレンドが出たら延長 or 修正)。ディスク節約のため「書いて検証して消す」ハーネス可。**(b) フル 24 h(100 Hz 相当連続)は ELI-NP 実機テストでハード込みで実施**(P5 前段 — §13-7 の現地項目に併合。ソフト soak は実機時間をハード起因の問題だけに純化するためのフィルタという位置づけ) |
| 6 | 瞬発負荷 | **v1.21: 達成** — 672 Mbps × 10 分・N=4 で保存系 drop 0(recv_overflow=0、達成 648.4 Mbps、290.8 events/s。TODO/064)。**v1.16 で定義を明示レート形に改訂**: **ペーシング付き ≥3× 目標レート(mini ≈ 672 Mbps / 84 MB/s)で 10 分、保存系 drop 0**(バッファ設計の証明)。旧「ペーシングなし全速」は loopback で **216× 目標(6.06 GB/s)** に達し物理的対応物が無い(031 実測)。なおその 216× 条件で receiver の never-stop + counted drop(§1.4)が設計どおり働き silent でないことは実測確認済み — これは §12-6 の合否とは別の設計検証として記録 |
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
   **実測済み(v1.27、2026-09-14)**: GDED enp1s0f1(MTU 9000)→ zCoBo 4.81 へ DF ping 8972 OK。
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
   **実機確認済み(v1.27、2026-09-14)**: `CoBo[0]` + `TCP` で configure → Ready(2,275 ms)、
   zCoBo が DataLinkSet の 192.168.4.2:46005 へ接続(peer 192.168.4.81:33145)。**TCP 経路に
   topology frame(frameType 7)は来ない** — decoder `topology_frames=0`、graw-writer
   `ctrl_frames=0`(2 run、142 frames)。069 Q1 確定(MemRead.cpp の「FDT のみ送出」どおり)。
5. **実 ECC**: 20190315_patched 版コンテナで e2e → 実機。
   **実機 e2e 済み(v1.27)**: getEccServer 20190315 を DAQ ホスト(Ubuntu 24.04)で**ネイティブ起動**
   (Boost 1.58 / Ice 3.7.2 / OpenSSL 1.0 をローカル展開、sudo 不要 — メモリ geteccserver-native-gded)。
   controller → ecc_bridge → ECC → zCoBo で describe 67 ms / prepare 4.9 s / configure 2.3 s /
   start 151 ms、2 run 目の歩き戻し `status→Ready, breakup→Prepared, reset→Described, reset→Idle`。
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
   **mini で実測(v1.27)**: 2 run とも `extra_connections=0` / `empty_connections=0`。
   **制御プレーン側の注記(v1.22、TODO/065 — 実 ECC 設定の生コピー調査)**: 実験側の
   `describe-elitpc.xcfg`(2018 年、現行 GetSoftware_config にも同一で残存)は **CoBo 2
   インスタンス**(192.168.10.40 / 192.168.3.40)+ DataRouter 2 本(`CoBo[0]`→NarvalActor、
   `CoBo[1]`→NarvalActor1)を定義しており、上記「1 リンク」と食い違って見えるが、これは
   **zCobo1k(1 台 4 AsAd、2021-11 初出)移行前の 2 ボード × 2 AsAd 時代の記述**と読むのが
   自然(現行 HIGS 運用は workspace 経由で単一 TARGET・zCobo1k 設定のみ使用 — 065 実測)。
   ワイヤ実態(v1.7)と矛盾しない。ただし Warsaw 本体が今もこの 2 インスタンス describe を
   使う可能性は残るため、P5 現地確認に「使用中 describe の CoBo インスタンス数」を含める。
   受信系は describe に CoBo が複数現れても壊れない(= 既存の複数 CoBo 前提)を維持する。
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
8. **R8「graw2root 互換 ROOT」の位置づけ再定義(v1.19、ユーザー裁定 2026-08-16)**:
   PROPOSAL R8 は ROOT 系統を「互換出力のオンライン再現」と定義したが、これを
   「**解析レディープロダクト**(互換 PEventTPC を基底に、Unfolding・ゲイン正規化等の
   処理済み派生物を追加しうる)」へ拡張する。**生データは graw のみ**という区別を明文化
   (§6.4 v1.19)。Warsaw 大 Wojciech 教授の要望 3 点(WARSAW_PLAN §7)が起点。

## 15. 用語

| 用語 | 意味 |
|---|---|
| フレーム | MFM フレーム(CoBo が送る単位、1 フレーム = 1 AsAd 分) |
| Fragment | デコード済みフレーム(§2.4) |
| built event | 同一 (run, event_idx) の全期待 Fragment を束ねたもの(§6.3) |
| 波高 | ストリップ波形(生 ADC)の時間方向最大値(R4) |
| ロスレス系 / モニタ系 | 落とさない経路(PUSH/PULL)/ 最新優先の経路(PUB/SUB)(CLAUDE.md 絶対ルール) |
