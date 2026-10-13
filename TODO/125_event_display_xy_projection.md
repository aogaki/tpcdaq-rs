# 125 — Event Display に XY / XZ / YZ 投影(U/V/W 三面一致)を出す

**Status: IN PROGRESS**(2026-10-13 起票 Opus。ユーザー「AsadCobo0Asad0 はあまり意味がない、2D トラック?」→
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
- `rangeXY` = 上の「InitTH2Poly の外接矩形」。
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

## 結果
(未記入)
