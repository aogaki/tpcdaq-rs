# PARAMS_KNOBS_ja — 078 §2 ノブ定義表のパス照合結果

**照合日**: 2026-09-22(078 レーン A、Opus)
**照合対象**(`reference/config/`、リポには入れない実ファイル):

| ファイル | 役割 | 行数 |
|---|---|---|
| `configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg` | HIGS2026 現用 physics(**既定テンプレート**) | 831 |
| `configure-physics-zCobo1k-extTrig_120fC_12.5MHz_232ns_trigDelay3576.xcfg` | 同 12.5 MHz 版 | 831 |
| `configure-pedestals-zCobo1k_120fC_25MHz_232ns.xcfg` | pedestal(periodically トリガ) | 831 |
| `configure-pedestals-zCobo1k_120fC_12.5MHz_232ns.xcfg` | 同 12.5 MHz 版 | 831 |
| `configure-pulser.xcfg` | AsAd 内部パルサ較正 | 833 |
| `describe-elitpc.xcfg` | describe 相(FW とペア、**管理対象外**) | 23 |

5 本の configure-*.xcfg は**構造が同一**(同じ要素が同じ入れ子・同じ行位置に並ぶ)。値だけが違う。
以下の「テンプレート実パス」は 5 本すべてで同一であることを確認済み。

## 0. 内部パス表記

レンダラ(`src/params`)とオラクル(`tools/ccfg_resolve`)は CompoundConfig の `Path` 構文をそのまま
内部表現に使う(078 §3-5「パスは XPath 風の内部表現 1 種類に固定」):

```
Node[CoBo].Instance[0].AsAd[1].Aget[2].channel[5].LSBThresholdValue
```

スコープ → パス接頭辞:

| スコープ | 接頭辞 |
|---|---|
| CoBo | `Node[CoBo].Instance[{i}]` |
| AsAd | `Node[CoBo].Instance[{i}].AsAd[{a}]` |
| AGET | `Node[CoBo].Instance[{i}].AsAd[{a}].Aget[{g}]` |
| channel | `Node[CoBo].Instance[{i}].AsAd[{a}].Aget[{g}].channel[{c}]` |

ルート要素 `Setup[Conditions]` はパスに含めない(CConfig の base attribute)。
`{i}` は CoBo インスタンス。JSON の `exceptions[].at` に cobo 欄は無い(078 §2)ので、
レンダラが書くのは常に `Instance[*]` 配下。

## 1. 照合の判定規則

078 §2 の表の「xcfg 上の位置」列は **「072 §3、要照合」と注記された下書き**なので、
照合で**位置が 1 段ずれていただけ**(スコープ・要素名・型・範囲が一致し、解決先が一意)は
**訂正して実装**、**スコープが違う / 要素名が違う / 1 ノブが複数要素の合成**は
**食い違い = 実装せず報告**(078 §9「食い違うノブ・見つからないノブは実装せず報告」)とした。

## 2. 照合結果(サマリ)

- 照合したノブ: **11**(§2 の表 10 個 + §2.1 裁定 2 の `TriggerInhibition`)
- 初回(2026-09-22 午前)で位置が一致 → そのまま実装: **7**(§3)
- 表と食い違ったので一旦保留 → **同日のレビュー裁定で位置を確定して実装: 4**(§4)
- **2026-09-22 時点で 11 ノブすべて実装済み**(JSON の葉は 13 個 —
  `zeroSuppression` が 2、`pulser` が 3 に分かれるため)

## 3. 初回の照合で位置が一致したノブ(7)

`現行値` は physics 25 MHz テンプレート(既定テンプレート)の値。

| knob | スコープ | テンプレート実パス(接頭辞の後ろ) | xcfg の型・range | 現行値 | 078 §2 表との差 |
|---|---|---|---|---|---|
| `triggerDelay` | CoBo | `Module.triggerDelay` | int、`unit="10ns"`、range 属性なし | 1748 | 位置一致。単位は表の「clock 単位」ではなく **10 ns 固定**(属性 `unit="10ns"`)。**同名要素が AsAd 配下(`AsAd[a].Generator.triggerDelay`、`unit="WCK periods"`、range [0,31])にもある** — 除去規則を要素名でやると壊れる |
| `writingClockFrequency_MHz` | CoBo | `Module.writingClockFrequency` | real、`unit="MHz"`、range `[1.0, 200.0]` | 25.0 | 位置一致。xcfg の range は連続区間だが、表の enum {12.5, 25, 50, 100} を採用(実ファイルでの出現値: 12.5 / 25.0 / 100.0) |
| `triggerMode` | CoBo | `Module.triggerMode` | enum、range `{external, external_2p, noTrigger, onMultiplicity, onMultiplicity_2p, onRequest, onRequest_2p, onSlowControl, onSlowControl_2p, periodically, periodically_2p}` | external | 位置一致。受理する値は §2.1 裁定 1 に従い FW が扱える 5 値 `{external, noTrigger, onRequest, onSlowControl, periodically}` のみ(実ファイルでの出現値: external / periodically / onSlowControl) |
| `globalThreshold` | AGET | `Global.Reg1.GlobalThresholdValue` | int、range `[0, 7]` | 1(pedestals は 7) | 表は `Aget/Global/GlobalThresholdValue` = **`Reg1` が抜けていた**。スコープ・要素名・range は一致 → 訂正して実装 |
| `lsbThreshold` | channel | `LSBThresholdValue` | int、range `[0, 15]` | 15 | 位置一致 |
| `gain` | channel | `Gain` | enum、range `{10pC, 120fC, 1pC, 240fC}` | 120fC | 位置一致 |
| `triggerInhibition` | channel | `TriggerInhibition` | enum、range `{inhibit_channel, inhibit_trigger, none}` | none | §2.1 裁定 2 で採用。位置一致 |

補足:

- `channel id` は 0..67(raw ch、FPN 込み)。テンプレートは `channel id="*"` に全値を持ち、
  `channel id="0"` 〜 `channel id="67"` は**空の明示ノード**(`<channel id="N" />`)。
  同様に `Aget id="*"` + 空の `Aget id="0..3"`、`AsAd id="*"` + `AsAd id="0..3"`。
- `AsAd id="0..3"` の明示ノードは `Control.isActive` と `CircularBuffer.*` を持つ(= 管理対象外ノブ)。
  `Instance id="1..11"` は `Module.coboId` を持つ(= 管理対象外)。**管理ノブを明示 id で上書きしている
  箇所は 5 本のテンプレートのどれにも無い**(除去規則 §3-1 は現行テンプレートでは no-op だが、
  将来のテンプレート改訂に対する保険として必須 → 負性テストを合成フィクスチャで持つ)。

## 4. 表と食い違ったノブ(2026-09-22 のレビュー裁定で位置を確定し、実装済み)

### 4-1. `peakingTime_ns` — スコープと要素名が違う → **AGET スコープで実装**

- 078 §2 表: スコープ channel / `Aget/channel/peakingTime`
- 実物: **AGET スコープ**の `Global.Reg1.peackingTime`(**GET 側のスペルは `peackingTime`**)、
  `unit="ns"`、range `{70, 117, 232, 280, 334, 383, 502, 541, 568, 632, 721, 760, 831, 870, 976, 1014}`。
  channel 配下に peaking time 相当の要素は**無い**(AGET レジスタ 1 = chip 単位のため)。
- **裁定(2026-09-22)**: AGET スコープで実装。JSON のキーは `peakingTime_ns` のまま、
  xcfg の要素名は GET の綴り(`peackingTime`)のまま。例外は AGET 粒度まで(`at` は `{asad, aget}`)。
- 値域は `src/params/schema.rs` の定数 `PEAKING_TIME_NS` = **AGET 仕様の 16 値**
  (実 xcfg の `range` 属性がそのまま列挙しているもの)。テンプレートから実行時に読まない。
  `tests/params_render.rs::the_peaking_time_table_matches_the_template_range_attribute` が
  定数表とフィクスチャの `range` 属性の一致を確かめる。
- 実テンプレートに現れる値: **232**(configure-physics 25/12.5 MHz + configure-pedestals 2 本)、
  **1014**(configure-pulser)。

### 4-2. `readout` — 合成ノブ。**差分に現れた 2 要素だけを管理する**

裁定(2026-09-22)は「`full` = `configure-physics-…_trigDelay1748.xcfg` の値、
`partial` = `configure-pulser.xcfg` の値を**差分から取る**」。候補 4 要素の実測:

| 要素 | スコープ | physics(= full) | pulser(= partial) | 管理 |
|---|---|---|---|---|
| `Module.enableWriteHittedregister` | CoBo | true | true | **しない** |
| `Global.Reg1.isAllChannelRead` | AGET | true | **false** | する |
| `Global.Reg1.isFPNRead` | AGET | true | **false** | する |
| `channel.Reading` | channel | always | always | **しない** |

- **差分に現れたのは AGET の 2 要素だけ**。`enableWriteHittedregister` と `Reading` は 2 本で同値なので、
  「差分から取る」方法ではモード依存の値が得られない。
- さらに `enableWriteHittedregister` は **configure-pedestals 2 本では `false`**。
  もし「full なら true」と一様に書くと、pedestals テンプレートを選んだ run の挙動を**勝手に変えて**しまう
  (往復のバイト一致も壊れる)。よって管理対象から外し、テンプレート由来のままにした。
- `Reading` は xcfg のコメントどおり `isAllChannelRead=false` かつ `enableWriteHittedregister=true` の
  ときだけ効く別軸の方針(ch ごとの読み出し条件)。pulser テンプレートでも `always` のまま。
- 実装: 同じ JSON キー `readout` を持つ [`KnobSpec`] が 2 つあり(`isAllChannelRead` / `isFPNRead`)、
  `full → true` / `partial → false` の対応表で展開する。
- **要確認(Fable)**: 上の 2 要素を管理対象から外した判断。もし `enableWriteHittedregister` も
  readout ノブに含めたいなら、pedestals テンプレートの `false` をどう扱うか(= テンプレート毎に
  full の意味が違う)を決める必要がある。
- 067 の注意(partial にすると frameType が 1 に変わる)は UI 側(レーン C)の表示事項として残る。

### 4-3. `zeroSuppression` — enabled と threshold のスコープが違う → **内部で 2 ノブ**

- 078 §2 表: CoBo / `CoBo/Module/enableZeroSuppression + zeroSuppressionThreshold`
- 実物:
  - `Module.enableZeroSuppression`(CoBo、false)✔
  - `Module.zeroSuppressionInverted`(CoBo、false)— 表に無い
  - **`zeroSuppressionThreshold` は CoBo に無く、channel スコープ**(`channel[c].zeroSuppressionThreshold`、
    range `[0, 4095]`、現行 600)。xcfg のコメントも「Threshold for each channel is defined by
    parameter Aget.channel.zeroSuppressionThreshold」と明記。
- **裁定(2026-09-22)**: JSON の形は `{"enabled": bool, "threshold": int}` のまま、**内部では 2 ノブ**。
  - `zeroSuppression.enabled` → `Module.enableZeroSuppression`(CoBo スコープ)
  - `zeroSuppression.threshold` → `channel[c].zeroSuppressionThreshold`(channel スコープ、range [0,4095])
  - 例外は `"knob": "zeroSuppression.threshold"` にだけ付けられる。`zeroSuppression.enabled` は
    CoBo スコープなので、どんな `at` も「スコープより細かい」= スキーマ違反として拒否される
    (特別扱いのコードは要らない)。
- `Module.zeroSuppressionInverted` は**管理しない**(表にも無く、5 本とも false)。

### 4-4. `pulser` — スコープが AsAd ではなく CoBo → **CoBo スコープで実装**

- 078 §2 表: AsAd / `AsAd/Generator/…`
- 実物: 2 つの `Generator` がある。
  - `Node[CoBo].Instance[i].Generator`(**CoBo スコープ**)= `periodicModeEnabled`(physics false /
    pulser **true**)、`amplitudeStart`(mV、range `[-1000.0, 1000.0]`、-1000.0)、`period_ms`(ms、1000)、
    `amplitudeStop` / `amplitudeStep` / `doublePulseEnabled` / `secondPulseAmplitude` /
    `poissonModeEnabled` / `defaultVoltage` / `externalTriggerMode`
  - `Node[CoBo].Instance[i].AsAd[a].Generator`(AsAd スコープ)= `isCalibMode` / `injectValue`
    (range `{11pF, 1pF}`)/ `triggerDelay`(WCK periods) — **{enabled, voltage, period} は無い**
- **裁定(2026-09-22)**: **CoBo スコープ**の `Instance.Generator` に 3 フィールドを写す。
  - `pulser.enabled` → `Generator.periodicModeEnabled`
  - `pulser.voltage_mV` → `Generator.amplitudeStart`(range [-1000.0, 1000.0] mV)
  - `pulser.period_ms` → `Generator.period_ms`
- **単位換算は不要(同値)**。一次資料 `GetBench/src/get/rc/CoBoConfigParser.cpp:1074`
  `CoBoConfigParser::asadPulserVoltage_mV` は `Generator.amplitudeStart` をそのまま
  「出力電圧と基準電圧(Vref = 1102 mV)の差 [mV]」として返す(`int16_t` に切り捨て)。
  xcfg の属性も `unit="mV"`、078 §2 の JSON 例の `-1000` もテンプレートの `-1000.0` と一致する。
  → **JSON のキー名は `voltage_mV` のまま**(`amplitude` への改名は不要)。
  小数を書いても GET 側で整数 mV に切り捨てられる点だけ注意。
- `Global.Reg1.TestModeSelection`(pulser では `functionality`)と
  `channel.isSelectedforTestMode`(pulser では `true`)、`Generator.defaultVoltage` は
  **管理しない = テンプレート由来のまま**。**パルサ run はテンプレートとして
  `configure-pulser.xcfg` を選ぶ運用**(JSON でパルサを「入れる」のではなく、
  パルサ用テンプレートに切り替えて `pulser.*` を微調整する)。

## 5. describe 相について

`describe-elitpc.xcfg` はルートが `Setup id="Hardware"`(configure/prepare は `Setup id="Conditions"`)で、
中身は `HardwareConfigId`(= `zCobo`)と `Endpoint`(IP の `addressField id="0..3"` と `port`)だけ。
上記 11 ノブはどれも現れない。078 の管理対象外(§2「含めない」+ FW とペアで不変)。

## 6. 実装中に判明した CompoundConfig の解決規則(078 §2 の例と食い違う)

**結論: 粗い例外を `<AsAd id="1"><Aget id="*"><channel id="*">` の形で書いても効かない。**
2026-09-22、`tools/ccfg_resolve`(実 GET の CompoundConfig)で実測。

### 一次資料

`reference/20190315_patched/CompoundConfig/src/CCfg/View/Search.hpp` の `search()`:

```cpp
string index = (btStart) ? "*" : iPath->index;
NodeIterator child = current.search(iPath->name, index);
if(child == current.end())  // not found
{
    // if we already found instances beyond the current point
    // and we have not already tried the default object.
    if(iInstance >= iPath and index != "*")   // ← ここ
    { ... index = "*"; ... }
    else { return 0; }
}
```

つまり **ある段で明示 id が無いとき、その場で `*` に落ちるのは「より深い段で既に実体を
見つけている」ときだけ**。通常は `backtrack()` が「1 段戻って `*` を選び直して降り直す」ことで
継承を実現する。この降り直しは、**降りる先の段に明示 id ノードが実在する**ことを前提にする。

実テンプレートが `<channel id="0" />` … `<channel id="67" />` のような**空の明示ノードを
全部並べている**のはこのため(飾りではなく、解決を成立させるための骨組み)。

### 実測(ccfg_resolve、合成フィクスチャ)

`<AsAd id="1"><Aget id="*"><channel id="*"><LSBThresholdValue>7` を置いて
`Node[CoBo].Instance[0].AsAd[1].Aget[0].channel[0].LSBThresholdValue` を引くと、
**7 ではなく一様値(`AsAd[*]` 側の 12)が返る**(22/158 インスタンスが不一致)。
探索は `AsAd[1]` に降りた後 `Aget[0]` を見つけられず、`AsAd[1]` ごと諦めて
`AsAd[*]` の連鎖に戻るため。

### 078 レーン A での対処(レンダラ)

スコープより粗い `at` は、**テンプレートに実在する id の直積に展開**して明示ノードで書く
(`{"asad":1}` の lsbThreshold → `AsAd[1]/Aget[g]/channel[c]/LSBThresholdValue` を全 (g,c) 分)。
JSON の意味(「AsAd1 全体」)は変わらず、`Params::effective_value` の解決(§3-4)も変わらない。
**変わるのは生成 xcfg の形だけ**。ELITPC 実寸での実測: 4 例外(うち 1 つが AsAd 全体の gain =
272 ノードに展開)で 831 行 → 1,668 行(44 KB → 62 KB)。

**裁定済み(2026-09-22 レビュー)**: 選択肢 **(a) = 直積への展開**で確定。
078 §2 の「`{"asad":1}` だけなら … `<AsAd id="1"><Aget id="*"><channel id="*">`」という記述は
**実機の解決器と合わない**ので、SPEC §3 にはこの節の方式(展開)を写す。
生成物が大きくなるのは許容(JSON の意味は不変)。

## 7. 実装済みノブ一覧(JSON キー → xcfg。2026-09-22 時点)

`src/params/schema.rs` の `KNOBS` が唯一の定義。スコープ深さ = CoBo 0 / AsAd 1 / AGET 2 / channel 3。
`exceptions[].at` はこの深さ**以下**(= 同じか粗い)だけ許され、粗い場合は実在 id の直積に展開される(§6)。

| JSON キー | スコープ | xcfg(スコープノードからの相対) | 型・許容値 |
|---|---|---|---|
| `triggerDelay` | CoBo | `Module.triggerDelay` | int ≥ 0(unit 10ns) |
| `writingClockFrequency_MHz` | CoBo | `Module.writingClockFrequency` | {12.5, 25, 50, 100} |
| `triggerMode` | CoBo | `Module.triggerMode` | {external, noTrigger, onRequest, onSlowControl, periodically} |
| `zeroSuppression.enabled` | CoBo | `Module.enableZeroSuppression` | bool |
| `pulser.enabled` | CoBo | `Generator.periodicModeEnabled` | bool |
| `pulser.voltage_mV` | CoBo | `Generator.amplitudeStart` | real [-1000, 1000] |
| `pulser.period_ms` | CoBo | `Generator.period_ms` | int ≥ 0 |
| `globalThreshold` | AGET | `Global.Reg1.GlobalThresholdValue` | int [0, 7] |
| `peakingTime_ns` | AGET | `Global.Reg1.peackingTime` | `PEAKING_TIME_NS` の 16 値 |
| `readout` | AGET | `Global.Reg1.isAllChannelRead` + `Global.Reg1.isFPNRead` | {full, partial} |
| `lsbThreshold` | channel | `LSBThresholdValue` | int [0, 15] |
| `gain` | channel | `Gain` | {10pC, 120fC, 1pC, 240fC} |
| `triggerInhibition` | channel | `TriggerInhibition` | {inhibit_channel, inhibit_trigger, none} |
| `zeroSuppression.threshold` | channel | `zeroSuppressionThreshold` | int [0, 4095] |

**管理しない = テンプレート由来のまま**の主なもの(意図的): `Module.enableWriteHittedregister`、
`channel.Reading`、`Module.zeroSuppressionInverted`、`Global.Reg1.TestModeSelection`、
`channel.isSelectedforTestMode`、`Generator.defaultVoltage`、`Module.coboId` / `nbEvent` /
`readoutDepth` / `frameType`、`AsAd.Control.isActive`、`AsAd.Generator.*`、PLL / CircularBuffer /
Monitoring / InspectionLines / LEMO / Alarms、`Node[Mutant]` 配下すべて。

## 8. 抽出(render の逆、TODO/111)

`src/params/extract.rs` の `extract(name, xml)`(純関数)= `GET /api/params/defaults?template=<name>` の中身。UI の初期値をテンプレート自身から読む(直書き既定値で mini の 100 MHz が 25 MHz に化ける穴を塞ぐ)。
- `knobs`: `Instance[*]` から `*` の連鎖で読めて、`KnobKind` の型・範囲(`Params` の検証と同じ検査)に収まった葉だけ。
- `unset`: `*` の連鎖に要素が無い葉(`.` 区切り)。**既定値で埋めない**。合成ノブは構成要素が 1 つでも無ければ unset。
- `unsupported` `{knob, found, why}`: 型・範囲外 / 合成ノブ(`readout`)の構成要素が同じモードに写らない / 明示 id ノードの値が例外で表せない。`found` は xcfg のテキストそのまま(合成は `isAllChannelRead=true,isFPNRead=false`)。
- `exceptions`: `Instance[*]` の下で**明示 id が先頭から連続する**ノードの値。`at` = その明示 id の接頭辞(`AsAd[1].Aget[*].channel[*].Gain` → `{asad:1}`)。
- 表せない明示ノード → `unsupported`: `*` の下の明示 id(`AsAd[*].Aget[1]…`、why `explicit id below a wildcard`)、CoBo の明示インスタンス(`Instance[0]…`、why `explicit CoBo instance`)、合成ノブの構成要素が同じノードに揃わない。
- render の除去規則(§3-1)は表せない明示ノードを消すので、「抽出 → 保存」の後にもう一度抽出すると `unsupported` は空になる(knobs / exceptions は不変 = 冪等)。
- 受け入れ: `tests/params_extract.rs`(合成フィクスチャ + env `TPCDAQ_PARAMS_REAL_TEMPLATES` の実テンプレート往復)、`tests/params_ccfg_oracle.rs`(抽出値の有効値 = ECC の解決値)。
