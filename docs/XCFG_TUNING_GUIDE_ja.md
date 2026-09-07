# GET xcfg 設定・チューニングガイド

- **status**: v1.0(2026-08-18、TODO/072)
- **性格**: 実務ガイド(SPEC ではない)。**一次資料 = `reference/config/`(実 ECC の生コピー、
  TODO/065)+ 実験ログブック(TODO/071 = `reference/logbook/_survey/SURVEY.md`)**。
  ここに書く「何を見ながら調整するか」は 5 年分のログブックで実際に行われてきた手順の要約。
- **読者**: xcfg を初めて触る人(= 我々)。ELITPC/mini の現行値を基準に書く。

---

## 1. まず全体像 — 設定は 3 層 + 1

GET の設定は役割の違う 3 種の XML と、run 毎に注入される 1 断片からなる:

| 層 | ファイル | ECC 状態遷移 | 触るか |
|---|---|---|---|
| ① セッション選択 | `workspace.xcfg` | (GetController がテストを選ぶ) | 実験の切替時のみ |
| ② ハード記述 | `hardwareDescription_zCobo.xcfg`(describe 系) | **describe** | **触らない**(FW とペア。2017 年から不変 = 065) |
| ③ 実験条件 | `configure-*.xcfg`(Conditions) | **prepare → configure** | **チューニングは実質ここだけ** |
| ④ データリンク | `DataLinkSet`(XML 断片) | configure の引数 | 我々の DAQ では controller が生成(SPEC §8.2) |

- ①は「Hardware パス + Conditions パス + Data 出力先 + Network(ECC/DAQ/TARGET の IP)」の
  組をテスト名で束ねたカタログ。現行実験のアクティブなテストは `Physics_extTrg_zCobo`
  (065 の結果節参照)。
- ②はレジスタ意味論とトポロジの記述で、**ワイヤ上の挙動の真実は FW 側**にある。
  変更対象ではない(CLAUDE.md「制御プレーンを改変しない」)。
- ③が本ガイドの主題。

## 2. Conditions xcfg の読み方 — ワイルドカード継承

構造は `Node → Instance → (CoBo) Module/AsAd → Aget → channel` の木で、
**`id="*"` がデフォルト、個別 id が差分上書き**という継承規則がすべて:

```xml
<AsAd id="*"><Control><isActive>false</isActive></Control>...</AsAd>  <!-- 既定: 無効 -->
<AsAd id="0"><Control><isActive>true</isActive></Control></AsAd>       <!-- AsAd0 だけ有効化 -->
```

この流儀のおかげで、実験側の config ファイル群は**「基準ファイル + ほぼ 1 行差分」**で
運用されている。実証(065): physics 系 xcfg は 2022-04 から現在まで
**差分が `triggerDelay`(とクロック)だけ**。つまり「たくさん設定できる」が
「実際に動かすのは数個」— これがチューニングの現実。

## 3. 実際に触るノブ(頻度順)

### 3.1 毎ラン級 — triggerDelay(唯一の常用ノブ)

```xml
<triggerDelay unit="10ns">1748</triggerDelay>
```

- 意味: **トリガ発生から AGET SCA(リング状 512 セル)の書き込み凍結までの遅延**。
  SCA は常時ぐるぐる書いているので、delay は「トラックを 512 セル窓のどこに置くか」を決める。
- 窓の長さ = 512 / f_sample(25 MHz → 20.48 µs)。delay = 1748×10 ns = 17.48 µs なら
  「トリガ後 17.48 µs + トリガ前 3.0 µs」が見える。
- **設計式(070/SURVEY)**: 見たい drift 深さ d に対し t = d / v_drift が窓に収まること。
  現行(130 mbar、v=0.390 cm/µs、70 mm 深)は t ≈ 18 µs で窓 20.48 µs にほぼぴったり
  (066 実測: 475/502 セル使用)。
- ログブックの標準対応(SURVEY §3): **1600@25MHz / 3200@12.5MHz / 800@50MHz** →
  2022 以降 1848@25MHz → HIGS2026 現地で 1848→1660→**1748** と微調整(065)。
- **何を見て調整するか**: イベント表示(monitor UI の StripTime/Waveform)で
  ①トラックが窓の端で切れていないか ②トリガ前ベースライン(較正に使う頭の空白)が
  数十セル残っているか。066 のベースライン窓 64 セルはこの空白を使っている。

### 3.2 実験切替級 — クロック・ゲイン・シェーパ

```xml
<writingClockFrequency unit="MHz">25.0</writingClockFrequency>  <!-- CKW: 12.5/25/50/100 -->
<peackingTime unit="ns">232</peackingTime>                       <!-- AGET シェーパ -->
<Gain range="{10pC, 120fC, 1pC, 240fC}">120fC</Gain>            <!-- channel 毎 CSA 容量 -->
```

- **f_sample は「深さ vs z 分解能」のトレードオフ**(070 §4)。E_drift・delay と三点セット
  で決める。運転点表(SURVEY §2)では深い構成(150 mm@250 mbar)だけ 12.5 MHz。
- peaking 232 ns は 2021 年の 70/117/232/568 ns 系統スキャン(logbook p.176–184)を経て
  収斂した値。**変更したら trigger delay の最適値もズレる**ことに注意(応答の遅延が変わる)。
- Gain 120 fC は飽和と分解能の妥協点。pulser で確認(FPN は 4095 飽和 = 068)。
- `readingClockFrequency`(CKR)は **25 MHz 固定**(仕様)。

### 3.3 ランの種類 — トリガモードと readout モード

| ランの種類 | triggerMode | readout | frameType(実測) |
|---|---|---|---|
| physics | `external` | フル(isAllChannelRead + isFPNRead、ZS off) | 2(compact、固定長 278,784 B) |
| pedestal | `periodically`(100 ms) | フル | 2 |
| pulser | `onSlowControl` 系 | **部分**(isAllChannelRead=false / isFPNRead=false / hit register on) | **1**(itemSize 4、557,312 B — 067) |

- **readout モードを部分にすると compact(2)から frameType 1 に落ちる**(067 実測)。
  デコーダは両対応だが、ファイルサイズ・レート見積りが変わるので意識すること。
- `enableWriteHittedregister` + `Reading`(always/never/only_if_hit)+
  `TriggerInhibition` が hit ベース読み出しの三点セット。physics では常にフル。

### 3.4 較正系 — Generator(AsAd 内蔵パルサー)と閾値

```xml
<Generator><amplitudeStart unit="mV">-1000.0</amplitudeStart>
           <periodicModeEnabled>false</periodicModeEnabled>...</Generator>
<GlobalThresholdValue range="[0, 7]">1</GlobalThresholdValue>       <!-- DAC 上位 3bit -->
<LSBThresholdValue range="[0, 15]">15</LSBThresholdValue>           <!-- channel 毎 下位 4bit -->
```

- パルサー較正は実験側の**毎セッション定型**(「ASAD pulser test OK」— 071)。
  我々の使い道: per-ch ゲイン表 + 電子回路応答(068 / 069 C-2)。
- 閾値(Global 3bit + LSB 4bit)は **multiplicity トリガと ZS のためのもの**。
  外部トリガ運用(現行)では波形取得に効かない。pedestal 用 xcfg が
  GlobalThreshold=7 にしているのは内部トリガを黙らせるため(065)。
- **HIGS の実トリガ閾値は xcfg の外**(プリアンプ+ディスクリの NIM 系、-55 mV 等 — 071)。
  混同しないこと。

### 3.5 めったに触らない — 構成・識別子

- `AsAd id=N / isActive`: 構成(mini 1 / ELITPC 4)。ジオメトリ .dat と整合させる。
- `coboId` / `dataSource`: フレームヘッダに載る識別子。**注意: dataSource は xcfg で 0 でも
  ワイヤは 1**(2022/2026 全実データで実測 — 067。実害なし、照合時に驚かないこと)。
- `readoutDepth`(512)/ `nbEvent`(GetController のテスト用 stop-after-N)/
  `Module.frameType`(**deprecated** — 実際のワイヤ形式は FW と readout モードで決まる)/
  CircularBuffer(AsAd 毎 32 MB 割り)— いずれも動かさない。
- ZS(`enableZeroSuppression` + `zeroSuppressionThreshold`): 2021-07 に実機試験済み
  (閾値 600、フレーム 1/3 化 — logbook p.123–126)だが**不採用のまま**(061 裁定とも整合)。
  使うなら frameType 1 化と閾値の per-ch 設計(068 のノイズ表が土台)が前提。

## 4. チューニングの実務ループ(何を見ながら回すか)

実験側の 5 年間の定型(071)+ 我々のツールで書くと:

1. **変更は 1 ノブずつ、ファイル名に条件を焼く**(実験側の流儀:
   `configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg`)。
   既存ファイルの上書き禁止 — 065 で見たとおり、履歴がそのまま系譜になる。
2. **pedestal ランで健全性確認**(実験側は 12.5/25 MHz を毎回巡回)。見るもの:
   per-ch ベースライン(224–512 ADC)とノイズ RMS(中央値 6.64、FPN 2.05)—
   **基準値は 068 の `reference/_spike/asset_survey/out/`**。逸脱 ch が増えたら設定より先に
   ハード(HV フィルタ・接地)を疑う(logbook のノイズ事件簿参照)。
3. **pulser ランでゲイン確認**: 相対ゲイン ±4.4%・AGET 系統差 5.3% が基準(068)。
4. **physics(または線源)ランで窓とトリガを確認**: StripTime 表示でトラック位置、
   `graw_replay` + decoder の counters(`malformed` / `unsupported` / `topology_frames` /
   フレームサイズ分布)が全て想定どおりか。イベントレートと `incomplete` 率。
5. **記録**: 我々の DAQ は run 毎に config の三つ組(042 の ConfigId)を logbook.jsonl に
  残す。実験側の紙ログブック相当はこれ。

## 5. 既知の罠(実測済みのものだけ)

- **internal Data Router モードの GetController はクラッシュ実績あり**(logbook p.227)→
  実験側は external(4×ASAD)運用。我々の DAQ は自前 receiver なので無関係だが、
  GetController で検証するときは external を使う。
- pulser xcfg の部分 readout は **frameType 1 を生む**(067)— リプレイ・解析側の想定に注意。
- `Module.frameType` は deprecated(触っても無意味)。
- 窓に対して v_drift が遅すぎる構成は「512 サンプルに収まらない」(logbook p.373 の実例)。
  E_drift・f・delay は**必ず三点セットで**考える(070)。
- xcfg の数値をいじる前に: **その値が本当に効く層か確認**(②ハード記述に同名っぽい
  ノブがあっても触らない)。
- ch 数(mini 256 / ELITPC 1024)を config やコードに焼き込まない — ジオメトリ .dat が正
  (CLAUDE.md 不変条件)。

## 6. 参照

- 現物: `reference/config/`(065 の結果節 = TODO/archive/065 が読み解き)
- 実運用の歴史: `reference/logbook/_survey/SURVEY.md`(071)
- 物理背景(E・f・delay の三点セット): `docs/DRIFT_DIFFUSION_PRIMER_ja.md`(070)
- 較正基準値: `reference/_spike/asset_survey/out/SUMMARY.md`(068)
- フレーム形式の実測: TODO/archive/067(frameType 1/2/7)
