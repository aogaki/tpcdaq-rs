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
