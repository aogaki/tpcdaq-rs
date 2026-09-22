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

