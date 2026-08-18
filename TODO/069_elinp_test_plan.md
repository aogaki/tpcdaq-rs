# 069 — ELI-NP 実機テスト計画(mini eTPC、1 CoBo、徹底実施)

**Status: DRAFT**(起草 Fable 2026-08-18。ユーザーレビュー待ち。
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
| C-6 ガス条件 | 記録のみ | — | ガス種・圧力・ドリフト電場・温度(ログブック転記)→ D_L/D_T 理論値 → 066 の実測関係式 `v/f = 2·D_L / 0.0461` とクロスチェック | 机上 |

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
