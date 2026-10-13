# 125 — Event Display に XY / XZ / YZ 投影(U/V/W 三面一致)を出す

**Status: IN PROGRESS — 改訂 2(下の「改訂 2」節が正。三面一致の投影は実機で失格)**(2026-10-13 起票 Opus。ユーザー「AsadCobo0Asad0 はあまり意味がない、2D トラック?」→
「閾値は固定でいい・XZ / YZ もあれば便利」。設計の確定は Fable)

## ユーザー裁定(2026-10-13)
1. Event Display の AsAd 電子回路座標パネル(077)は monitor タブでは意味が薄い → **XY(上から見た図)に置き換える**。
2. **XZ・YZ も出す**。
3. **ヒットの閾値は固定値**(pedestal 表を使えるなら使う、という含み付き)。設定場所は Opus 提案 =
   **Event Display タブの数値欄(ブラウザごと、localStorage に記憶)**。

## 設計判断(Opus 案。Fable 確認待ち)
- **投影の計算は UI(TypeScript)で行う**。monitor はジオメトリの mm 情報(strip の線分)を接続時に 1 回送るだけ。
  - 理由: (a) 閾値をブラウザごとに即時に変えられる(monitor で計算すると、閾値が全員共通になるか、
    クライアントごとに monitor の CPU が増える)。(b) 表示専用の変換で、保存系にもワイヤ(生 ADC)にも触らない。
    UI は既にベースライン減算(先頭 25 cell 平均)を表示専用でやっている(058)。(c) 毎秒の表示刻みで 1 イベントだけ計算すればよい。
- **pedestal 表(081 `calib pedestal` の JSON)は今は使わない**。Event Display は既にイベントごとに先頭 25 cell 平均を
  引いているので、「pedestal を引いた上で固定閾値」は成り立っている。失うのは ch ごとの k×σ だけ。
  pedestal 表は GET の設定(gain・peaking time)が変わるたびに取り直しが要り、古い表は黙って閾値を狂わせる。
  固定閾値で特定 ch のノイズが偽の点を作ると分かったときに、別チケットで足す。
- **既定値**: 閾値 50 ADC(TPCReco GUI `config_DotFinder_NGRAW.json` の `hitThr: 50`)、
  三面一致の許容半径 2.0 mm(TPCReco v0.09 `HistoManager::get2DReconstruction` の `radius = 2.0`、固定・UI に出さない)。
- **Z は time bucket のまま**(mm にしない)。ドリフト速度はガスと電場で変わり、.dat の `DRIFT VELOCITY` は古い値のことがある。
- AsAd 電子回路座標パネルは消さずに **Waveform タブへ移す**(dead / noisy ch や FPN を見るときに使う。データは同じ 0x03)。
  monitor タブは 0x03 を購読しなくなる(帯域が減る)。

## 移植元(reference/TPCReco、読み取りのみ)
- strip の線分: `TPCReco-HIGS2026_online/DataFormats/src/GeometryTPC.cpp`
  - pad_pitch = DIAMOND SIZE × √3、strip_pitch = DIAMOND SIZE × 1.5(:199-200)
  - strip_unit_vec[p] = (cos ANGLE_p, sin ANGLE_p)、pitch_unit_vec は U = −(W+V).Unit()、V = (U+W).Unit()、W = (V−U).Unit()(:271-283)
  - offset = off_strip × strip_pitch × pitch_unit[p] + off_pad × pad_pitch × strip_unit[p](:338-342)
  - **長さ = pad_pitch × (int)len_pads**(切り捨て。`StripTPC` は pad 数を `int` で受け取る — StripTPC.h:30 / StripTPC.cpp:33。
    `Load` の `double length` はデバッグ表示にしか使われない)。**`len_pads < 1.0` の行は TPCReco が捨てる** → 線分を作らず件数を warn。
  - **同じ (plane, section, strip) の重複は後勝ち**(`mapByStrip[...] = strip`、:352。ch 引きの mapByAget の先勝ちとは別)+ warn 1 回。
  - `GetCrossPoint`(:1556-1578)/ `MatchCrossPoint`(:1618-1632)。reference point は交点に最後に足す(平行移動なので先に足しても同じ)。
    線分上の有効範囲は `[−0.5·pad_pitch − tol, Length + tol]`、tol = `Utils::NUMERICAL_TOLERANCE` = 1e-5(UtilsMath.h:11)。
  - **rangeXY = TPCReco `rangeXY()` = TH2Poly の軸**(Get2D が使う値)。`InitTH2Poly` は線分の始点 `ref + offset − 0.5·pad_pitch·unit`・
    終点 = 始点 + `Length·unit` の外接矩形で TH2Poly を作るが(:552-560)、`SetFloat(true)` のため strip の多角形(菱形 pad の角)を
    足すと軸が広がる(:578 以降、SetTH2PolyStrip :636-680)。**最終的な軸の範囲に合わせる**(2026-10-13、レーン A の指摘で修正。
    mini の X は中心線の外接矩形 ±53.25 → TH2Poly ±53.75、ELITPC の Y は ny 112 → 113)。
- 投影: `tpcreco-v0.09_01_04_2022/DataFormats/src/EventTPC.cpp` `EventTPC::Get2D`(:1239 から)。
  time bucket ごとに U×V×W の全組(各 strip の全 section)で `MatchCrossPoint`、同じ (u,v,w) は最初の一致だけ採る。
  電荷の配分は **method 0(各 strip の電荷をその strip の一致数で等分)**。**TPCReco GUI の既定は method 1**
  (`EVENTTPC_DEFAULT_RECO_METHOD 1`、v0.09 EventTPC.h:21)だが、**表示では意図的に 0**(KISS)。HIGS2026 版では同関数はコメントアウトされている。
- 既知の差(意図的、KISS): TPCReco はクラスタ(SigClusterTPC: seed + 近傍)からヒットを取るが、こちらは**閾値を超えたセルだけ**。
  section を合算した strip はノイズが √n 倍になるので固定閾値がやや甘い(許容)。

## WS 契約(SPEC §10.3 に追加 — 両レーンはこれに従う)
S→C の JSON `stripGeometry`。**接続時に 1 回**(`meta` の直後、reliable 側の JSON キュー)。monitor 起動時に 1 回だけ計算する。
```json
{"type":"stripGeometry",
 "padPitch":1.7320508, "stripPitch":1.5,
 "rangeXY":[xmin,xmax,ymin,ymax],
 "planes":{
   "U":{"unit":[ux,uy],"pitchUnit":[px,py],"segments":[[strip,section,x0,y0,length], ...]},
   "V":{...},"W":{...}}}
```
- `x0,y0` = reference point + offset(mm)。線分の有効範囲は (x0,y0) から `unit` 方向に `[−0.5·padPitch, length]`(上の tol 込み)。
  `length` = padPitch × (int)len_pads。`strip` は 1 始まり(0x02 Uvw の strip 番号と同じ)。
- 同じ (plane, section, strip) に複数行があっても線分は 1 本(**後勝ち** — TPCReco mapByStrip と同じ)。並びは plane → strip → section の昇順。
- `rangeXY` = TPCReco `rangeXY()` = TH2Poly の軸(上の「移植元」)。
- 大きさ: ELITPC で線分 ≈ 3,300 本 ≈ 200 KB。reliable 側の JSON キューが 1 通 200 KB を通せること(容量・1 通の上限)をテストで確かめる。
- JSON と live(0x02)は別キューで**順序の保証はない**。UI は 0x02 が先に来ても「stripGeometry 未着」を出すだけで壊れないこと。
- ヘッダの ANGLES / DIAMOND SIZE / REFERENCE POINT のどれかが無い・strip 行が 0 本 →
  `{"type":"stripGeometry","unavailable":"<理由>"}` を送り、monitor は warn を 1 回出す(silent 禁止)。UI は理由をそのまま表示する。

## レーン A — Rust(ジオメトリの mm と WS 送出 + TPCReco オラクル)— Opus
**作業ブランチ**: worktree で `git merge --ff-only main` してから着手。
1. `src/geometry.rs`: NEW(10 欄)と LEGACY(7 欄)の strip 行で、今捨てている `off_pad` / `off_strip` / `len_pads` を保持する
   (ch 表に採られた行をファイル順に全部残し、`strip_xy` が `(plane, section, strip)` ごとに**後勝ち**で選ぶ —
   TPCReco の mapByAget(ch 重複は丸ごと捨てる)→ mapByStrip(後勝ち)と同じ。2026-10-13 主対話承認)。
   **lookup・dump_tsv(§4.5 の C++ 照合)の出力は変えない**。
2. 新モジュール `src/strip_xy.rs`(純関数、IO なし。**mm を扱うのはこのモジュールだけ** — SPEC §4.4 の例外の範囲):
   `Geometry` → 上の `stripGeometry` の中身(serde)/ 足りなければ `Err(理由)`。式は「移植元」のとおり。
3. monitor: 起動時に 2 を 1 回計算し、各クライアントの接続時に `meta` の直後に送る(既存の meta 送出と同じ経路)。
4. **オラクル** `tools/geometry_xy_oracle/`(C++17 + ROOT、`root-config`):
   新しい仕組みは作らず、**`tools/root_sink/Makefile` の流儀をそのまま使う**(`TPCDAQ_TPCRECO_DIR`、既定
   `../../reference/TPCReco/TPCReco-HIGS2026_online` から `GeometryTPC` と依存する最小限のソースを直コンパイル)。
   **`~/Workspace/Higs/TPCReco/build` のライブラリは使わない**(GeometryTPC.cpp が reference と違うローカル版)。
   .dat を読み、(a) 全線分(dir, section, strip, reference point + Offset, unit, Length)と rangeXY、
   (b) `MatchCrossPoint(radius=2.0)` の結果(true/false と点)を、各組の全 section の組み合わせについて JSON で出す。
   (b) の組は 2 種類: 乱数(seed 固定)1000 件 + **境界集合**(U×V 交点に最も近い W とその ±1〜2 本、線分の有効範囲の端 ±tol 付近)。
   合計の JSON は 300 KB 未満に抑える。Mac で ROOT を使う時は memory「Mac ROOT cling と CLT 27」(SDKROOT=MacOSX26.sdk)を参照。
   **TPCReco のソースはリポに入れない**(パスで指すだけ)。
5. **合成ジオメトリ** `tests/fixtures/geometry_xy_synth.dat`(手で/スクリプトで作る合成物。実 .dat 由来の行を入れない。
   DIAMOND SIZE / REFERENCE POINT は架空の値):
   ELITPC と同じ角度(0 / −120 / −60)で、**V・W に 2 section を持つ strip**、**len_pads が整数でない行**(切り捨ての確認)、
   **len_pads < 1.0 の行**、**同じ (plane, section, strip) の重複行**(後勝ちの確認)を含める。作り方を README かコメントに残す。
   4 の出力(合成ジオメトリ分)を `tests/fixtures/geometry_xy_oracle/{segments,match_cases}.json` としてコミットする。
6. テスト:
   - `strip_xy` 単体: 角度 90/−30/30・0/−120/−60・**11/22/33(六角でない — 既存 `geometry_mini_reduced.dat` の角度)**の手計算ケース
     (pitch_unit の向き、offset、長さの切り捨て、len_pads < 1.0 の除外、重複の後勝ち、rangeXY)。
   - 合成ジオメトリで 5 の segments.json と全線分・rangeXY 一致(許容 1e-9)。
   - **env-gated**: `TPCDAQ_REAL_GEOMETRY_XY`(実 .dat のパス)と `TPCDAQ_REAL_GEOMETRY_XY_ORACLE`(4 で作ったその .dat の segments.json)
     が両方あるときだけ全線分一致を確認(mini と ELITPC の 2 本で手元実行し、結果節に記録)。
   - 欠けたヘッダ → `Err`、monitor が `unavailable` を送る。
   - monitor の接続時に `meta` → `stripGeometry` の順で届く(既存の meta 接続テストに並べる)。
   - 200 KB 級の `stripGeometry` が reliable 側の JSON キューを通って欠けずに届く。
7. SPEC は触らない(主対話が Fable 確認後に入れる)。

## レーン B — UI(投影の計算・3 パネル・閾値欄・AsAd パネルの移動)— Sonnet
**作業ブランチ**: worktree で `git merge --ff-only main` してから着手。
1. `ws/json.ts`: `stripGeometry`(通常形 / `unavailable` 形)のパース。`meta` と同じく未知キーは許し、型違いは `bad-field`。
   `ws-client` に最新の `stripGeometry` を signal で持たせる。
2. 純ロジック `monitor/xy-projection.ts`(Angular / jsroot を import しない):
   - `crossPoint(segA, segB, unitA, unitB, padPitch)`・`matchCrossPoint(…, radius)` = 移植元の式そのまま
     (許容誤差 `NUMERICAL_TOLERANCE` の値は TPCReco の `Utils` を読んで合わせる)。
   - `projectEvent(uvw: [U,V,W] の 0x02, geometry, {threshold, radius})` →
     `{xy, xz, yz: {nx, ny, xmin, xmax, ymin, ymax, values(row-major, iy 外側)}, points, evaluations, capped}`。
     - 各面・各 strip でベースライン = 先頭 25 cell 平均(既存の表示と同じ関数を再利用)、値 − ベースライン > threshold のセルがヒット。
     - 3 面とも同じ eventNumber の 0x02 が揃っていないときは計算しない。
     - 全 time bucket について、3 面ともヒットがある bucket で、U×V の各線分の組の交点 p0 を `crossPoint` で求め、W の候補を絞ってから `matchCrossPoint`:
       各 W 線分について `n_W = (−unit_W.y, unit_W.x)`、`c = n_W·(x0,y0)` を前計算しておき(section ごと — 同じ strip 番号の section が
       一直線でなくてよい)、**候補 = `|n_W·p0 − c| ≤ radius + 1e-9`** の線分だけ(最終判定は `matchCrossPoint`)。
       根拠: V×W の交点は W の直線上にあり、`|p0 − p_VW| ≤ radius` なら p0 と W の直線の距離は radius 以下 = 必要条件。
       **「W の pitch 座標」で絞ってはいけない** — `pitchUnit_W` が `unit_W` に直交するのは六角配置のときだけ。
     - (u,v,w) の組は最初の一致だけ採る。一致数 n は TPCReco と同じく**重複チェックの前に**数える(section の組み合わせで複数回一致したら複数回)。
     - 電荷 = method 0: `q_u/n_u + q_v/n_v + q_w/n_w`(q はベースライン減算後の値)。XY に (x, y)、XZ に (x, bucket)、YZ に (y, bucket) で詰める。
     - ビン: X 幅 = stripPitch、Y 幅 = padPitch、範囲 = rangeXY(TPCReco と同じ `(int)((max−min)/pitch − 1)` 本)。Z = time bucket そのまま(nBuckets 本、−0.5〜nBuckets−0.5)。
       `values` の並びは 0x11 Histo2d と同じ **iy 外側 row-major(`values[iy*nx+ix]`)**。
     - 安全弁: **`crossPoint` の呼び出し回数**(= 評価回数)が **2×10⁶** を超えたらそのイベントは打ち切り `capped: true`
       (画面に「閾値を上げて」と出す)。2×10⁷ だとメインスレッドが 1〜2 秒止まる(Fable 見積もり)。
     - 0x02 の `incomplete` が立っているイベントは投影の状態表示にも `INCOMPLETE` を出す(fragment 欠落で合算値が落ちるため)。
3. `monitor-view`: Event display タブ
   - 1 段目 U/V/W(今のまま)、2 段目に **XY / XZ / YZ**(JsrootPanel、`colz;nostat`、軸 `X [mm]` / `Y [mm]` / `time bucket`)。
   - 数値欄「hit threshold (ADC above baseline)」既定 50。localStorage キー `tpcdaq.eventDisplay.threshold`、読み書きは try/catch(使えなくても既定値で動く)。
   - 1 行の状態表示: `N points (U/V/W coincidences) · threshold T ADC · radius 2 mm`。0 点・capped・`unavailable`・stripGeometry 未着はそれぞれ文で出す(silent 禁止)。
   - 計算は表示刻み(DisplayClock の tick)で 1 回だけ、同じ (event, threshold) なら再計算しない。タブが閉じているときは計算しない。
   - ヒント文に「表示専用。点 = 同じ time bucket の U/V/W 三面一致(TPCReco MatchCrossPoint、radius 2 mm)、Z は time bucket」と書く。
   - **AsAd パネルと `ws.setWaveforms(true)` を monitor-view から外す**。
4. `waveform-view`: AsAd 電子回路座標パネル(077 のもの、`asadPanelSpec` / `asadEventMismatch` を再利用)を移す。`≠ UVW` 表示も保つ。
5. テスト(vitest):
   - `crossPoint` / `matchCrossPoint` の手計算ケース(平行 → false、線分の外 → false、−0.5 padPitch の端、radius の境界)。
   - 合成ジオメトリ(テスト内で組む小さなもの)に直線トラックの 0x02 を作り、出てきた点がすべて真の直線から 1 stripPitch 以内。
   - **鏡像・回転を落とすテスト**(「直線から 1 pitch 以内」は鏡像でも通るため):
     (a) ビン index の手計算 — 既知の (x, y) / (x, bucket) / (y, bucket) を 1 点だけ詰めて `values[iy*nx+ix]` の位置を assert。
     (b) **原点を通らない非対称な直線**(例: 第 1 象限の (20, 5) mm から (60, 30) mm、bucket 100→200)で、点の象限と、
         bucket が増える向きに x・y が増えることを assert。
   - W 候補の絞り込みと総当たりで、乱数(seed 固定)イベント 200 個の結果が完全一致。**六角配置(0/−120/−60)と六角でない配置(11/22/33)の両方**で回す。
   - capped の挙動(上限 2×10⁶)、0 点の表示、incomplete の表示、閾値欄の既定値・保存・localStorage が throw しても動く。
   - `stripGeometry` のパース(通常 / unavailable / 型違い)。0x02 が `stripGeometry` より先に届いても例外を出さず「未着」を表示する。
   - monitor-view は 0x03 を購読しない、waveform-view に AsAd パネルが出る。
   - 性能: ELITPC 規模の合成イベント(3 本の飛跡、各面 1000 strip 前後)で `projectEvent` の所要を測って結果節用に報告する(合否ではない)。

## レーン C — 言語をまたぐ照合(A・B のマージ後)— Sonnet
1. Rust 側で合成ジオメトリの `stripGeometry` JSON をファイルに書く(§10.4 の生成器に足す、または同じ流儀の別出力)。
2. TS 側でそれを本番パーサで読み、`tests/fixtures/geometry_xy_oracle/match_cases.json`(TPCReco の結果)の全件について
   `matchCrossPoint` が true/false と点(1e-9)で一致することを確かめる。

## 各レーンの報告に必須
- 変更ファイル一覧、追加したテスト名(この発注書の各項目と対応づけて)、`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`
  / UI のテストとビルドの結果。
- **実行できなかった操作**(権限拒否・失敗したコマンド)。無ければ「なし」。

## 受け入れ(archive の前に必須)
- 上の自動テストがすべて green(レーン A・B・C)。
- **目視**: 実データで、線源・ビームの位置が期待どおりの象限に出ること(鏡像・回転はテストを通り抜けうるため)。
  (a) 手元: 実 .graw リプレイ(mini。ELITPC 検出器の .graw が手元にあればそれも)。
  (b) ELITPC 本番機(今は mini eTPC を接続)へ反映後、ユーザーが実イベントを見て確認。radius 2.0 mm で mini でも点が出ることを記録する。

## SPEC 差分案(v1.43 — Fable 条件付き承認 2026-10-13 の文面。実装マージ後に主対話が入れる)
- §4.4: 「**(v1.43)** mm の使用範囲を `src/strip_xy.rs`(strip 線分の mm を `stripGeometry` で配る、monitor 起動時 1 回)と
  UI の Event Display 投影(表示専用、TODO/125)まで広げる。DAQ・保存・§5.2 ヒストの hot path は引き続き strip/bucket のみ。」
- §5.4 AsAd 項: 「(v1.43)本パネルは **Waveform ビュー**へ移設。monitor タブの Event Display 2 段目は XY/XZ/YZ 投影
  (UI 計算、閾値 = ブラウザ毎の数値欄・既定 50 ADC、radius 2.0 mm 固定、Z = time bucket、TPCReco `Get2D` method 0 /
  `MatchCrossPoint` 準拠、crossPoint 評価 2×10⁶ 超で打ち切り表示)。」
- §10.3 行: 「`stripGeometry` | S→C(接続時 1 回、`meta` の次) | `{padPitch, stripPitch, rangeXY, planes:{U,V,W:{unit,pitchUnit,segments}}}`
  または `{unavailable}`」。`subscribe` 注記は「波形ビューを開いたクライアントだけ」に戻す。
- 履歴: v1.43(2026-10-13)。

## Fable 設計レビュー(2026-10-13、スポット Fable、読み取りのみ)— 条件付き承認
条件(すべて本発注書に反映済み): 長さの切り捨てと len_pads < 1.0 の除外 / 重複の後勝ち / rangeXY = InitTH2Poly の外接矩形 /
W 候補は W の法線距離で絞る(pitch 座標は六角配置でしか使えない)/ 打ち切り上限 2×10⁶ / 鏡像を落とすテスト(ビン index・非対称直線)/
実データの目視 / 0x02 先着の順序テスト。UI 側で計算する案・method 0・n_match の癖の維持・section 合流の扱い・オラクルの分量は OK。

## 改訂 2(2026-10-13 ユーザー指示「シンプルに macros にあるファイルでやってるアルゴリズムでお願いします」)— **こちらが正**

### 何が起きたか(実機の目視、run 71)
- 三面一致(TPCReco Get2D 方式)の XY は、**ドリフト方向にほぼ垂直な長い飛跡(この検出器の典型)で菱形のゴーストの塊**になった
  (スクリーンショット event 23618: 275,415 点)。飛跡全体が同じ time bucket に来るので、U・V・W の帯の交わり全体が
  「三面一致」を満たしてしまう。閾値を 300 に上げても消えない(手元で同じ計算を再現 — 原因は閾値ではなく方式)。
  加えて、ほぼ全 strip に飛跡の時刻で 35〜200 ADC の小さな山(halo)が乗る(analyzeUVW.C の STRIP_Q_MIN の注記と同じ現象)。
- macros の方式(下)を Python で同じイベントに当てると、ゴーストの菱形だった event 24857 / 24871 も直線 1 本になった。

### 新しい方式 = macros/analyzeUVW.C + macros/makeTracks.C の移植(**1 イベント = 直線 1 本**、UI で計算、表示専用)
- **macros/ は読むだけ**(ユーザーのファイル。書き換えない)。定数は macros と同じ値を TS の定数にする:
  ADC_THRESHOLD 35 / ペデスタル = cell 1〜20 の平均 / PEAK_HALF_CELLS 5 / STRIP_Q_MIN 2000 / BAND_MM 10 /
  MIN_VIEW_HITS 4(analyzeUVW の帯フィット)と 3(makeTracks)/ QUANTILE 0.02 / MIN_EIGEN_GAP_MM2 1.0 / MIN_PROJ 0.3。
- **z の mm 換算は macros の既定値**(1200 V / 20 cm / 100 mbar → v = 0.7233 × E/p = 0.43398 cm/µs、25 MHz → 0.173592 mm/cell)。
  帯選択と z フィットの内部だけで使い、**画面の Z は time bucket のまま**。
- **strip 位置 s**: macros は channel_map.csv の s_mm(mini 専用)を使うが、こちらは `stripGeometry` から
  `s = pitchUnit·(x0, y0)`(その strip の section 番号が最小の線分。TPCReco Strip2posUVW と同じ)。
  mini の 256 本で channel_map.csv と 0.0004 mm 以内で一致(CSV の丸め分)、pitchUnit も macros の PITCH 定数と一致(主対話で確認済み)。
- **makeTracks 手順 2 の一般化**: macros の閉じた式(Sxx = var_U …)は mini の pitch 専用。こちらは
  `var_v = p_vᵀ S p_v` の 3×3 連立を解く(mini の pitch では macros の式と同じになることをテストで確かめる)。ELITPC の角度でも動く。
- **1 strip 1 ヒット**: 0x02 は section を合算した strip × bucket の生 ADC なので、そのまま analyzeUVW の「1 ch」に当たる
  (mini は 1 section)。ピークは最初の最大(analyzeUVW のループと同じ同値の扱い)。
- **表示**: XY / XZ / YZ に、帯選択を通ったヒットのうち |pitch_v·d_xy| ≥ MIN_PROJ の view のものを、
  `(x, y) = anchor + l·d_xy`、z = そのヒットの z(time bucket)で詰める(重み = q)。ビンは従来どおり
  (rangeXY、X 幅 stripPitch、Y 幅 padPitch、Z = 512 bucket)。
- **状態表示**: ok なら `straight track: φ = …°, xy length … mm (2–98 % charge), strip hits U/V/W = a/b/c`。
  ok でなければ理由を文で(例「no track: V has 2 strip hits after the halo cut (needs 3)」「no xy elongation (track along the drift)」)。
  未着・unavailable・3 面が揃わない・INCOMPLETE は従来どおり。頭尻は決めない(macros の WARNING と同じ)。
- **閾値欄と localStorage は撤去**(定数は macros と同じに固定 — オフライン解析と同じ絵になることを優先)。
- **撤去するもの**: 三面一致の計算(xy-projection.ts の crossPoint / matchCrossPoint / projectEvent と W 絞り込み)、
  閾値欄(event-display-settings.ts)、レーン C の matchCrossPoint 照合部分と `tests/fixtures/geometry_xy_oracle/match_cases.json`、
  オラクルの match case 出力。**残すもの**: Rust の strip_xy / `stripGeometry`(s と rangeXY に使う)、
  オラクルの線分・rangeXY 照合、stripGeometry の言語をまたぐ照合(パーサ・rangeXY・線分)。

### レーン D — UI(macros 移植)— Opus
1. 純ロジック `ui/src/app/monitor/track-fit.ts`(Angular / jsroot を import しない): analyzeUVW の 0 / 1〜4 と makeTracks の 1〜5 の移植 + 表示用の点。
2. monitor-view: 上の表示・状態表示・ヒント文(「Display only. Same algorithm as macros/analyzeUVW.C + makeTracks.C: one hit per strip
   (peak ±5 cells, halo cut q ≥ 2000 ADC), one straight track per event; Z is the time bucket; head/tail not determined.」の趣旨)。
3. テスト(vitest):
   - 合成ジオメトリ(mini の角度 90/−30/30 と ELITPC の角度 0/−120/−60 の両方)に既知の直線の 0x02 を作り、φ が 0.5° 以内、
     2–98 % の端点が 2 mm 以内、z の傾きの向きが合う。**原点を通らない非対称な直線**で XY のビン位置と象限を assert(鏡像対策)。
   - halo(q < 2000)の strip が落ちる、帯から 10 mm 超離れた strip が落ちる、ok = 0 の各理由、ピークの同値は最初の最大。
   - 一般化した 3×3 の解が mini の pitch で macros の閉じた式と一致する。
   - **env-gated の macros オラクル**(下の 4)。
4. **macros オラクル**(env-gated、出力はコミットしない): `tools/macro_oracle/`(ROOT マクロか小さな C++。コミットする)で、
   `reference/exp_data/2026/elitpc_run70/run0070_raw_head200.root`(主対話が ELITPC の run70 から先頭 200 イベントを切り出したもの。
   mini eTPC、ローカルのみ)に**本物の macros/analyzeUVW.C(既定 100 mbar)と makeTracks.C を実行**し(macros/ を一時ディレクトリに
   コピーして、そこで実行。macros/ は書き換えない)、イベントごとの (a) adc[4][68][512]、(b) uvw の slView/slZ/slS/slQ、
   (c) tracks の全ブランチを JSON に書き出す。TS 側の env-gated テスト(`TPCDAQ_MACRO_ORACLE` = その JSON)が、adc から 0x02 を
   組み(macros/channel_map.csv の (aget, ch) → (view, strip))、mini の stripGeometry(`ws_proto_sample --geometry
   /Users/aogaki/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat --strip-geometry-out …`)を使って track-fit.ts を回し、
   ヒットの集合(view, s)・z・q と、ok・方向(符号は問わない)・端点(2–98 % と F、順序は問わない)・length を macros と比べる
   (許容は float32 で書かれた値に合わせて決め、結果節に書く)。一致件数(ok / not ok)を報告。
5. 撤去(上の「撤去するもの」)。Rust 側は match case の出力とフィクスチャの削除だけ(strip_xy・monitor は触らない)。
6. ゲート: cargo fmt / clippy -D warnings / cargo test、UI の ng test と ng build、`ui/run_ws_conformance.sh`。
7. 報告: 変更ファイル、各項目に対応するテスト名、オラクルの一致件数と許容、ゲート結果、**実行できなかった操作**(無ければ「なし」)。

## 結果

### 実装とマージ(2026-10-13、Mac、main = 3697c68)
- レーン B(Sonnet)51f4be8 → merge 8507a12。レーン A(Opus)2650aba + 追補 d74a843(rangeXY を TH2Poly の軸に)→ merge 526ed17。
  レーン C(Sonnet)296d52e(fast-forward)。発注書の修正 cb78d75、SPEC v1.43 3697c68。
- 各レーンの報告は `git show --stat` と発注書の各項目に対応するテスト名で照合してから受け入れた(CLAUDE.md)。

### ゲート(main 3697c68、Mac、2026-10-13)
- `cargo fmt --check` / `cargo clippy --tests -- -D warnings`: clean。`cargo test`: **1018 passed / 0 failed / 1 ignored**(着手前 992)。
- UI `npx ng test --watch=false`: **413 passed / 9 skipped**(skip = 生成器が要る適合 spec。従来の 5 + 新規 4)。
- `ui/run_ws_conformance.sh`: **422 passed / 0 skipped**(新規 `strip-geometry-conformance.spec.ts` の 4 本が実際に走ることを主対話で確認)。
- `npx ng build`: OK。

### オラクル照合(TPCReco HIGS2026_online の GeometryTPC を直コンパイル、ROOT 6.40.04)
| ジオメトリ | 線分 | 最大差 [mm] | rangeXY(X / Y) | nx / ny |
|---|---|---|---|---|
| 合成 `geometry_xy_synth.dat`(コミット済み) | 57 | 3.55e-15 | TH2Poly 範囲と 1e-9 一致 | — |
| mini `new_geometry_mini_eTPC.dat`(env-gated、ローカル) | 256(U72/V92/W92) | 1.42e-14 | ±53.75 / −57.591〜48.930 | 70 / 60 |
| ELITPC `geometry_ELITPC.dat`(env-gated、ローカル) | 1,018(U264/V376/W378) | 2.84e-14 | ±164.978 / ±98.75 | 218 / 113 |
- `MatchCrossPoint`(radius 2.0、tol 1e-5): **3,706 件(true 176 / false 3,530)すべてで、Rust が作った stripGeometry を TS 本番パーサで読み、
  TS 本番 `matchCrossPoint` が TPCReco と true/false・点(1e-9 mm)で一致**(レーン C)。乱数 2,095 件 + 境界集合 1,611 件(許容誤差の探針 W15–W18 は狙いどおり)。
- stripGeometry の大きさ: mini 15,364 B、ELITPC 59,831 B(見積もりの約 200 KB より小さい。200 KB 級の到達はテストで確認)。
- オラクルの当て物: TPCReco `GeometryTPC::LoadAnalog`(GeometryTPC.cpp:508)の sscanf が 1 要素の配列に書いてスタックを溢れさせる
  (ASan で確認)→ `sscanf_guard.h` を `-include` してその書式だけ幅を守って読み直す。線分・一致判定の結果は変わらない(Fable 確認)。

### 性能(レーン B、vitest / Node、Mac)
- ELITPC 規模の合成イベント(各面 1,001 strip、V・W 2 section、3 本の飛跡): 細い飛跡 1,230 点・評価 19,760 回で **2–3 ms**、
  太い飛跡(±3 strip)144,107 点・評価 1.99×10⁶ 回(上限ぎりぎり)で **100–145 ms**。上限 2×10⁶ の打ち切りで画面が止まるのは 0.1 s 程度。

### ELITPC への反映(2026-10-13 06:34Z、ユーザー了承「今すぐ反映」)
- run 71 は走らせたまま。bundle → `cargo build --release --bin monitor` → monitor だけ SIGINT → 再起動。
  起動ログ `strip segments ready for the XY projection (stripGeometry) u=72 v=92 w=92`。run 71 は継続(elapsed 4,988 s 時点で確認)。
- UI は Mac の `ng build` を `/opt/tpcdaq/ui` に差し替え(旧版は `/opt/tpcdaq/ui.prev`)。HTTP 200。
- Node の WS クライアントで確認: 接続直後に `meta` → `stripGeometry`(15,387 B、rangeXY = オラクルの mini 値と一致、unavailable なし)→ `status`、live のバイナリも流れている(4 s で 85 通)。

### 既知の差(承認済み・実 .dat では起きない)
- `LEN_PADS < 1` の行も geometry.rs では ch を占有する(TPCReco は ch 表にも入れない)。strip 番号 0 の行を受理する。aget=4 は我々では malformed(TPCReco は `aget <= AGET_Nchips` のバグで受理)。ANGLES 行が複数あると TPCReco は最初、geometry.rs は最後を採る。
- 同じ strip の section の走査順: TPCReco はファイル出現順、こちらは section 昇順。section 境界で両方が一致するときの「最初の一致」が入れ替わりうるが、同じ直線上の同じ交点なので位置差は丸め程度(Fable)。
- ビン index の浮動小数の演算順が ROOT と違い、境界ちょうどで 1 ビンずれうる(表示専用、無視)。
- 評価回数 = U×V の `crossPoint` + `matchCrossPoint` 内の 3 回(発注書の「crossPoint の呼び出し回数」どおり)。閾値欄は 0 以上の有限値だけ受け付け、不正値は元に戻す。

### Fable 完了レビュー(2026-10-13、スポット Fable、読み取りのみ、e94846d..3697c68)
**承認(archive 可、残りは目視のみ)**。must-fix なし。should-fix = 本節の「WS 契約」の rangeXY 文言(直した)。

### 残り(archive の前に)
- **目視(ユーザー)**: Event Display で XY / XZ / YZ に点が出ること(radius 2.0 mm で mini でも点が出るか)、線源の位置が期待どおりの象限に出ること。
- 主対話のブラウザでの目視はできなかった(Chrome 拡張が未接続)。

### 実行できなかった操作
- レーン A・B・C: worktree 隔離の権限チェックで複合コマンドが数回拒否 → 単純なコマンドに分けて実行し直した(未実行の操作は残っていない)。
  レーン C は一度 spec の書き込みが拒否されたのに気づかず green に見えた → 書き直して再実行(主対話が main で適合スクリプトを回し、新 spec が走ることを確認)。
- 主対話: Chrome 拡張が未接続でブラウザの目視ができなかった。
