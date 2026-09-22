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
- 実装: **7**
- 食い違い → 未実装・Fable 待ち: **4**(`peakingTime_ns` / `readout` / `zeroSuppression` / `pulser`)

## 3. 実装するノブ(7)

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

## 4. 食い違い(実装せず、Fable の裁定待ち)

### 4-1. `peakingTime_ns` — スコープと要素名が違う

- 078 §2 表: スコープ channel / `Aget/channel/peakingTime`
- 実物: **AGET スコープ**の `Global.Reg1.peackingTime`(**GET 側のスペルは `peackingTime`**)、
  `unit="ns"`、range `{70, 117, 232, 280, 334, 383, 502, 541, 568, 632, 721, 760, 831, 870, 976, 1014}`、現行値 232。
- channel 配下に peaking time 相当の要素は**無い**(AGET レジスタ 1 = chip 単位のため)。
- 影響: スコープが channel → AGET に変わると、例外リストの粒度(`at` の最細が `aget`)が変わる。
  JSON のキー名を `peakingTime_ns` のままにするか、xcfg のスペル(`peackingTime`)に寄せるかも要裁定。

### 4-2. `readout` — 単一パスが無い合成ノブ

- 078 §2 表: CoBo / `CoBo/Module/…`(= isAllChannelRead + isFPNRead + hit register の合成)
- 実物: 3 要素が**別スコープに分かれている**:
  - `Module.enableWriteHittedregister`(CoBo、physics=true / pedestals=**false**)
  - `Global.Reg1.isAllChannelRead`(**AGET**、true)
  - `Global.Reg1.isFPNRead`(**AGET**、true)
  - 加えて channel 単位の `Reading`(range `{always, never, only_if_hit}`、現行 always)が
    `isAllChannelRead=false` かつ `enableWriteHittedregister=true` のときだけ効く(xcfg のコメント)。
- `full` / `partial` の 2 値がこの 4 要素にどう写るか(partial のとき `Reading` をどうするか、
  `enableWriteHittedregister` を CoBo 一様にするか)は**発注書に書かれていない設計分岐**なので、
  勝手に決めない。加えて 078 §7 の「partial にすると frameType が 1 に変わる」警告の扱いも要裁定。

### 4-3. `zeroSuppression` — enabled と threshold のスコープが違う

- 078 §2 表: CoBo / `CoBo/Module/enableZeroSuppression + zeroSuppressionThreshold`
- 実物:
  - `Module.enableZeroSuppression`(CoBo、false)✔
  - `Module.zeroSuppressionInverted`(CoBo、false)— 表に無い
  - **`zeroSuppressionThreshold` は CoBo に無く、channel スコープ**(`channel[c].zeroSuppressionThreshold`、
    range `[0, 4095]`、現行 600)。xcfg のコメントも「Threshold for each channel is defined by
    parameter Aget.channel.zeroSuppressionThreshold」と明記。
- 1 つの JSON ノブが 2 スコープにまたがると、例外リストの意味(`{"asad":1}` で threshold だけ効く?)が
  発注書の「ノブ = 1 スコープ」モデルから外れる → 分割するか複合のままにするかは裁定事項。

### 4-4. `pulser` — スコープが AsAd ではなく CoBo

- 078 §2 表: AsAd / `AsAd/Generator/…`
- 実物: 2 つの `Generator` がある。
  - `Node[CoBo].Instance[i].Generator`(**CoBo スコープ**)= `periodicModeEnabled`(physics false /
    pulser **true**)、`amplitudeStart`(mV、range `[-1000.0, 1000.0]`、-1000.0)、`period_ms`(ms、1000)、
    `amplitudeStop` / `amplitudeStep` / `doublePulseEnabled` / `secondPulseAmplitude` /
    `poissonModeEnabled` / `defaultVoltage` / `externalTriggerMode`
  - `Node[CoBo].Instance[i].AsAd[a].Generator`(AsAd スコープ)= `isCalibMode` / `injectValue`
    (range `{11pF, 1pF}`)/ `triggerDelay`(WCK periods) — **{enabled, voltage, period} は無い**
- つまり JSON の `pulser.{enabled, voltage_mV, period_ms}` は AsAd ではなく **CoBo スコープ**。
  さらに configure-pulser.xcfg は `Global.Reg1.TestModeSelection=functionality` と
  channel の `isSelectedforTestMode=true` も併せて変えている(= パルサ運用はこの 3 系統の組)。
  「pulser を 1 ノブにまとめたときスコープと随伴ノブをどうするか」は裁定事項。

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

**要裁定(Fable)**: 078 §2 の「`{"asad":1}` だけなら … `<AsAd id="1"><Aget id="*"><channel id="*">`」
という記述は**実機の解決器と合わない**ので SPEC §3 に写さないこと。
(a) 今の展開方式でよいか、(b) 粗い例外自体を禁止して `at` はノブのスコープ丁度のみ許すか、
(c) 骨組み(空の明示ノード)を `AsAd[1]` 配下に複製する方式(ノード数は減るが探索の細部に依存)か。
