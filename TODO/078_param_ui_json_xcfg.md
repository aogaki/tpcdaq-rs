# 078 — パラメータ設定 UI(JSON が正 → xcfg を生成)

**Status: READY**(2026-09-14 起票、Fable。**着手はユーザー指示待ち**。
ユーザー裁定(同日): **JSON を正とする** / **078 として起票** / ノブは「結局ほとんど全チップ同一」—
**普段はユーザーから隠れているが、あるチップだけ変えられる**形にする)

## 0. 結論(1 分で)

- **xcfg を UI で直接編集しない。JSON(自前スキーマ)を正とし、xcfg は生成物**。ECC は改変せず、
  実験側の正典 xcfg を**テンプレート**として丸ごと引き継ぐ(管理ノブ以外は一切触らない)。
- **ノブモデル = 「一様値 + 例外リスト」**。UI はノブを 1 回だけ見せる(= 全インスタンス共通)。
  例外(特定 AsAd / AGET / ch だけ別値)は折りたたまれた「Exceptions (0)」に入れ、**件数バッジと
  ログブックで常に見える**(隠れているが黙らない — CLAUDE.md の silent 禁止)。
- これは xcfg 自身の継承規則(`id="*"` が既定、個別 id が上書き — 072 §2)をそのまま JSON に
  写したもの。生成物も `*` 1 か所 + 例外分の明示ノードで済み、ELITPC 1,088 ch でも小さい。
- 手間 ≈ **2 週間 + レビュー**(レンダラ Opus 2〜3 日 / REST Sonnet 1〜2 日 / UI Sonnet 3〜4 日 /
  SPEC Fable)。

## 1. 背景(072 / 065 の事実)

- 3 層のうちチューニングで触るのは **configure-*.xcfg だけ**(describe は FW とペアで不変、prepare は
  mini では configure のコピー運用)。
- 常用ノブは triggerDelay の 1 個。実験切替級 = CKW / Gain / peaking、ラン種別 = triggerMode /
  readout モード、較正 = pulser と閾値(Global 3 bit + per-channel LSB 4 bit)。physics xcfg は 2022 年
  以来 delay 以外不変(065)。
- 「あるチップだけ変えたい」の現実例 = **068 が見つけた恒常異常 7 ch**(死んではいない、multiplicity
  トリガ / ZS 用の閾値を個別化する対象)。ノイズ ch のマスク・閾値個別化が例外リストの主用途。
- ECC は configure のたびに `configure-<id>.xcfg` をファイルから読む(ConfigFileManager、命名規則は
  2026-09-14 に実機で確認)。

## 2. JSON スキーマ(案 — 実装時に reference/config の現行 xcfg でパスを全数照合すること)

```json
{
  "schema": "tpcdaq-params/1",
  "template": "configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg",
  "knobs": {
    "triggerDelay": 1748,
    "writingClockFrequency_MHz": 25.0,
    "gain": "120fC",
    "peakingTime_ns": 232,
    "triggerMode": "onRequest",
    "readout": "full",
    "globalThreshold": 7,
    "lsbThreshold": 15,
    "zeroSuppression": { "enabled": false, "threshold": 0 },
    "pulser": { "enabled": false, "voltage_mV": -1000, "period_ms": 1000 }
  },
  "exceptions": [
    { "at": { "asad": 1, "aget": 2, "ch": 5 }, "knob": "lsbThreshold", "value": 3,
      "note": "noisy channel (068 #4), 2026-09-14" }
  ],
  "note": "HIGS2026 working point #3"
}
```

### ノブ定義表(スコープ = xcfg 上でその値が住む階層。UI はスコープを見せない)

| knob | スコープ | 型 / 範囲 | xcfg 上の位置(072 §3、要照合) | 出る相 |
|---|---|---|---|---|
| triggerDelay | CoBo | int(clock 単位) | CoBo/Module/triggerDelay | configure |
| writingClockFrequency_MHz | CoBo | enum 12.5/25/50/100 | CoBo/Module/writingClockFrequency | prepare + configure |
| gain | channel | enum 120fC/240fC/1pC/10pC | Aget/channel/Gain | configure |
| peakingTime_ns | channel | enum(AGET 仕様値) | Aget/channel/peakingTime | configure |
| triggerMode | CoBo | enum onRequest/onSlowControl/… | CoBo/Module/triggerMode | configure |
| readout | CoBo | enum full/partial(= isAllChannelRead + isFPNRead + hit register の合成) | CoBo/Module/… | configure |
| globalThreshold | AGET | int 0..7 | Aget/Global/GlobalThresholdValue | configure |
| lsbThreshold | channel | int 0..15 | Aget/channel/LSBThresholdValue | configure |
| zeroSuppression | CoBo | {enabled, threshold} | CoBo/Module/enableZeroSuppression + zeroSuppressionThreshold | configure |
| pulser | AsAd | {enabled, voltage, period} | AsAd/Generator/… | configure |

- **含めない**(072 §3.5 の構成・識別子系): AsAd `isActive`(mini 1 / ELITPC 4)、readoutDepth、
  nbEvent、Module.frameType(deprecated)。これらは**テンプレート選択**(mini / elitpc)が運ぶ。

### 2.1 マルチプリシティ系(ユーザー要望 2026-09-14「イベントリジェクションがあれば入れたい」— 調査結果)

**GET/CoBo には存在する**(一次資料 = CoBoNode.cpp / CoBoConfigParser.cpp / 現行 xcfg):

| 機能 | xcfg ノブ | 意味 |
|---|---|---|
| マルチプリシティトリガ | `triggerMode=onMultiplicity`、`multiplicityThreshold`(4 AsAd 合計)、`multWindowSize`(移動窓のサンプル数) | AGET のマルチプリシティ出力の合計が閾値を超えたら自己トリガ |
| **2p(two-proton)モード = CoBo で唯一の「イベント棄却」** | `enableMem2pMode`(SCA を 256+256 に分割)、`multiplicityThreshold_2p`、`triggerDeadTime_2p`(第 2 トリガ受付開始)、`triggerTimeOut_2p`(**「この時間内に第 2 トリガが来なければ第 1 トリガはキャンセル」**= CoBoNode.cpp `setTriggerTimeOut2p`)、`triggerDelay_2p`、`triggerTime_2p`、`*_2p` トリガモード | 2 段トリガ。timeout で半イベントを捨てる |
| chip 単位の除外 | AGET `suppressMultiplicity` | その chip をマルチプリシティ和から外す |
| **ch 単位の除外** | channel `TriggerInhibition` = none / inhibit_trigger(マルチプリシティに寄与しない)/ inhibit_channel(マルチプリシティにも hit register にも寄与しない) | AGET レジスタ 10/11。hit register にも効く = **partial readout の対象からも外れる** |
| ノイズ補正 | multiplicity moving average / deviation subtraction | AGET マルチプリシティ信号のベースライン補正 |

**ただし我々の zCoBo FW では動かない**: `reference/ZC706_20181031_ELINP/docs/README.txt` §5 Limitations
「The current firmware version does not handle triggering on MULTIPLICITY nor 2-PROTON mode.」
reference/config の `configure-multiplicity.xcfg` は 2015-02 付の GET 標準サンプル(zCoBo 用ではない)、
ログブック 5 年分(071 SURVEY)にマルチプリシティ run の記録は無い → **zCoBo で使われた実績なし**。
ECC はレジスタを書くが FW が無視する = **黙って効かないノブ**になる。

**078 での扱い(ユーザー裁定 2026-09-14「zCoBo で使えないものは要らない」)**:
1. **マルチプリシティ / 2p のノブ群はスキーマに入れない**(firmware-gated 案は撤回 — KISS)。
   `triggerMode` の enum は FW が扱える値だけ(external / onRequest / onSlowControl / periodically /
   noTrigger)。`onMultiplicity*` / `*_2p` はスキーマ違反として拒否(テンプレートにある値は非管理ノブ
   としてそのまま通る)。将来 FW が対応したら、そのときスキーマ版を上げて足す。
2. **`TriggerInhibition`(channel)は通常ノブとして採用**(FW 非依存の AGET レジスタで hit register にも
   効く)。例外リストと組み合わせた**ノイズ ch のマスク**(068 の恒常異常 7 ch)の正規の道具。
3. ソフト側のイベント棄却(decoder / root-sink での multiplicity カット等)は本ユニットの範囲外
   (保存系はロスレス — CLAUDE.md 絶対ルール。やるなら QA 再構成 074 のモニタ側で「印を付ける」まで)。
- `exceptions[].at` は knob のスコープ**以上の粗さ**を許す(例: `{"asad":1}` だけなら AsAd1 の全 AGET
  全 ch に効く = xcfg の `<AsAd id="1"><Aget id="*"><channel id="*">`)。スコープより細かい指定
  (triggerDelay を ch 単位等)はスキーマ違反として拒否する(黙って丸めない)。

## 3. レンダラ(Rust、`src/params/`)の規則

1. **管理ノブ**: テンプレートから当該ノブの**全インスタンスノード(`*` と明示 id の両方)を除去**
   → 一様値を `*` ノードに書く → 例外を明示 id ノードで書く(個別 id は `*` に勝つ = CompoundConfig の
   規則)。**除去を省くと、テンプレートに残った明示ノードが JSON の一様値に勝ってしまう**(最重要の罠)。
2. **非管理ノブ**: テンプレートのまま 1 バイトも変えない(意味論として。整形は統一してよい)。
3. **出力**: `configure-<id>.xcfg` と `prepare-<id>.xcfg`(同内容)。`<id>` = `<template stem>__<JSON の
   sha256 先頭 8 桁>`(同じ JSON → 同じファイル = 既存ファイルの上書きは起きない。072 の系譜の作法)。
   ECC の設定ディレクトリは設定(TOML `[controller] ecc_config_dir`)。
4. **有効値の解決(表示用)**は JSON 内で完結する(most specific wins)。CompoundConfig の継承解決を
   Rust に再実装しない — 管理ノブは JSON が正、それ以外は「テンプレート由来」と表示するだけ。
5. XML は `quick-xml`(または `roxmltree` + 書き手)。パスは XPath 風の内部表現 1 種類に固定。

## 4. controller / UI

- REST(§8.1 に追加): `GET /api/params` / `PUT /api/params {token, json}`(スキーマ検証 → 保存 →
  render → 生成物のパスと hash を返す)/ `GET /api/params/render`(dry run、生成 xcfg の diff)。
  `POST /api/run/start` は現在の params の `<id>` を configure/prepare の config_id として使う
  (§3.1 の 3 相 config_id の上に「params 由来」が乗る形。describe は不変)。
- **ログブック(§9)**: run/start の audit レコードに `params_hash` と生成 xcfg のパスを載せる。
  例外リストが非空なら **`exceptions: N` を run レコードに必ず出す**。
- UI(§11): 「Parameters」ビュー = ノブ 10 個のフォーム(型・範囲はスキーマ由来、焼き込み禁止)+
  折りたたみ「Exceptions (N)」テーブル(at / knob / value / note、追加・削除、"reset to uniform")+
  生成 xcfg のテンプレートとの diff 表示 + 保存は token 必須(操作権 1 名モデル)。
  status バーに `params <id> · exceptions N` を常時表示(隠れているが黙らない)。

## 5. 受け入れ

1. **往復**: `exceptions=[]` かつ knobs = テンプレートの現行値 → 生成 xcfg がテンプレートと**意味同一**。
2. **CompoundConfig オラクル**(一次資料はテストダブルではない): `tools/ccfg_resolve/`(C++、
   reference/20190315_patched/CompoundConfig を**ビルド時参照・コミットしない** — TPCReco と同じ流儀)
   で xcfg の有効値を任意パスで印字し、Rust レンダラの出力を**全インスタンス**(ELITPC 1,088 ch 含む)
   で照合する(env-gated)。除去規則 1 の負性テスト(テンプレートに明示ノードを仕込む)を含める。
3. **実 ECC**: 生成 xcfg で describe → prepare → configure が通る(CmdException 無し)。vcobo 構成で
   検出器なしに回す(041 の起動レシピ)。
4. REST / UI: 既存 controller 統合テスト + UI vitest に追加。スキーマ違反(範囲外、スコープより細かい
   例外)が 400 で返ること。
5. 変更したノブが**実データに現れる**ことを 1 つ実証(例: triggerDelay を変えて .graw ヘッダ /
  イベント時刻に反映 — 新 DAQ 機 + 実 zCoBo で)。

## 6. 見積もりとレーン

| レーン | 内容 | モデル | 日数 |
|---|---|---|---|
| A | スキーマ + レンダラ + 往復テスト + ccfg_resolve オラクル | Opus | 2〜3 |
| B | controller REST + run/start 結線 + ログブック | Sonnet | 1〜2 |
| C | UI(フォーム / 例外テーブル / diff / status バー) | Sonnet | 3〜4 |
| D | SPEC §3 / §8 / §9 / §11 改訂 + 完了時レビュー | Fable | — |

A → B → C の順(B/C は A のスキーマが固まれば並列可)。

## 7. リスク・注意

- **readout を partial にすると frameType が 1 に変わる**(067)— decoder は両対応だが、解析側の
  想定に波及するので UI で注意表示。
- CKW / PLL は prepare 段のノブ → prepare-*.xcfg も同内容で出す(mini は現状もコピー運用)。
- Ice の個別 op(閾値 / pulser)で run 中に変える経路は**本ユニットでは扱わない**(xcfg に残らず
  再現性を壊す)。較正スキャン用途は後続で「ログブック記録 + 次 run の JSON に反映」の規則付きで。
- テンプレートの更新(Warsaw 側の xcfg 改訂)は JSON に影響しない(レンダラが再適用する)— これが
  JSON 方式の最大の利点なので、テンプレートは reference/config から**無改変で**取り込む。
- xcfg 上の `<channel id="N">` は Aget 配下(0..67 = raw ch、FPN 込み)。JSON の `ch` も raw ch で
  統一し、ジオメトリの strip 番号は使わない(UI で strip → ch の逆引きを補助表示するのは可)。

## 8. 裁定済み / 残り

- ✔ JSON を正とする ✔ 078 ✔ 一様値 + 例外リスト(隠れているが黙らない)。
- 残り: §2 のノブ集合と名前の最終確認(実装時にテンプレート xcfg で全パス照合したうえで、
  Fable が SPEC §3 に固定する)。

## 9. レーン A 発注(2026-09-22、Opus)

- 着手前に worktree で `git merge --ff-only main`(ff できなければ固有コミットが無いことを確認して `git reset --hard main`)。発注書は本体の
  絶対パス `/Users/aogaki/Workspace/tpcdaq-rs/TODO/078_param_ui_json_xcfg.md` で読んでよい。
- テンプレート xcfg は worktree に無い(reference/ は gitignore): `/Users/aogaki/Workspace/tpcdaq-rs/reference/config/configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg`
  (+ `..._12.5MHz_232ns_trigDelay3576.xcfg`、`configure-pedestals-*.xcfg` 2 種、`configure-pulser.xcfg`、`describe-elitpc.xcfg`)。
  **コミットしない**。リポに入れるフィクスチャは**合成の縮小テンプレート**(同じ構造で AsAd 1 / AGET 1 / ch 数本)。
- 範囲 = §2 スキーマ + §3 レンダラ + §5-1 往復 + §5-2 オラクル(ccfg_resolve)。REST / UI(レーン B/C)は含めない。
- **最初の仕事 = §2 ノブ定義表のパス照合**: 各ノブについてテンプレートに実在する xcfg パス(要素名・id・属性・型・現行値)を
  `docs/PARAMS_KNOBS_ja.md`(新規、表)に書き出す。表と実物が食い違うノブ・見つからないノブは**実装せず報告**(Fable が SPEC §3 に固定してから)。
  合致したノブだけ実装する。
- 配置: `src/params/{mod,schema,render,paths}.rs`。XML は `quick-xml`(新規依存、MIT)。スキーマ検証は serde + 手書き範囲チェック(jsonschema crate は入れない)。
  生成物名 `<template stem>__<sha256 先頭 8>.xcfg`、configure / prepare 同内容(§3-3)。除去規則(§3-1)は**負性テスト必須**。
- オラクル `tools/ccfg_resolve/`(C++17、Makefile、`CCFG_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched/CompoundConfig` を
  ビルド時参照、コミットしない、make は `-j`)。Rust 側は env `TPCDAQ_CCFG_RESOLVE_BIN` があるときだけ照合テストを走らせ、無ければ skip を**出力に明示**。
  CompoundConfig の Mac ビルドが通らなければその事実(エラー要旨)を結果節に書き、往復 + 負性テストだけで green にする。
- 受け入れ: fmt / clippy / test green、production `.unwrap()` 無し。結果節に: 照合したノブ数 / 食い違い / オラクル照合の件数(全インスタンス数)。

## 結果(レーン A — スキーマ + レンダラ、2026-09-22、Opus レーン)

実行環境: macOS 26.6.2 (arm64) / rustc 1.97.1 / Apple clang 21.0.0 / boost 1.90(homebrew)。

### 実行コマンド

```
cargo fmt && cargo clippy --tests -- -D warnings && cargo test
make -j8 -C tools/ccfg_resolve CCFG_DIR=/Users/aogaki/Workspace/tpcdaq-rs/reference/20190315_patched/CompoundConfig
TPCDAQ_CCFG_RESOLVE_BIN=$PWD/tools/ccfg_resolve/build/ccfg_resolve \
TPCDAQ_XCFG_TEMPLATE=/Users/aogaki/Workspace/tpcdaq-rs/reference/config/configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg \
  cargo test --test params_ccfg_oracle -- --nocapture
```

### テスト数

**614 passed → 645 passed / 0 failed / 1 ignored**(+31)。clippy 警告ゼロ、production の `.unwrap()` 無し。
内訳: `tests/params_render.rs` 18(往復・除去規則の負性・例外・スキーマ違反・生成物名)、
`tests/params_ccfg_oracle.rs` 4(env 無しでは SKIP を stderr に出して green)、
`src/params/{paths,schema,render}.rs` の単体 9。

### ノブ照合(最初の仕事 — `docs/PARAMS_KNOBS_ja.md`)

`reference/config` の configure-*.xcfg 5 本 + describe-elitpc.xcfg で全パスを照合。
**照合 11 ノブ(§2 表の 10 + §2.1 裁定 2 の `TriggerInhibition`)→ 実装 7 / 食い違い 4**。

- 実装(7): `triggerDelay`(CoBo, `Module.triggerDelay`, unit=10ns)、`writingClockFrequency_MHz`、
  `triggerMode`(FW が扱える 5 値のみ)、`globalThreshold`(AGET, **`Global.Reg1.` が §2 表から抜けていた**)、
  `lsbThreshold`、`gain`、`triggerInhibition`。
- **未実装 = 要裁定(4)**: `peakingTime_ns`(実物は AGET スコープの `Global.Reg1.peackingTime`、綴りも違う)、
  `readout`(CoBo 1 か所ではなく `Module.enableWriteHittedregister` + AGET の `isAllChannelRead`/`isFPNRead`
  + channel の `Reading` の合成)、`zeroSuppression`(enabled は CoBo だが threshold は **channel** スコープ)、
  `pulser`(AsAd ではなく **CoBo** スコープの `Instance.Generator`)。詳細は PARAMS_KNOBS_ja.md §4。

### オラクル照合(§5-2)

`tools/ccfg_resolve` は **Mac でビルドできた**(C++11、後述の逸脱 2 点)。

- 合成フィクスチャ(AsAd 0..1 × Aget 0..1 × ch 0..5、CoBo instance 2 本): **158 インスタンス**を
  全数照合、一致。一様値をテンプレートと変え、3 スコープすべてに例外を置いた状態。
- **実テンプレート**(`configure-physics-…_trigDelay1748.xcfg`、ELITPC 実寸 4 AsAd × 4 AGET × 68 ch):
  **3,283 インスタンス**(channel ノブ 3 × 1,088 + AGET ノブ 1 × 16 + CoBo ノブ 3)を全数照合、一致。
- 除去規則(§3-1)の負性テストもオラクルで実施: 明示 id ノードを仕込んだテンプレートでは
  `999 / 6 / 0 / 2 / 1pC` が勝つ(= 罠が本物)→ 生成物では `1748 / 1 / 15 / 15 / 120fC` に戻る。
- 往復(§5-1)は**バイト一致**まで強めた。合成フィクスチャと実テンプレート(831 行)の両方で
  `render(knobs = テンプレート現行値, exceptions = [])` == テンプレート。

### 承認が必要な逸脱

1. **粗い例外の xcfg 上の形(最重要)**: 078 §2 の「`{"asad":1}` → `<AsAd id="1"><Aget id="*"><channel id="*">`」
   は**実機の解決器では効かない**(ccfg_resolve で実測、158 中 22 が一様値に負けた)。一次資料
   `CompoundConfig/src/CCfg/View/Search.hpp` の `if(iInstance >= iPath and index != "*")` が理由。
   レンダラは**テンプレートに実在する id の直積へ展開**して明示ノードで書くことにした
   (JSON の意味は不変、生成 xcfg の形だけが変わる)。実寸コスト: 4 例外(うち 1 つが AsAd 全体の
   gain = 272 ノード)で 831 行 → 1,668 行。SPEC §3 に写す前に裁定が要る(PARAMS_KNOBS_ja.md §6 に
   選択肢 a/b/c)。
2. **生成物名の stem**: §3-3 の「`<template stem>__<hash8>`」を字義どおりにすると
   `configure-configure-physics-…` になるため、テンプレート名から相のプレフィクス
   (`configure-`/`prepare-`)を落とした部分を stem とした(ECC は `<相>-<configId>.xcfg` で読む)。
3. **`at` は (asad, aget, ch) の接頭辞のみ**(`{"ch":3}` 単独や空の `at` はスキーマ違反)。
   §2 の「粗さ」を末尾の切り落としと解釈した。
4. **全ノブ必須**(`knobs` に 7 個すべて無いと 400)。「省略 = テンプレート由来」は §2 に無いので採らない。
5. **sha256 の対象**は受け取った JSON テキストのバイト列(正規化しない)。
6. **ccfg_resolve は C++17 ではなく C++11**: CompoundConfig のヘッダが `std::auto_ptr`(C++17 で削除)を
   使うため。加えて Makefile で `-DWORDS_LITTLE_ENDIAN`(autotools の config.h 代替)と
   **`-DTIXML_USE_STL`**(これが無いと TinyXml のレイアウトが食い違い `XmlToConfigVisitor::isObject` で
   SEGV する — 実測)を与えている。
7. 例外の宛先 id がテンプレートに無ければ**エラー**(黙って無視しない)。許される id はテンプレートから
   集める(ch 数をコードに焼き込まない — CLAUDE.md)。

### 未実施

- §5-3(実 ECC で describe→prepare→configure)、§5-4(REST/UI)、§5-5(実データへの反映)は
  レーン B/C/実機作業。レーン A の範囲外。

### 追記(同日レビュー裁定の反映 — 未実装 4 ノブの実装、2026-09-22)

レビューで逸脱 1 は選択肢 **a**(粗い例外はテンプレート実在 id の直積へ展開)で確定、
逸脱 2–7 は承認、未実装 4 ノブは位置を裁定のうえ実装 —— として追加実装した。

- **テスト数: 645 → 655 passed / 0 failed / 1 ignored**(レーン A 全体では 614 → 655、+41)。
  `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green。
- **ノブ: 7 → 11 実装(JSON の葉は 13)**。`docs/PARAMS_KNOBS_ja.md` §4 に裁定後の位置、
  §7 に一覧表(JSON キー → xcfg)を追加。
- **オラクル照合(新ノブ込み)**: 合成フィクスチャ **238 インスタンス**(158 → 238)、
  **実テンプレート 4,423 インスタンス**(3,283 → 4,423 = CoBo 7 + AGET 4×16 + channel 4×1,088)、
  いずれも全数一致。除去規則の負性テストも罠を 5 → **10 パス**に増やし、
  テンプレートでは `999/6/0/2/1pC/70/false/11/true/true` が勝つ → 生成物では
  `1748/1/15/15/120fC/232/true/600/false/false` に戻ることをオラクルで確認。
  実テンプレートの往復は 11 ノブでも**バイト一致**のまま。

#### 各ノブの実装位置(裁定どおり)

| JSON | スコープ | xcfg | 備考 |
|---|---|---|---|
| `peakingTime_ns` | AGET | `Global.Reg1.peackingTime` | 綴りは GET のまま。値域は定数 `PEAKING_TIME_NS`(AGET 仕様 16 値)。実テンプレートの出現値は 232(physics/pedestals)と 1014(pulser) |
| `readout` | AGET | `Global.Reg1.isAllChannelRead` + `isFPNRead` | full=true,true / partial=false,false |
| `zeroSuppression.enabled` | CoBo | `Module.enableZeroSuppression` | 例外は付けられない(スコープが CoBo) |
| `zeroSuppression.threshold` | channel | `channel.zeroSuppressionThreshold` | 例外はここにだけ |
| `pulser.enabled` / `.voltage_mV` / `.period_ms` | CoBo | `Generator.periodicModeEnabled` / `amplitudeStart` / `period_ms` | `TestModeSelection` と `isSelectedforTestMode` は触らない |

#### 追加で承認が必要な逸脱(2 点)

8. **`readout` は 4 要素ではなく 2 要素だけ管理**。裁定の方法(「実テンプレート 2 本の差分から取る」)で
   実測したところ、差分に現れたのは `isAllChannelRead`(true→false)と `isFPNRead`(true→false)の
   **AGET 2 要素だけ**で、`Module.enableWriteHittedregister`(physics も pulser も true)と
   `channel.Reading`(どちらも always)は差分に現れない。さらに
   **`enableWriteHittedregister` は configure-pedestals 2 本では `false`** なので、「full なら true」と
   一様に書くと pedestals テンプレートの挙動を勝手に変え、往復のバイト一致も壊れる。
   よってこの 2 要素は非管理(テンプレート由来)のままにした。表は PARAMS_KNOBS_ja.md §4-2。
9. **`pulser.voltage_mV` は換算なしで `amplitudeStart` と同値**と確定(名前は `voltage_mV` のまま、
   §2 の JSON 例から落とす必要はない)。根拠: `CoBoConfigParser::asadPulserVoltage_mV`
   (GetBench/src/get/rc/CoBoConfigParser.cpp:1074)が `amplitudeStart` をそのまま
   「出力電圧と基準電圧(Vref=1102 mV)の差 [mV]」として返す(int16 に切り捨て)+ xcfg の
   `unit="mV"` + テンプレート値 -1000.0 が §2 の例 `-1000` と一致。
   なお `peakingTime_ns` の値域は「16 値のうち実テンプレートに現れる値だけ」と読むと
   実質 {232}(+pulser の 1014)になり実験切替級ノブとして使えないため、**AGET 仕様の 16 値**
   (= xcfg の `range` 属性そのもの)を定数表とし、テンプレートの `range` と一致することを
   テストで検算する形にした。

### Fable レビュー(レーン A、2026-09-22、マージ `worktree-agent-a92b0c123f7438f2d` → main)
- 自分でも再実行: 実テンプレートのオラクル照合 **4,423 インスタンス一致**(`params_ccfg_oracle` 4 本 green)、ブランチ `cargo test` 655 passed、fmt / clippy clean。
- 逸脱 1 = **選択肢 a 確定**(粗い例外は実在 id の直積へ展開)。逸脱 2–9 **承認**(stem の相プレフィクス除去 / `at` 接頭辞 / 全ノブ必須 / 生バイト sha256 /
  ccfg_resolve C++11 + `-DTIXML_USE_STL` / 宛先 id 不在はエラー / readout は差分に現れた AGET 2 要素のみ管理(`enableWriteHittedregister` は pedestals で false = モード非依存) /
  `voltage_mV` = `amplitudeStart`(CoBoConfigParser.cpp:1074)/ `peakingTime_ns` は xcfg `range` の 16 値をテストで検算)。
- **SPEC v1.32 §3.3 に契約を固定**(ノブ 11、スコープ、展開規則、除去規則、受け入れ)。§2 の JSON 例の `peakingTime_ns` は AGET スコープ、`pulser` は CoBo スコープと読み替える(表は PARAMS_KNOBS_ja.md §7 が正)。
- 次 = レーン B(controller REST + run/start 結線 + 監査ログ、Sonnet)→ レーン C(UI、Sonnet)。§5-3(実 ECC)は新 DAQ 機。

## 10. レーン B 発注(controller REST + run/start 結線 + ログブック — Sonnet、2026-09-23)

前提: レーン A 済(`src/params/{schema,render,paths}.rs`、`render(&Params, template_xml) -> Rendered`、`Params` は serde、`ParamsError` は文言付き)。
契約 = SPEC v1.32 §3.3。着手前に worktree で `git merge --ff-only main`(ff できなければ固有コミットが無いことを確認して `git reset --hard main`)。
発注書は本体の絶対パス `/Users/aogaki/Workspace/tpcdaq-rs/TODO/078_param_ui_json_xcfg.md`(§2–§4、§9 結果節、本節)で読んでよい。UI(レーン C)は含めない。

### 作るもの
1. `src/config.rs` `[controller]` に **`ecc_config_dir: Option<PathBuf>`**(ECC の config repo = 生成 xcfg の出力先。テンプレート `template` もこの下から読む)と
   **`params_dir: Option<PathBuf>`**(JSON の保存先。省略 = `<output_root>/params`)。`ecc_config_dir` 省略時は params 系 REST が **503 `{"error":"ecc_config_dir is not configured"}`**
   を返す(黙って無効にしない)。`deny_unknown_fields` 維持、既存 TOML は無変更で読める。
2. `src/controller.rs` REST(§8.1 に追加、token は run/start と同じ操作権 token):
   - `GET /api/params` → 現在の JSON があれば `{ "id": "<stem>__<sha8>", "sha256": "...", "exceptions": N, "params": {...}, "files": {"configure": path, "prepare": path} }`、無ければ 404。
   - `PUT /api/params` body `{ "token": "...", "params": {...} }` → `Params` として検証(失敗 = **400** に `ParamsError` の文言)→ テンプレートを
     `<ecc_config_dir>/<template>` から読む(無ければ 400)→ `render` → `<ecc_config_dir>/configure-<id>.xcfg` と `prepare-<id>.xcfg` を書く
     (**既存なら上書きしない** = 同じ JSON は同じファイル)→ JSON を `<params_dir>/current.json` と `<params_dir>/<id>.json` に保存 → GET と同じ形を返す。
     監査 `audit` レコード(`action: "params/put"`、`params: {id, exceptions}`)を書く。
   - `GET /api/params/render?template=<name>` + body 無し(dry run): current.json を指定テンプレートで render し **書かずに** `{ "id", "exceptions", "configure_xcfg": "<text>" }` を返す
     (UI の diff 表示用。テンプレート省略時は current.json の template)。
   - Run 中(phase ≠ Idle)の PUT は **409**(次 run から効くという半端を作らない)。
3. **run/start 結線**: current.json があれば **prepare / configure の config_id = `<id>`**(describe は TOML のまま)。無ければ従来どおり TOML の config_id。
   logbook `run_start` に **`params: {id, sha256, exceptions}`(nullable、無ければ省略)**、`audit run/start` の params に `params_id`。
   `run_start_config_fields` の `config_ids` 分岐(§9.2 v1.13: 非同値時のみ出す)は params 由来でも同じ規則で出す。
4. `GET /api/status` に `params: {id, exceptions} | null` を足す(status バーの `params <id> · exceptions N` の元。UI はレーン C)。

### テスト(`tests/controller_integration.rs` の Harness / MockTransport の流儀、TDD)
- テンプレート = レーン A の合成フィクスチャ `tests/fixtures/params/template_mini.xcfg` を一時ディレクトリ(= `ecc_config_dir`)にコピーして使う。
- ① GET 404 → PUT(有効 JSON = `tests/fixtures/params/params_roundtrip.json`)→ 2 ファイル生成・GET 200・`exceptions` 件数 ② PUT(範囲外 / スコープより細かい例外 / 未知ノブ)→ 400 と文言
  ③ 同じ JSON を再 PUT → ファイルのタイムスタンプ不変 ④ `ecc_config_dir` 無し TOML → 503 ⑤ run/start → MockTransport が受けた prepare / configure の `config_id` が `<id>`、
  describe は TOML の値、logbook `run_start.params` と `audit.params_id` ⑥ Running 中の PUT → 409 ⑦ dry-run render が書かないこと(ディレクトリの一覧不変)。
- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(現 784)。production `.unwrap()` 無し。

### 受け入れ・逸脱
結果節に: エンドポイントの最終形(JSON 例)、テスト数 before → after、承認が要る逸脱。SPEC §8.1 / §9.2 / §3.1 への転記は Fable(触らない)。
設計判断が残る分岐(例: run 中 PUT の扱いを 409 以外にしたい、`<id>` 衝突時の扱い)は実装せず報告。

## 結果(レーン B — REST + run/start 結線、2026-09-23、Sonnet レーン)

実行環境: macOS 26.6.2 (arm64) / rustc 1.97.1。着手前に worktree で `git merge --ff-only main`
→ `71a857c`(a11013a 以降)に到達、`cargo test` 784 passed を確認してから着手。

### 実行コマンド

```
cargo fmt && cargo clippy --tests -- -D warnings && cargo test
```

### テスト数

**784 passed → 793 passed / 0 failed / 1 ignored**(+9、`ignored` はレーン A から既存で未変更)。
内訳: `tests/controller_integration.rs` に発注 §10 の ①〜⑦ + 「作るもの」④(status)の **8 本**
(`params_get_404_then_put_creates_the_xcfg_pair_and_get_returns_the_same_shape` / `params_put_rejects_schema_violations_with_400_and_the_params_error_text` /
`params_put_is_idempotent_and_leaves_existing_file_timestamps_alone` / `params_rest_returns_503_when_ecc_config_dir_is_not_configured` /
`params_id_drives_prepare_and_configure_config_id_at_run_start` / `params_put_is_refused_with_409_while_a_run_is_active` /
`params_render_dry_run_does_not_write_any_file` / `status_reports_the_current_params_id_and_exceptions_or_null`)、
`src/logbook.rs` に `run_start.params` の位置・省略を固定する単体 **1 本**
(`run_start_carries_the_params_id_after_slow_and_omits_it_when_there_is_no_current_json`)。
production コードに `.unwrap()` 無し(新規追加分は grep で確認済み — `.unwrap()` が付いたのはテスト内の 2 箇所のみ)。
`config.rs` の既存 2 テスト(`controller_016_keys_default_to_the_spec_port_table` /
`controller_016_keys_are_overridable`)を拡張して `ecc_config_dir` / `params_dir` の既定(`None`)と
上書きを追加検証(新規テスト関数は増やさず、既存の「016 追記キー」の流儀に合わせた)。

TDD の逸脱: `src/params/` 側(レーン A、変更禁止)の契約が固まっていたため、REST 層のテスト(①〜⑦)は
ハーネス配線(`ControllerParams` へのフィールド追加、`harness_full` への `ecc_config_dir` 引数追加)と
ハンドラ実装を同じセッションでまとめて書き、`cargo test` で最初に確認したのは green(実装込み)だった
—— 個々のテストを実装前に赤で確認する古典的な手順は踏めていない。`src/logbook.rs` の `params` フィールド
(schema 定数 `RUN_START_WITH_PARAMS` を含む新設テスト)は先にテストを書いてから struct に手を入れており、
そちらは赤→緑を実測している。

### エンドポイント最終形(実行例、`tests/fixtures/params/params_roundtrip.json` を PUT した場合)

`PUT /api/params` req: `{"token": "<token>", "params": <params_roundtrip.json の中身>}`

`PUT` / `GET /api/params` 応答(同じ形。パスは実行毎の一時ディレクトリなので値は例):

```json
{
  "id": "mini-fixture__797d5dc2",
  "sha256": "797d5dc245553d3aaae0c7172993e6a311de94e7d48f211855ecc6c762112ac5",
  "exceptions": 0,
  "params": {
    "schema": "tpcdaq-params/1",
    "template": "configure-mini-fixture.xcfg",
    "knobs": { "triggerDelay": 1748, "writingClockFrequency_MHz": 25.0, "triggerMode": "external",
      "globalThreshold": 1, "lsbThreshold": 15, "gain": "120fC", "triggerInhibition": "none",
      "peakingTime_ns": 232, "readout": "full",
      "zeroSuppression": { "enabled": false, "threshold": 600 },
      "pulser": { "enabled": false, "voltage_mV": -1000.0, "period_ms": 1000 } },
    "exceptions": [],
    "note": "template_mini.xcfg の現行値そのまま(078 §5-1 の往復テスト)"
  },
  "files": {
    "configure": "<ecc_config_dir>/configure-mini-fixture__797d5dc2.xcfg",
    "prepare": "<ecc_config_dir>/prepare-mini-fixture__797d5dc2.xcfg"
  }
}
```

`GET /api/params/render`(省略時 = current.json の template)応答: `{"id", "exceptions", "configure_xcfg": "<xcfg テキスト>"}`(書かない)。

`GET /api/status` の追加分: `"params": {"id": "mini-fixture__797d5dc2", "exceptions": 0}`(current.json 無し / ecc_config_dir 未設定 / render 失敗はどれも `null`)。

`run/start` 成功時の `logbook.run_start.params`: `{"id": "...", "sha256": "...", "exceptions": 0}`(current.json が無ければキーごと省略)。
同じ run の `audit run/start` の `params` に `"params_id": "..."`(current.json 無しは `null`)を追加。

### 承認が必要な逸脱

1. **`ControllerConfig.ecc_config_dir` / `params_dir` は `Option` のまま持ち回り**、`config.rs::resolve()` では
   既定を解決しない(§10 発注の型注記どおり)。`params_dir` の既定 `<output_root>/params` は
   `Controller::params_dir()`(呼び出し側)で解決する ——既存の `router_ip: Option<String>` と同じ
   「Option を持ち回り、既定は使う側で足す」流儀に揃えた(`output_root` が `[system]` セクションの値で
   `[controller]` 単独では解決できないのも `router_ip` と同型の事情)。
2. **PUT の検証順序**: authorize(401)→ `ecc_config_dir` の有無(503)→ JSON スキーマ検証(400)→
   相が Idle か(409)→ テンプレート読込 + render(400)→ 書き込み。発注書はこの順序を明記していない
   (既存の `post_run_next` の「authorize → 入力の形 → 相」の順に、503 の可用性チェックを
   authorize の直後に足した形)。
3. **GET と PUT/render での render 失敗の HTTP コードを分けた**: PUT・`GET /api/params/render` は
   リクエストの入力(JSON 本体やクエリの `template`)が原因なので 400、`GET /api/params` は
   「保存済み current.json をそのまま映すだけ」の閲覧系なので同じ失敗を 500(サーバ側の不整合、
   例えば PUT 後にテンプレートファイルが消された)として返す。発注書に明記が無いための解釈。
4. **PUT の本文の正本化**: axum の `Json` extractor は受信バイト列を `serde_json::Value` へ parse
   済みで、生バイト列そのものは残らない。`request.params.to_string()`(`serde_json::Map` は
   `preserve_order` 機能を有効にしていないので `BTreeMap` 裏付け = キー常に辞書順)を「正本の
   JSON テキスト」とし、`Params::from_json_str` の検証・sha256・`current.json`/`<id>.json` の
   保存すべてこの文字列を使う。空白の揺れは吸収されるが、これは §3-3「同じ JSON → 同じファイル」を
   REST 越しでも保つのに必要な性質だと判断した。
5. **`current.json` は content-addressed 扱いにしない**(常に上書き)。`<id>.json` /
   configure-`<id>`.xcfg / prepare-`<id>`.xcfg は既存なら書かない(§3-3 の規則そのまま)。
6. **run/start で current.json はあるが render できない場合は run を始めず `Err`**
   (黙って TOML の config_id へ後退しない — CLAUDE.md の silent failure 禁止を優先)。
   `ecc_config_dir` が(current.json 保存後に)外された場合も同様に run を拒否する。
7. **`GET /api/status` の `params` は失敗しても `null` + `notes` に一言、status 応答自体は
   200 のまま**(他コンポーネントの不達と同じ「集約 view は落とさない」既存の設計に合わせた)。
8. **`audit params/put` の `params` は常に `{id, exceptions}` の 2 キー**(sha256 は載せない —
   発注書の例に忠実。`GET`/`PUT` 応答には sha256 を含める)。

### 未実施 / 範囲外

- UI(レーン C)、SPEC §3.3/§8.1/§9.2 への転記(Fable)は本ユニットの範囲外(発注書どおり)。
- `src/params/{schema,render,paths}.rs` は無変更(発注どおり「既存の src/params/ を使い、変えない」)。
- 実 ECC(vcobo)での describe→prepare→configure 通し確認(078 §5-3)は新 DAQ 機作業として未実施(レーン A から持ち越し、レーン B の範囲外)。

### Fable レビュー(レーン B、2026-09-23、マージ `worktree-agent-af680f503dd115338` → main `a69ad50`)
- 逸脱 1–8 **承認**(Option のまま持ち回り / 検証順序 401→503→400→409→400 / GET の render 失敗は 500・PUT は 400 / 正本は serde_json 再直列化(辞書順)の
  テキスト = **id の正規化規則として SPEC §3.3 に固定** / current.json は上書き・`<id>` 系は書かない / render 不能なら run を始めない / status は null + notes / audit は `{id, exceptions}`)。
- TDD の逸脱(REST 層は実装と同時に緑を見た)は記録のみ。logbook 側は赤→緑を実測済み。
- **SPEC v1.33** に転記(§3.1 `ecc_config_dir` / `params_dir`、§3.3 id と REST の状態規則、§8.1 エンドポイント、§9.2 `run_start.params` / `audit.params_id`)。
- 次 = レーン C(UI: Parameters ビュー、Exceptions (N) テーブル、diff 表示、status バー、Sonnet)。§5-3(実 ECC で describe→prepare→configure)は新 DAQ 機。

## 11. レーン C 発注(UI: Parameters ビュー — Sonnet、2026-09-23)

前提: レーン A/B 済(REST は SPEC v1.33 §8.1、契約は §3.3。`GET /api/params` 404 = 未設定、`PUT /api/params {token, params}`、
`GET /api/params/render?template=`、`/api/status.params`)。着手前に worktree で `git merge --ff-only main`(`79df4b2` 以降)。ui/node_modules は
`ln -s /Users/aogaki/Workspace/tpcdaq-rs/ui/node_modules ui/node_modules`。発注書は本体の絶対パスで読んでよい。
**同時に別レーン(079 Phase 4)が `app.routes.ts`(NAV_ITEMS の `power` 行を `slow` に変える)と `status-bar.{ts,html}`(`slow` バッジを足す)を触る。**
あなたは `app.routes.ts` に `params` の 1 行(NAV は `run` の直後、route は `run` の直後)、`status-bar.html` に `params` の `<span class="item">` 1 個を
**既存行の末尾に追加するだけ**にする(既存行の並び替え・整形をしない = 衝突を小さく)。

### 作るもの
1. `ui/src/app/api/params-api.ts` + spec: `fetch` 直(controller-api.ts と同じ流儀、`FetchLike` 注入でテスト)。
   `getParams(): Promise<ParamsInfo | null>`(404 → null、503 → `{error}` を throw)/ `putParams(token, params): Promise<ParamsInfo>`
   (400 / 401 / 409 / 503 はサーバの `error` 文言を `ParamsApiError { status, message }` で throw)/ `renderParams(template?): Promise<{id, exceptions, configure_xcfg}>`。
   型: `ParamsJson { schema, template, knobs, exceptions: ExceptionRow[], note? }`、`ParamsInfo { id, sha256, exceptions, params, files }`(§10 結果節の JSON 例が正)。
2. `ui/src/app/params/knob-spec.ts` + spec: **11 ノブ(葉 13)の表**(`docs/PARAMS_KNOBS_ja.md` §7 を写す: キー / スコープ深さ / 型 / 範囲・選択肢 / 単位)。
   焼き込みではなく「サーバ `src/params/schema.rs` の写し」であることをコメントし、spec で `tests/fixtures/params/params_roundtrip.json` の全キーが表に
   在ること(過不足なし)を検査する(サーバ側の表が変わったら赤になる = 二重管理の検知)。最終検証はサーバ(400 の文言をそのまま見せる)。
3. `ui/src/app/params/params-view.{ts,html,scss}` + spec(遅延ルート `params`、NAV `{ path: 'params', label: 'Parameters', icon: 'tune' }`):
   - 読み込み時に `getParams()`。無ければ「No parameters yet」+ テンプレート名入力 + 「Load defaults from template」(= `renderParams(template)` は使えないので、
     **knobs は表の既定値(configure-physics 25 MHz の現行値 = params_roundtrip.json の値)で空フォームを作る**。それを保存すると生成される)。
   - フォーム: ノブ 11 個(select / number / checkbox、範囲は表由来)。**スコープは見せない**(§0)。
   - 折りたたみ「Exceptions (N)」: 行 = `at`(asad / aget / ch、接頭辞。空欄 = 粗い指定)、knob(select)、value、note、削除。追加ボタン、「Reset to uniform」(全削除)。
     件数バッジは常に見える(隠れているが黙らない)。
   - 「Preview xcfg」= `renderParams()` の `configure_xcfg` を `<pre>` に表示(**テンプレートとの diff は出さない** — テンプレート本文が REST に無い。
     代わりに管理ノブの要素名を含む行をハイライト)。「Save」= `putParams(token, …)`(token は `RunControlApiService` の sessionStorage、無ければ
     「Acquire control in Run control first」と表示して無効化)。成功で `id` / `exceptions` / `files` を表示、失敗はサーバ文言をそのまま。
   - Running 中(WS status の `state === 'running'`)は Save を無効化し理由を表示(サーバも 409 を返す)。
4. `status-bar`: `params <id> · exceptions N`(`getParams()` を 15 s 毎に poll、404/503/失敗は `—`)。既存の項目の後ろに 1 個追加。
5. 英語 UI(063 の方針)。モックの値を出さない。

### テスト(vitest、`npm test -- --watch=false`)
params-api(200 / 404 / 400 文言 / 409 / 503 / 401)、knob-spec(fixture との過不足)、params-view(読み込み表示、例外の追加・削除・件数、
Save 無効化条件、Preview のハイライト)。`npm run build` green、prettier。現 204 passed から増分を結果節に。

### 受け入れ・逸脱
結果節に: 画面構成の要旨(スクリーンショットは不要、テストで縛る)、テスト数 before → after、承認が要る逸脱(特に diff 表示を出さない件の代替)。
SPEC §11 への転記は Fable。設計判断が残る分岐は実装せず報告。

## 結果(レーン C — Parameters UI、2026-09-23、Sonnet レーン)

実行環境: macOS 26.6.2 (arm64) / Angular CLI 22.1.4 / vitest 4.0.8。着手前に worktree で
`git merge --ff-only main` → `1cead35` に到達、`ln -s .../ui/node_modules ui/node_modules`、
`cd ui && npm test -- --watch=false` で 204 passed を確認してから着手。

### 実行コマンド

```
cd ui
npm test -- --watch=false
npm run build
npx prettier --check "src/app/**/*.{ts,html,scss}"
```

### 画面構成の要旨

- `ui/src/app/api/params-api.ts`(+ spec): `controller-api.ts`/`run-control-api.ts` と同じ
  `FetchLike` 注入の薄いクライアント。`getParams()`(404→`null`、それ以外の失敗は
  `ParamsApiError{status,message}` をサーバの `error` 文言のまま投げる)/ `putParams(token,
  params)` / `renderParams(template?)`。
- `ui/src/app/params/knob-spec.ts`(+ spec): `docs/PARAMS_KNOBS_ja.md` §7 の写し。11 グループ・
  葉 14(§7 の表をそのまま数えると 14 —— 発注書本文の「葉は 13」という記述とは 1 個ずれている。
  実装は表・フィクスチャ・サーバのどれとも矛盾なく一致させたので、本文側の数え間違いと判断し
  そのまま進めた。設計判断ではないため報告のみ)。`flattenKnobs`/`nestKnobs` で JSON の入れ子
  (`zeroSuppression`/`pulser`)とフラットな葉のキー(`.` 区切り、`exceptions[].knob` と同じ表記)
  を往復させる。`knob-spec.spec.ts` が `tests/fixtures/params/params_roundtrip.json` の
  `knobs` と過不足なく一致することを検査(サーバ側スキーマが変わったら赤になる = 二重管理の検知)。
- `ui/src/app/params/params-view.{ts,html,scss}`(+ spec、遅延ルート `params`、NAV
  `run` の直後): 読み込み(`GET /api/params`)→ 404 なら「No parameters yet」+ テンプレート名
  入力 + 「Load defaults from template」(knobs は `DEFAULT_KNOBS` = `params_roundtrip.json` の
  値そのもの、`params-view.spec.ts` の別 `describe` でフィクスチャと一致することを検査)。
  ノブ 11 個のフォーム(select/number/checkbox、スコープは非表示)、折りたたみ
  「Exceptions (N)」テーブル(at 接頭辞・knob・value・note、追加/削除/Reset to uniform、
  件数バッジは常時表示)、Save(token 必須・Running 中は無効・失敗はサーバ文言そのまま)、
  Preview xcfg(`renderParams()` の `configure_xcfg` を表示。テンプレート本文が REST に無い
  ため diff は出さず、管理ノブの xcfg 要素名を含む行をハイライト)。
- `status-bar`: 末尾に `params <id> · exceptions N` を追加(`ParamsApiService.getParams()` を
  15 s 毎に poll、404/503/失敗はどれも `—`)。

### テスト数

**204 passed → 246 passed / 0 failed / 5 skipped**(既存の `describe.skipIf` 4 本 + 元からの
1 skip、変更なし)。内訳(新規 42):

- `params-api.spec.ts` 15(200/404/503/500/ネットワーク断 × GET、body 形/400/401/409/503/
  インスタンス確認 × PUT、template 有無/URL エンコード/400/本文非 JSON × render)。
- `knob-spec.spec.ts` 12(グループ数・葉数、フィクスチャとの過不足、往復、個別ノブの型・範囲、
  未知キー、スコープ深さの集合)。
- `params-view.spec.ts` 11(404→既定値フォーム、200→表示、503→ロードエラー(No parameters yet
  にしない)、例外の追加・削除・Reset、token 無し→Save 無効、Running 中→Save 無効、Save 成功、
  Save 失敗(400 文言そのまま)、Preview 成功+ハイライト、Preview 無効(未保存)、
  DEFAULT_KNOBS↔フィクスチャ一致)。
- `status-bar.spec.ts` 4(取得成功、404→`—`、503/失敗→`—`、15 s ごとの再取得)。

`npm run build` green(`params-view` は独立の遅延チャンク 23.09 kB raw / 5.99 kB transfer)。
`npx prettier --check` green。

### 承認が要る逸脱

1. **Preview xcfg は「テンプレートとの diff」ではなく「管理ノブの行のハイライト」**
   (発注書 §11-3 が明記した代替そのもの)。ハイライト対象の xcfg 要素名(`KnobField.xcfgElements`)
   は `docs/PARAMS_KNOBS_ja.md` §7 の「xcfg」列から手で写した表示専用のヒントで、
   型・範囲の表(`KNOB_FIELDS` 本体)と違って**フィクスチャとの自動突き合わせテストが無い**
   (画面のハイライトが 1 行増減する程度の実害しかないため、往復・過不足検査ほど厳格にしなかった)。
   サーバのノブ表が変わった場合、ここだけ手で追従する必要がある——承認 or 追加テストの要否を確認したい。
2. **`status-bar.ts` に polling ロジックを直接足した**(status-bar.html は末尾に 1 行足すだけ、
   という発注書の指示は `.html` について明記していたが `.ts` には触れていなかった)。
   `params <id> · exceptions N` は 15 s poll という状態を持つ機能なので、既存の
   `state`/`run`/`eventsBuilt` 等と同じ置き場(`StatusBar` クラス)に signals・
   `OnDestroy`・`constructor` を追加した。同時進行の 079 レーン(slow バッジ)も
   `status-bar.ts` を触る前提なので、マージ時に競合が起きる可能性がある(内容は独立した
   追記のみなので解消は容易と見立てている)。
3. **`<select>` の選択状態は `[value]`(`<select>`側)ではなく `[selected]`(各 `<option>` 側)で
   持たせた**。`@for` で生成した `<option>` は親 `<select>` の `[value]` 束縛より後に DOM へ
   挿入されるため、素直に `[value]` を付けると選択が反映されない(実測— 最初のテスト実装で
   Gain が常に先頭の選択肢に固定される不具合として顕在化、`isEnumSelected`/
   `isExceptionKnobSelected`/`isExceptionValueSelected` で修正)。発注書に明記の無い実装細部
   だが、念のため記載する。
4. **`GET /api/params` が 503/500 等(404 以外)で失敗した場合は「No parameters yet」の
   新規作成導線を出さない**(ロードエラーのバナーだけを出す)。サーバ側の不整合がある状態で
   ユーザーに「新規作成」を促すのは危険と判断した(発注書に明記なし)。
5. **例外テーブルの `value` 入力は選択した `knob` の型(bool は true/false の select、
   enum は選択肢の select、int/real は number input)に追従させた**。発注書は
   「value(select)」とだけ書いており型の出し分けまでは指定していないが、078 §2 の
   スキーマ検証(型違いは 400)と整合させるため、フォーム側でも同じ型のコントロールにした。
6. **テストの `whenStable()` だけでは fetch ベースの非同期チェーン(2 段以上 await)を
   待ちきれなかった**ため、`params-view.spec.ts`/`status-bar.spec.ts` の `settle()`/`flush()`
   は `whenStable()` の後にマイクロタスクを 20 回 `await` している(zoneless 環境の既知の
   制約。`run-view.spec.ts` は 1 段 await の fake しか使っていなかったため気付かれていなかった
   可能性がある — 他レーンの async テストで同種のフレークが出た場合はこの手当てを検討されたい)。

### 未実施 / 範囲外

- SPEC §11 への転記(Fable)。
- 実 ECC / 実機での確認(078 §5-3/§5-5)は本レーンの範囲外。
- demo スタック起動は行っていない(指示どおり)。

### Fable レビュー(レーン C、2026-09-23、マージ `worktree-agent-add54223d5e1c8051` → main)
- 079 Phase 4 と `app.routes.ts` / `status-bar.{ts,html,spec}` で衝突 → 両方残して解決(NAV は `params` → `slow` の順、spec は `status-bar.spec.ts`(params)と
  `status-bar-slow.spec.ts`(slow)に分割、template の閉じタグ抜けを修正)。マージ後 `npm test` **268 passed / 5 skipped**、`npm run build` green(Initial 512 kB)、prettier clean。
- 逸脱 1–6 **承認**(xcfgElements の手動転記は許容 — 変更頻度が低く、Preview のハイライトだけの用途 / status-bar.ts の poll / `[selected]` / 503 では新規導線を出さない /
  例外 value の型別コントロール / テストの flush)。
- **SPEC v1.34 §11** に Parameters ビューを固定。残 = §5-3(実 ECC、新 DAQ 機)と §5-5(実データへの反映)。**078 のソフト側は完了**(A/B/C)。

## 結果(§5-3 / §5-5 — 実 ECC + 実 zCoBo、2026-09-23 12:40–12:44、GDED 遠隔、Fable)
- 構成: 076 Phase 3 前提と同じ(実 ECC ネイティブ + Rust ecc_bridge + bench スタック)。`bench2.conf` に `ecc_config_dir = /home/daq/ELITPC/GetSoftware_config`、
  `params_dir = out/params`。全 bin を再ビルド(古い graw_writer 等が `deny_unknown_fields` で新キーを拒否した → 「bin は全部同時に更新する」)。
- **P1 = テンプレート(`configure-pulser.xcfg`、GDED 実物 md5 e10434965…)の現行値をそのまま JSON にして PUT** → `id = pulser__75bb9a9e`、
  `configure-pulser__75bb9a9e.xcfg` / `prepare-…` が config dir に生成(44,212 B)。run 12 / 13: `run_start.config_ids = {describe: zCobo, prepare: pulser__75bb9a9e,
  configure: pulser__75bb9a9e}`、`run_start.params = {id, sha256, exceptions: 0}`、**ECC ログ `Reading configuration file '…/configure-pulser__75bb9a9e.xcfg'`**、
  prepare / configure / start とも `NO_ERR`、30 frames、malformed 0。→ **§5-3 合格**(生成 xcfg を実 ECC が受理)。
- **P2 = gain 120fC → 240fC だけ変えて PUT** → `id = pulser__aecb7641`、run 14(30 frames)。`calib pulser`(−300 mV):
  | run | 設定 | signal ch 振幅中央値 [ADC] |
  |---|---|---|
  | 8 | 純正 `pulser` config(120fC) | 1074.0 |
  | 12 | params P1(120fC、テンプレート値) | 1079.7 |
  | 14 | params P2(240fC) | 598.2 |
  P1 / 純正 = **1.005**(生成 xcfg は純正と同じ挙動)、P2 / P1 = **0.554**(gain 段が実データに現れる)。→ **§5-5 合格**。
- 注意: `run_bench.sh` が操作権を横取りするため、スクリプト内で先に取った token が PUT で `stale or unknown token` になった(P2 の 1 回目 = run 13 は P1 のまま)。
  token を PUT 直前に取り直して解決。生成 xcfg 2 組は GDED の config dir に残置(純正ファイルは無改変)。
- 078 は §5-4(REST/UI テスト = レーン B/C 済)を含め**全受け入れ完了**。UI からの操作は現地で。

