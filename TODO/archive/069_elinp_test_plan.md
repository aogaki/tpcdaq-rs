# 069 — ELI-NP 実機テスト計画(mini eTPC、1 CoBo、徹底実施)

**Status: CLOSED(2026-10-13、処置は末尾)** — 旧: DRAFT(起草 Fable 2026-08-18。ユーザーレビュー待ち。
解禁条件「デモ改良が一段落してから」は 2026-08-15 のトラック完了で成立)

## 目的と枠

- **対象**: 実 CoBo(zCoBo)+ 実 ECC + mini TPC を相手に、仮想スタックでは構造的に
  検証できない点(docs/VIRTUAL_ZCOBO_ja.md §5.3 の 3 点: 実レジスタ・FW 由来データ・実タイミング)
  と、032/036 以降の結果節に散在する「実機で確認」項目を**全てクローズ**する。
- **Warsaw 固有項目はやらない**(2026-08-14 裁定): 旧 ROOT 互換 / grawToEventTPC 実機互換 /
  zCoBo リンク本数(ELITPC 筐体)/ 先方 LAN / zCoBo 台数構成(SPEC v1.22 §13-7 注記)は現地送り。
- **直前はコード凍結**(044 裁定)。凍結前に済ませる前提作業は Phase 0。

## Phase 0 — 持ち込み前(宅内、凍結前に完了させる)

| # | 項目 | 完了条件 |
|---|---|---|
| 0-1 | **067 実ストリーム対応**(topology frame 防御 / frameType 1 実データ照合 / 読み側耐性) | 067 が COMPLETED |
| 0-2 | 一晩 @224 Mbps / N=4 soak(§12-5(a) 完全形、064 の推奨) | 全カウンタ 0・RSS 平坦 |
| 0-3 | **現地手順のリハーサル**: 本計画 Phase 1〜3 の操作列を仮想スタック(start_demo.sh)で通しで 1 回実走し、所要時間と観測ログを本チケットに付記 | リハーサル記録あり |
| 0-4 | 現地持ち込み物の確定: 実 .graw 一式(オラクル照合用)・本計画の印刷/オフライン写し・`soak_evidence_054/prof_054_probe.cxx`(worker 数再計測用) | リスト固定 |
| 0-5 | 066 ハーネスの mini ジオメトリ切替スモーク(`.dat` 読み込み + forward モデルが mini 構成で回ること。トラック実データは現地 C-3 が初 — 2025 mini run はパルサーデータのため不可) | run_all.sh が mini .dat で exit 0 |

## Phase 1 — 接続・疎通(初日午前)

各項目は【確認手段】【期待値】【出典】。**観測値は全て本チケット結果節に転記する。**

1. **NW 経路**: MTU 9000 疎通(ping -s / iperf)。mini は 1GbE で足りるかの実測。
   【出典】SPEC §13-1,2
2. **listen-before-start**: receiver bind → links → `ecc start` → CoBo connect の順で成立。
   【確認手段】receiver ログの accept 時刻と ecc start 発行時刻の前後
   【出典】041(仮想で実証済み、実機再確認)
3. **DataSender id / flowType の最終確認**: `CoBo[0]`・大文字 `TCP` で configure が通る。
   【出典】SPEC §13-4
4. **flowType × topology frame の確定(032 と 067 の残矛盾を潰す)**:
   一次資料(GetBench MemRead.cpp `sendTopology`)では topology frame(frameType 7)は
   **FDT 接続のみ**送出。我々は TCP なので**来ないはず**だが、zCoBo 組込みビルドが
   VxWorks/2019 版と同一挙動かは未確認(032 の「唯一の推定」)。
   【確認手段】接続直後の受信バイト列(067 で decoder が frameType 7 をカウンタ+INFO で
   可視化済みのはず → カウンタ 0/1 で即断)【期待値】TCP なら 0。1 なら SPEC §13-4 を改訂
   【出典】032 / 067 / MemRead.cpp:362
5. **初回 configure の所要**: コールドスタートは分単位(実測 261 s の前例)を許容し慌てない。
   【確認手段】controller ログ `ecc command applied` の `elapsed_ms`(v1.20 常設)
   【出典】057(原因は未特定 — 現地の `elapsed_ms` 内訳が初の一次データになる)

## Phase 2 — run 制御(初日午後)

6. **歩き戻しの実機確認(最重要)**: 2 本目以降の run/start で logbook audit の
   `ecc_walk_back` が `["status->Ready","breakup->Prepared","reset->Described","reset->Idle"]`。
   `["status->Ready"]` 単独なら 034 事故の再来。`Off` 連発は ECC 不達を疑う。
   【出典】036 / SPEC §1.3
7. **連続 run ×10**(毎 run 完全リセットの作法で、オペレータ介入なし)。
   【確認手段】全 run の logbook + `next_run` 増分が連番【出典】034
8. **Arm bind 競合リトライ**: audit の `ArmRetry{attempts, waited_ms}` が仮想実測
   (最悪 4 試行 / 157 ms)から桁で乖離しないこと。現地マシンは遅い前提。
   【出典】034
9. **run 開始失敗時の `next_run` 巻き戻し**: わざと 1 回失敗させ(例: CoBo 未 configure で
   start)、番号が消費されないこと・500 連発にならないことを確認。【出典】034
10. **`ecc_error` の可視性**: 異常時に `/api/status` の `ecc_error`(例 `WHEN_PREPARE`)が
    UI に出る。`state=Off` 観測時は breakup 失敗(Ice UnknownException)/ onUnPrepare halt の
    例外由来の可能性を疑う — 復旧手順は「ECC 再起動 → describe からやり直し」を一次手段とし、
    実際に 1 回演習する。【出典】043 / CURRENT 保留事項
11. **run/start・run/stop 所要の実測**(仮想: start ≈7 s / stop 1.3 s)。60 s/REQ の余裕確認。
    【出典】033 / 036 / 057

## Phase 3 — データ経路・異常系(2 日目)

12. **stop 後のリンク挙動**: sm-stop 相当の後、receiver `GetStatus.metrics.peer` が
    non-null のまま(= 実機も close しない)か。breakup で null に落ちるか。
    【期待値】stop で close せず → 強制 EOS が正規経路(`forced_eos:true` が常態)
    【出典】032 / SPEC §9.2
13. **`forced_eos:false` の意味論**: run 中に CoBo 側を殺す(電源断 or プロセス kill 相当)と
    自然 EOF で「normal に見える異常」になること、`forced_eos:false` がそのシグナルに
    なることを実機で 1 回再現。【出典】041 D-2
14. **S1 復旧リハーサル**: CoBo 電源断 → 再 configure → `extra_connections` 増分 +
    peer 付き warn → run 組み直しで回復。【出典】032
15. **データリンク本数**: run 中 `extra_connections` が 0 のまま(mini は当然 1 本の想定だが
    機械確認を記録に残す)。【出典】SPEC §13-7
    【前倒し実測】ELITPC 側は HIGS 2026 現地 netstat で **1 本を実測済み**(073 #7、
    SPEC v1.24)— 本項は「我々の DAQ + TCP 構成でも同様」の確認に縮小。
16. **オラクル照合**: 現地で取った run 1 本を graw レベルで持ち帰り実 .graw と同一処理
    (021 の compare 系)にかけ、バイト一致 / イベント数一致を確認。ZS を試す場合は
    frameType 1 経路の初実機検証になる(067-B の照合値と突き合わせ)。【出典】055 / 067

## Phase 2.5 — 較正ランメニュー(mini Unfolding 用データの自前取得、半日)

**mini は自前の機械なので Mikolaj 依存なし** — unfolding の応答パラメータは全部ここで取る
(066 裁定の縮退破りを含む)。DAQ テスト(Phase 1〜3)の run と同一滞在で取り切る。
解析ツールは全て完成済み(068 サーベイ / 066 ハーネス — mini ジオメトリ .dat 切替のみ)。
**注**: 手元の実 2025 mini run はパルサーデータであり(2026-08-18 ユーザー確認)、
トラック系の量(σ_T/σ_L/v_drift/PRF)は**現地データが初**になる。

| run | トリガ / 条件 | 分量 | 取れる量 | 解析 |
|---|---|---|---|---|
| C-1 pedestal | 内部 periodically(pedestals xcfg) | 2 本 × 10 分 | per-ch ベースライン・ノイズ | 068 survey(B) |
| C-2 pulser | AsAd 内蔵パルサー(pulser xcfg) | 2 本 × 10 分 | per-ch 相対ゲイン + **電子回路応答の平均波形**(FPN は飽和するので信号 ch を使う — 068 実証) | 068 survey(C)+ 平均波形 |
| C-3 トラック | **α 線源 or 宇宙線**(自トリガ = onMultiplicity、閾値は C-1 のノイズから設定) | 数時間、**選別後 ≥300 イベント目標**(066 実績: 選別効率 ~9% → raw ≥4,000、余裕を見て 10,000) | ①σ_T・σ_L(066 ハーネス)②strip PRF(電荷シェアリング vs トラック位置)③**イオンテール**(孤立ヒット平均波形 − C-2 の電子回路応答) | 066 run_all.sh(mini .dat)+ 小解析 2 本 |
| C-4 ドリフト端 | C-3 と同一データ(全ドリフト長を跨ぐトラック — 宇宙線が最適) | 追加ランなし | **v_drift = L / t_max**(時間分布の両端 ↔ 既知ドリフト長 L)— 066 で実証した (strip,cell) 空間の厳密縮退を**検出器の物理的な縁で破る** | 小解析 1 本 |
| C-5 絶対スケール | 既知エネルギー α 線源(可能なら三重 α) | C-3 に含めて可 | ADC/keV | ピークフィット |
| C-6 ガス条件 | 記録のみ | — | **確定済み**(CO₂ 130 mbar / working point #3 / E_drift 70 V/cm は**ユーザー確認 2026-08-18**、v_drift 0.390 cm/µs は確認済み (p,E) で一意 — `reference/logbook/_survey/SURVEY.md` §1。σ_L=0.94 mm・D_L=熱限界一致まで導出済み)。現地は**当日の実条件の転記のみ**。最終クロスチェックは C-4 のドリフト端法が兼ねる | 机上(確定) |

**持ち帰り判定**: C-1〜C-6 が揃えば mini の forward モデル(σ_T, σ_L, v_drift, ゲイン表、
電子回路応答 + イオンテール)が閉じ、unfolding 本実装ユニットの起票条件が成立する。

## Phase 4 — 性能・耐久(3 日目〜)

17. **AGET 読み出しデッドタイム実測**(~1.4 ms/event 想定)と 100 Hz 実トリガでの取得。
    【出典】SPEC §13-3
18. **ディスク持続書き込み**: 実運用ストレージで 28 MB/s(mini)持続。【出典】SPEC §13-6
19. **recorder worker 数の現地再計測**: `prof_054_probe.cxx` で隔離プローブ → N を決める
    (M4 Pro より遅い前提、決め打ちしない)。【出典】055
20. **root-sink RSS とキュー**: 現地マシンのメモリ量を記録し、`--queue`(個数建て)の
    バイト建て化 or 既定値変更の裁定材料を持ち帰る(宅内実測: burst 時ピーク 6.88 GiB)。
    【出典】CURRENT 保留(SPEC 検討残 1 件)
21. **フル 24 h soak(ハード込み、§12-5(b))**: 031 と同じ様式(RSS/fd/カウンタ定期
    サンプリング、後半半分で単調増加なし)。ビーム無しでも外部トリガ源(パルサー/クロック)で可。
    【出典】031 / SPEC §12 表 5

## 現地で確定させる問い(明示リスト — 曖昧なまま帰らない)

- Q1: TCP 経路に topology frame は来るか(→ #4。SPEC §13-4 の確定)。
- Q2: zCoBo 組込みビルドの stop/close 挙動は VxWorks 版ソースの読みどおりか(→ #12)。
- Q3: 初回 configure 261 s の再現有無と `elapsed_ms` 内訳(→ #5)。
- Q4: 現地マシンの実力(CPU/mem/disk)と worker 数・キュー上限の適正値(→ #19, #20)。

## 出口

全項目の観測値を本チケット結果節に記録 → SPEC §13 の該当項目を「実測済み」へ改訂(Fable)→
Warsaw 展開(P5)の前提が揃う。**1 項目でも黒なら現地で潰すか、要改修として持ち帰り
チケット化してから離脱する**(「たぶん大丈夫」で帰らない)。

## 結果(第 1 弾: 2026-09-14 遠隔ベンチ実測 — GDED + 実 zCoBo + AsAd 内蔵パルサー、Fable 5.1 主導)

**構成**: DAQ ホスト GDED(Ubuntu 24.04、192.168.4.2/enp1s0f1 MTU 9000)上に tpcdaq-rs を
ネイティブビルド(receiver/decoder/graw_writer/controller + 新規 `null_sink` = ROOT 不在の
root-sink 代役)。実 ECC = getEccServer 20190315 をホストでネイティブ起動(sudo/Docker 不要、
Ice 3.7.2 は VM から採取 — 手順はメモリ geteccserver-native-gded)。ecc_bridge は Mac 上、
SSH 逆トンネル(GDED:47200 → Mac)で controller から到達。config_id =
{describe=zCobo, prepare=pulser, configure=pulser}(`prepare-pulser.xcfg` は configure の写しを追加)。
GET 純正 `getEccClient` で `asad-pulser-start -1000 1000`(triggerMode=onSlowControl)。

| # | 項目 | 実測 | 判定 |
|---|---|---|---|
| 1 | NW 経路 | GDED→zCoBo(4.81) DF ping 8972 OK、1500 OK、リンク 1000Mb/s | ✅ |
| 2 | listen-before-start | receiver `armed — listening` 11:06:13.316 → ECC start 11:06:20.56 → `CoBo connected peer=192.168.4.81:33145` | ✅ |
| 3 | DataSender id / flowType | `CoBo[0]` + `TCP` で configure → Ready(2,275 ms) | ✅ |
| 4 | topology frame(TCP) | decoder `topology_frames=0`、graw-writer `ctrl_frames=0`(run1/run2 とも) | ✅ **Q1: TCP では来ない** |
| 5 | 初回 configure 所要 | describe 67 ms / prepare 4,896 ms / configure 2,275 ms / start 151 ms(run/start 全体 7 s) | ✅ **Q3: 261 s は再現せず** |
| 6 | 歩き戻し | run2 の audit `ecc_walk_back = ["status->Ready","breakup->Prepared","reset->Described","reset->Idle"]` | ✅ |
| 11 | start/stop 所要 | run/start 7 s・8 s、run/stop 3 s | ✅ |
| 12 | stop 後リンク | `forced_eos:true` / `eos_closed:true` / reason normal(強制 EOS が正規経路として動作) | ✅(peer 観測は下記) |
| 15 | データリンク本数 | `extra_connections=0`、`empty_connections=0`(2 run とも) | ✅ |

**データ**: run1 = 122 frames / 34,011,648 B(= 122 × 278,784 ちょうど、~1 Hz pulser)、
run2 = 20 frames / 20 s。フレーム = frameType **2 / rev 5(compact)**、metaType 0x08、
278,784 B = 256 + 139,264 items × 2 B(全 ch 読み出し)。decoder `malformed=0`、
`fragments_out = frames_in`、receiver `overflow_frames=0`、graw-writer `write_errors=0`。
出力 = `out/run0001/CoBo0_AsAd0_2026-09-14T14:06:21.714_0000.graw`(実機命名)。
ECC ログ副産物: AsAd 温度 Tint 28 °C / Text 29 °C(stop 時読み出し)。

**未実施**: #7(連続 ×10)、#8/#9/#10、Phase 3 #13/#14/#16(オラクル照合は .graw を持ち帰り次第)、
Phase 4。root-sink(ROOT)は GDED に無く未検証 — run.root 経路は次回。

## 結果(第 2 弾: 2026-09-23 遠隔ベンチ — Rust ecc_bridge、実 ECC ネイティブ、VM の GET 停止、Fable)

| # | 項目 | 実測 | 判定 |
|---|---|---|---|
| 7 | 連続 run × 10 | run 15–24(各 20 s、パルサー −300 mV): run/start 7.89–7.92 s、run/stop 3.63–3.64 s、**10 本とも `ecc_walk_back = [status->Ready, breakup->Prepared, reset->Described, reset->Idle]`**、20 frames / malformed 0 / eos_closed true / reason normal | ✅ |
| 7'' | ACM(**300 s** アイドル後の run 26) | 12:57:41 → 13:02:41 に run/start 8.02 s で成功、20 s run 正常。ブリッジ log の `connected to Ecc` は起動時の 1 回のみ = **5 分のアイドルでも Ice 接続は切られない**(この ECC は ACM で閉じない、または往復が無くても保持)。再接続経路は依然未実証(意図的に ECC を再起動して試す = #14' で) | ✅ |
| 7' | ACM(90 s アイドル後の run 24) | run 23 停止 12:49:20 → 90 s 待ち → run 24 開始 12:50:50、**再接続なしで成功**(bridge ログに切断・再接続の記録なし)。Ice 3.7 サーバ既定 60 s の ACM 切断は**発生しなかった** — 再接続経路は未実証のまま(次は 5 分以上のアイドルで) | ✅(条件付き) |
| 6' | 歩き戻し(Rust ブリッジ) | 076 結果節: C++ 時と同一 | ✅ |
| 3' | 生成 xcfg(078) | prepare / configure が `configure-pulser__<sha8>.xcfg` で通る(078 結果節) | ✅ |

| 15' | 余分な TCP 接続(run 25、GDED から `nc 192.168.4.2 46005` を 15 s) | `extra_connections` 0 → **1**、receiver WARN「a second data link arrived while one is already up — closing it at once」、本物のリンク(peer 192.168.4.81:40225)は無傷で run 継続(25 frames、malformed 0) | ✅ |
| 12 | stop 後のリンク | run/stop 後の receiver `peer` = **null**(receiver の Stop がこちら側でリンクを閉じる = 強制 EOS が正規経路、`forced_eos:true / eos_closed:true`)。CoBo 側が自分から close しないことは 09-14 の観測どおり | ✅(意味論確認) |
| 13 | CoBo 側の kill(run 27、13:28:58 に板で `S99gethwserver stop`) | receiver: 即 `TCP EOF — end of stream` → EOS を両リンクへ送出、`peer` null。run/stop → **`forced_eos:false` / `eos_closed:true`**(自然 EOF = 「normal に見える異常」のシグナルが立つ)。ただし `ok:false, reason: abort:ecc-stop-failed`(ECC の stop が死んだ板を待って 60 s 超 → bridge REQ timeout)。receiver に「downstream is full — blocking on backpressure」→ teardown で 1 通 abandoned(counted、= EOS 後のハートビート。要確認 → 092) | ✅(意味論) |
| 14 | S1 復旧(板で `S99gethwserver start` → 2 s で 46001 open) | run/start の歩き戻し: `status->Ready` → **`breakup` が失敗し ECC が `Off`(WHEN_BREAKUP、ice UnknownException)** → run/start は 56 s で abort。controller の `reset` ×2 でも Off のまま → **getEccServer を再起動**(Rust bridge は `connection lost` → 次の呼び出しで**自動再接続**)→ run/start 7.5 s で `status->Idle` から describe/prepare/configure/start → run 28(15 frames、malformed 0、正常停止)。**復旧手順 = 板 `S99gethwserver start` → ECC 再起動 → run/start** | ✅ |
| 7''' | Ice 再接続経路 | #14 の ECC 再起動で bridge が `connection lost … closed the connection while reading message header` を status に出し、次の run/start で再接続成功 = **再接続経路を実機で実証** | ✅ |
- zCoBo の root ssh: host key は起動ごとに変わる(ユーザー: 仕様)。GDED から `python3 ~/tpcdaq-rs/zcobo.py "<cmd>"`(pty でパスワード投入、`StrictHostKeyChecking=no` + `UserKnownHostsFile=/dev/null`)で非対話実行できる。パスワードはリポに書かない。

構成・手順 = メモリ geteccserver-native-gded(09-23 追記)。**残**: #8–#10(zCoBo 側の切断は遠隔では不可、ECC halt と receiver 余分接続は遠隔で可能 — 次回)、
#13/#14、#16(compact のオラクル照合 = 今日の run 6–11 で可)、Phase 4(root-sink は新機)。

## #16 発注(compact フレームの MFM オラクル照合 — Sonnet、2026-09-23、Mac 側)
- 目的: 実機 compact(frameType 2 / rev 5、blkSize 256、`CoboFormats-Rev-5-Compact.xcfg`)の実データを **GET 純正 MFM ライブラリ**で復号し、我々の decoder と全一致を示す
  (067 B は frameType 1 で実施済み。compact は未照合 = SPEC §12-1 の穴)。
- 照合元: `reference/_spike/prefix/lib/libMultiFrame.dylib`(既存ビルド成果物、本体の絶対パス)に直接リンクする使い捨てダンパ(C++、`reference/_spike/mfm_compact/` に置く、
  gitignore、リポにはコミットしない)。経路 = 067 B と同じ(`mfm::FrameDictionary::addFormats(reference/config/CoboFormats.xcfg)` → `mfm::Frame::read` → item 走査)。
  compact の item は `<Item>` 定義が sample のみ(chan/bucket はブロック構造から決まる)なので、CoBoFrameViewer の compact 経路(`CoBoEvent::decodeSamples` の compact 分岐)を読んで同じ順序で並べる。
- 対象: `/Users/aogaki/Workspace/tpcdaq-rs/reference/exp_data/2026/d3_pulser_scan/run0011/*.graw`(60 frames、278,784 B 固定)と run0006。
- 我々側: `graw_replay` / `decoder` の出力ではなく **`src/decode.rs` を直接呼ぶ小さな Rust テスト**(`tests/decode_real_compact.rs`、env `TPCDAQ_EXP_DATA_2026` 配下のときだけ)で、
  frame 毎の header(frameType/rev/cobo/asad/eventIdx/eventTime/readOffset/status/mult/windowOut/lastCell)と全 item(aget, ch, bucket, sample)の **sha256** を出し、
  ダンパの同形式 sha256 と一致させる(ダンパは JSON/CSV でヘッダと sha256 を出す。全 item を吐くと巨大なので sha256 で比較)。
- 受け入れ: 2 run × 60 frames で全一致(ヘッダ全項目 + item sha256)、item 総数 60 × 139,264。オラクル値を 1 ずらすと赤になること。`cargo test` green、fmt / clippy。
  結果節に照合表(067 B と同じ体裁)を書く。

## 結果(#16 — compact の MFM オラクル照合、2026-09-23、Sonnet レーン)

**照合元 = GET 純正 MFM ライブラリの実リンク**(067 B と同じ経路、フォールバック不要)。
使い捨てダンパ `reference/_spike/mfm_compact/mfm_compact_dump.cpp`(gitignore、リポ非コミット、
本体側 `/Users/aogaki/Workspace/tpcdaq-rs/reference/_spike/mfm_compact/` に配置)を
`reference/_spike/prefix/lib/libMultiFrame.dylib` へ直接リンクしてビルド
(`clang++ -std=c++14`。`std::auto_ptr` 依存のため C++14 止まり — 使い捨てスクリプトなので
`tools/` の C++17 規約は適用しない。SDK は既定の Xcode CLT で問題なし、`SDKROOT` 明示は不要だった)。
経路は CoBoFrameViewer の `CoBoEvent::fromFrame()`/`decodeSamples()` の compact 分岐
(`reference/20190315_patched/CoBoFrameViewer/src/get/CoBoEvent.cpp:187-259`)と同一 ——
`mfm::FrameDictionary::addFormats(reference/config/CoboFormats.xcfg)` →
`mfm::Frame::read` → `frame.headerField(...)` / `itemAt(i).field("").bitField("agetIdx"/"sample")`、
ch/bucket は per-AGET カーソル(numChannels=68)で full-readout 順から復元 —— これは
`src/decode.rs` の `decode_items` compact 分岐と完全同一のアルゴリズムであることをソース比較で確認した。

**ハマった点(オラクル側のバグ、我々の decoder とは無関係)**: `in.peek() != EOF` をループ条件にすると、
60 フレーム目(ファイル末尾ぴったり)を読み終えた直後に `peek()` が偽陽性で non-EOF を返し、
存在しない 61 本目を `Frame::read` しようとして `ios_base::failure`(libMultiFrame.dylib 側の
古い ABI と衝突し `try/catch(std::exception&)` で捕まらず `libc++abi: terminating due to uncaught
exception` で落ちる)。ファイルサイズを事前に `tellg()` で確定し `while (tellg() < fileSizeB)` に
変えて解消(60 フレームちょうどで正常終了)。

対象 = `reference/exp_data/2026/d3_pulser_scan/run0011/CoBo0_AsAd0_2026-09-23T12:13:30.285_0000.graw`
と `run0006/CoBo0_AsAd0_2026-09-23T12:07:16.744_0000.graw`(いずれも単一 CoBo0/AsAd0、
topology frame なし、16,727,040 B = 60 × 278,784 B ぴったり)。

我々側 = `tests/decode_real_compact.rs`(`TPCDAQ_EXP_DATA_2026` 配下のときだけ走る)。
`Framer` + `Decoder::decode` の出力(`Fragment`)から、ダンパと同一の正規化バイト列
(ヘッダ固定順 LE + `Fragment::items` そのまま = `pack_item` 済み u32 LE 連結)を組み立て、
60 フレーム分を 1 本の SHA-256 に畳んで比較する(item を全部文字列比較すると巨大なので、
発注書の指示どおり sha256 で照合。frame0 のヘッダ・先頭/末尾 item は個別 assert でも二重に固定)。

### 照合表

| # | 項目 | run0011 | run0006 |
|---|---|---|---|
| データフレーム数 | オラクル/我々 | 60 / 60 | 60 / 60 |
| フレーム長 | オラクル/我々 | 278,784 B 固定 / 278,784 B 固定 | 278,784 B 固定 / 278,784 B 固定 |
| itemCount/frame | オラクル/我々 | 139,264 / 139,264 | 139,264 / 139,264 |
| item 総数 | オラクル/我々 | 8,355,840 / 8,355,840 | 8,355,840 / 8,355,840 |
| malformed / unsupported | 我々 | 0 / 0 | 0 / 0 |
| frame0 ヘッダ全項目 | オラクル | frameType 2 / rev 5 / cobo 0 / asad 0 / eventIdx 0 / eventTime 116,416,830 / readOffset 0 / status 0 / mult [1,2,4,8] / windowOut 0xFFFFFFFF / lastCell [690,690,690,690] | frameType 2 / rev 5 / cobo 0 / asad 0 / eventIdx 0 / eventTime 116,437,331 / readOffset 0 / status 0 / mult [1,2,4,8] / windowOut 0xFFFFFFFF / lastCell [690,690,690,690] |
| frame0 ヘッダ全項目 | 我々(`Fragment`) | 全一致 | 全一致 |
| frame0 先頭 10 item (aget,ch,buck,adc) | オラクル/我々 | (2,0,0,416)(2,1,0,402)(3,0,0,476)(3,1,0,449)(0,0,0,477)(0,1,0,460)(1,0,0,362)(1,1,0,429)(3,2,0,403)(3,3,0,474) — 全一致 | (1,0,0,368)(1,1,0,432)(2,0,0,430)(2,1,0,404)(3,0,0,461)(3,1,0,449)(0,0,0,486)(0,1,0,468)(2,2,0,402)(2,3,0,397) — 全一致 |
| frame0 末尾 5 item | オラクル/我々 | (3,67,511,472)(0,66,511,560)(0,67,511,535)(1,66,511,498)(1,67,511,445) — 全一致 | (2,67,511,452)(3,66,511,429)(3,67,511,446)(0,66,511,541)(0,67,511,516) — 全一致 |
| **item sha256(60 フレーム全部、ヘッダ+item 込み)** | オラクル/我々 | `a19c01f9…e34b` / **一致** | `4ab94f85…283e` / **一致** |

### オラクルを 1 ずらす赤の実測(TDD の赤確認)

`run0011` の期待 sha256 の末尾 1 桁を `b→c` に書き換えて `cargo test --test decode_real_compact`
を実行 → `real_compact_run0011_matches_the_mfm_oracle` が
`assertion `left == right` failed` で **FAILED**(run0006 は無関係のため引き続き ok)。
確認後に元の値へ復元(`git diff --stat` で無変化を確認済み)。

### 実行環境・コマンド

- 環境: macOS(Darwin 25.6.0)、実データ手元(`TPCDAQ_EXP_DATA_2026=/Users/aogaki/Workspace/tpcdaq-rs/reference/exp_data/2026`)、2026-09-23。
- ダンパビルド/実行:
  ```
  clang++ -std=c++14 -Wall -Wno-deprecated-declarations \
    -I<repo>/reference/_spike/prefix/include -pthread -DWORDS_LITTLE_ENDIAN=1 -DTIXML_USE_STL \
    mfm_compact_dump.cpp -o mfm_compact_dump \
    -L<repo>/reference/_spike/prefix/lib -lMultiFrame -lUtilities -lCompoundConfig -ltinyxml \
    -framework CoreFoundation -Wl,-rpath,<repo>/reference/_spike/prefix/lib
  ./mfm_compact_dump reference/config/CoboFormats.xcfg <run>.graw
  ```
- Rust: `TPCDAQ_EXP_DATA_2026=.../reference/exp_data/2026 cargo test --test decode_real_compact -- --nocapture`
  → `run0011: frames=60 items=8355840 malformed=0 unsupported=0 frame_sizes={278784} sha256=a19c01f9…`、
  `run0006: frames=60 items=8355840 malformed=0 unsupported=0 frame_sizes={278784} sha256=4ab94f85…`、
  **2 passed; 0 failed**。
- `cargo fmt` → 適用後 `cargo fmt -- --check` クリーン。`cargo clippy --tests -- -D warnings` → 警告 0。
- `cargo test`(フルスイート、実データ env 未設定 = 新規 2 本は skip 経路で ok): **846 passed; 0 failed;
  1 ignored**(58 バイナリ)。新規テスト追加前は 844 passed(本ユニットで +2)。
  新規テスト名: `real_compact_run0011_matches_the_mfm_oracle`、`real_compact_run0006_matches_the_mfm_oracle`
  (`tests/decode_real_compact.rs`)。
- スキップしたテスト: なし(該当 2 本は `TPCDAQ_EXP_DATA_2026` 未設定時のみ skip。実データ env
  設定下では上記のとおり実行され green)。

### 発注書からの逸脱

- 発注書は「ダンパは JSON/CSV でヘッダと sha256 を出す」としたが、実装はプレーンテキスト
  (`key=value` 行)にした——中身は同一情報(ヘッダ全項目・先頭10/末尾5 item・sha256)で、
  唯一の消費者が目視 + このドキュメントへの転記だったため JSON/CSV パーサを足す理由がなく、
  KISS を優先した。実質的な差分ではない。
- sha256 の対象を「frame 毎」ではなく「run 全体(60 frame)を 1 本の incremental digest に畳む」
  形にした。発注書の「frame 毎の header と全 item の sha256 を出し…一致させる」を素直に読むと
  60 行 × 2 run のハッシュ表になるが、それでは受け入れ条件の「2 run × 60 frames で全一致」を
  結局 1 個の bool として見るだけなので、run 単位の 1 digest に畳んでも検証力は同じ(あるフレーム
  のどこか 1 バイトでもずれれば digest が変わる)。frame0 は個別に header/first10/last5 を平文でも
  固定してあるので、可読性(CLAUDE.md「テストは仕様書」)は損なっていない。


## 処置(2026-10-13 クローズ、Opus — ユーザー「スタックしているのを確認して解決していきたい」)

現地出張の計画として書いたが、実際は **GDED 遠隔ベンチ(09-14 / 09-23)と ELITPC 本番機(10-08 以降、107)で実施**した。
項目ごとの所在は下の表のとおり。残りは [131](../131_onsite_and_hardware.md)(現地・機器・ELITPC 実機)と、CURRENT の「解析バックログ」へ移した。

| 項目 | 状態 | 所在・実測 |
|---|---|---|
| 0-1 067 実ストリーム対応 | ✅ | archive/067 COMPLETED |
| 0-2 一晩 @224 Mbps soak | 不要(実機で代替) | 064 の 30 分 @224 + 031 の一晩 @45 Mbps。実機では 182 Hz(≈ 102 MB/s)の全数取得(109)と 13.5 h の α run(107 run 2)。サンプリング付き 24 h は 131 A-3 |
| 0-3 現地手順のリハーサル / 0-4 持ち込み物 | 不要 | 出張せず遠隔で実施したため |
| 0-5 066 ハーネスの mini .dat スモーク | 移動 | 解析バックログ(較正解析と一緒に) |
| #1–#5 接続・疎通 | ✅ | 結果 第 1 弾(Q1: TCP に topology frame は来ない、Q3: 261 s は再現せず) |
| #6 / #7 歩き戻し・連続 ×10 | ✅ | 第 1 弾 / 第 2 弾(run 15–24) |
| #8 Arm の bind 競合リトライ | 移動 | 131 A-1(ELITPC の audit `arm_retries` を集計するだけ) |
| #9 start 失敗時の `next_run` 巻き戻し | 移動 | 統合テスト `a_failed_start_rolls_back_and_leaves_no_half_started_run`(034)は済み。実機確認は 131 A-2 |
| #10 `ecc_error` の可視性 | ✅ | 第 2 弾 #14: breakup 失敗で ECC が `Off`(WHEN_BREAKUP、Ice UnknownException)、ECC 再起動で復旧を演習 |
| #11 start / stop 所要 | ✅ | run/start 7.5–8 s、run/stop 3–3.6 s |
| #12–#15 リンク・異常系・復旧・本数 | ✅ | 第 2 弾。#13 の副産物 = 092(EOS 後の Heartbeat、2026-10-13 修正) |
| #16 compact の MFM オラクル照合 | ✅ | 本書 #16 結果(2 run × 60 frames、sha256 一致) |
| Phase 2.5 C-1 pedestal(mini) | 移動 | 081 の `calib` で随時取れる。解析バックログ |
| C-2 pulser | ✅ | archive/091(GDED、振幅スキャン) |
| C-3〜C-5 トラック・ドリフト端・絶対スケール | 一部 | データは 09-25 以降の α run(093–100、107)。α 線(²¹²Bi / ²²⁰Rn / ²¹⁶Po)の同定まで済み(101 追記 3)。σ_T・σ_L・PRF・イオンテール・ドリフト端の v_drift は未解析 → 解析バックログ。ドリフト端は 100 MHz の窓(≈ 24 mm)では見えない → 131 C-1(25 MHz に戻すか)と連動 |
| C-6 ガス条件 | ✅ | 机上で確定済み(本書の表) |
| #17 デッドタイム | ✅ | ELITPC: zCoBo の読み出し **2.931 ms / イベントで一定**(periodic、557 kB の frameType 1)= 上限 ≈ 340 Hz。約 1 s の不感時間は CAEN の NIM→TTL 変換器が原因(104 / 106 の訂正節) |
| #18 ディスク持続書き込み | ✅ | 182.5 Hz × 557,312 B = 101.7 MB/s を全数・連番で graw に(109)。ZFS の素の書き込み 226–328 MB/s |
| #19 recorder worker 数 | ✅ | workers 8 で root_sink ≈ 122 events/s(109 追記) |
| #20 root-sink RSS とキュー単位 | ✅(据え置き) | ELITPC はメモリ 123 GiB、182 Hz × 30 s の RSS 最大 3.1 GB(109)→ `--queue` は個数建てのままでよい |
| #21 24 h soak | 移動 | 131 A-3 |
| Q1–Q4 | ✅ | Q1 / Q3 は上記。Q2 = 第 1・2 弾(CoBo は stop で close しない、receiver の Stop が閉じる)。Q4 = ELITPC(20 スレッド / 123 GiB / ZFS)で workers 8 |

**出口の SPEC §13**: 3(デッドタイム)・6(ディスク)・8(HiVolta)を「実測済み」にする文案を CURRENT の Fable キューに積んだ(確定は Fable)。

**Status: CLOSED(2026-10-13)** — 実施分は上表、残りは 131 と解析バックログ。
