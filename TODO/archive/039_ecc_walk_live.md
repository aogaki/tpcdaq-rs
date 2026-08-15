# 039 — 実 ECC 歩きの実走検証(mini プロファイル、シーダ実験)

**Status: COMPLETED**(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS。
スクリプト・ログ・スタブの全成果物 = `reference/_spike/run039/`)

## 結果

- **到達点: フルウォーク完走。** mini プロファイル(fullCoBoStandAlone **無改変** — diff で
  バイト一致確認)で Idle → Described → Prepared → Ready → Running → Ready → Prepared →
  Described → Idle の全遷移 `result: 0`。hwServer 側の例外・警告 **0 件**。
  自作コードは 46004 スタブ(daqstub.cpp、約 120 行)のみ。
- **実測所要**: describe 0.10 s / **prepare 4.85 s** / **configure 2.21 s** / start 0.22 s /
  stop 0.63 s / breakup 0.63 s / reset 0.09 s。**run 開始レイテンシの下限 ≈ 7 s**
  (ECC 内部 sleep が支配的。`ecc_timeout` 60 s は十分)。
- **R2(シード要否)= 解決**。実測表(§4.2 の机上表と突合):
  **シード必須 4 つ** = `ctrl`/`asadConnection.PLG0`=1、`asad`/`monitorID`=0x41、
  `asad`/`VDD`=0xCD(=3.01 V ≥ 閾値 3 V)、`aget`/`reg5`=0x201(1 レジスタで 4 チップ分 —
  ECC が agetChipSelect で切替えて同一レジスタを読む)。
  **AIM USR は ECC 自身が書く → シード不要**(CoBoNode.cpp:1171→2051 で確定)。
  memStart/温度/firmware 版数は**チェックされない**(全ゼロで可)。
  **唯一の例外 = `asadStatus.alarm`**: power-off 後 1・power-on 後 0 を**同一ビット**に要求 —
  静的メモリでは原理的に両立不能。`checkPowerSupply=false`(configure xcfg 1 行)で回避を実証。
- **R1(レース)= 解決**。ポーリング型シーダは **5/5 敗北**(describe 0.10 s、窓は構造的に
  ゼロ)。**失敗後リトライは 5/5 成功**(失敗 prepare は `Described/WHEN_PREPARE` に留まり、
  シード → prepare 再発行のみで回復。再 describe 不要)。同一セッション内
  describe→seed→prepare は race-free で一発 green。
  **シードの寿命**: prepare 再試行・breakup・reset(Prepared→Described)を**跨いで生存**、
  **describe で消える**(destroy+create)= describe 毎に再シード要。
  NodeConfig に値フィールドは存在しない(HardwareTypes.ice)= xcfg 初期値の道は無い。
- **シーダ v0(CLI)成立**: `getEccClient` の `node-select`/`device-select`/`field-write`/
  `reg-write` で 8 行(`run039/seed_mini.ecc`)。readOnly 非強制を実書き込みで確認。
- **46004 スタブ実録**(docs §4.2/4.3 と突合、全て実測):
  encoding **1.0**(ワイヤ実測)/ identity `DaqCtrlNode`・type `::mdaq::DaqCtrlNode` /
  `ice_isA` は **configure 毎に 1 回のみ**(プロキシキャッシュ — 46001 の HwNode とは別挙動。
  docs 要注記)/ `connect("TCP", "127.0.0.1:46005")` — 第 2 引数は素の `ip:port` 文字列 /
  configure 末尾に `setCircularBuffersEnabled(0x1)` 必須 / **`daqStop` は breakup で 2 度目が
  来る = 冪等必須** / Prepared からの再 configure は `disconnect` 無しで `connect` が来る =
  **旧センダ破棄は connect 実装側の責務** / Ready での configure 再発行はスタブに何も届かない
  (ST_PREPARED ガード実測)/ `setAlwaysFlushData`/`shutdown` は一度も来ない /
  46004 不在時の configure は `PREPARED/WHEN_CONFIGURE` に健全着地し、スタブ起動後の再発行で
  Ready 回復(ハング無し)。
- **prepare の設定読み込みは実測でも `configure-` プレフィクス**(SPEC v1.13 の注記を確認)。
- **受け入れ検証**: reference/ 正本 2 ディレクトリ `find -newermt` 0 件。リポのコード無変更
  (並行 042 の diff と切り分け済み)。残留プロセス無し。
- **設計分岐 4 件を発注側へ返却 → Fable 裁定(2026-08-15、docs v1.1 に反映)**:
  ①シーダの置き場 → **どれでもなく「vcobo-daq が 46001 も名乗る」に方針転換**(外部シーダは
  レースに構造的に勝てず、毎 run describe でシードが死ぬ。controller フックは実機 footgun)
  ②getEccClient の demo 常用 → 主経路では不要化(_spike にデバッグ用として残す)
  ③prepare_id の名前空間 → SPEC v1.13 注記済み(042 に影響なし)
  ④demo xcfg のリポ同梱 → 従来どおり当面リポ外(R6)。
- 実行環境・日付: macOS Darwin 25.5.0(arm64)、Ice 3.7.11 keg、2026-08-15。

---

(以下、起票時の発注書)

**Status(起票時): READY**(起票 2026-08-15 Fable。033/031 と独立)
**起票**: 2026-08-15(038 の結果を受けた次フェーズ。フォーク B の最終設計リスクを実測で閉じる)
**仕様の正**: [docs/VIRTUAL_ZCOBO_ja.md](../docs/VIRTUAL_ZCOBO_ja.md) v1.0 — 特に §4.2
(チェック付き read-back 表・最小 Ice 面)/ §4.3(データリンク挙動)/ §6(R1/R2)
**前提資産**: `reference/_spike/`(038 成果 — `prefix/bin/{getEccServer,getEccClient,getHwServer}`、
`build_all.sh` / `env.sh`、実行レシピは `run/` のログ参照)
**関連**: [archive/038_real_ecc_local_spike.md](archive/038_real_ecc_local_spike.md)

---

## 目的

フォーク B に残る設計リスクは 2 つだけ(docs §6):
**R1**(シーダ = describe→prepare 窓のレース)と **R2**(Sim レジスタの「書いた値がそのまま
読める」意味論で ECC のチェックを通せるか)。これを **mini プロファイル
(fullCoBoStandAlone)の実走**で閉じ、040(vcobo-daq 実装)の発注書を確定させる材料を取る。
038 で describe の入口(`Added HW node`)までは実証済み — 本チケットはその先、
**describe 完走 → prepare 完走 → configure が Ready に到達**するまでを歩く。

## やること

### A. 環境の再現(038 レシピどおり)

- `reference/_spike/prefix/bin/` の getEccServer + getHwServer を起動
  (**getHwServer は `--Ice.IPv6=0` 必須**。env.sh の注意書きどおり Ice 3.7/3.8 混線に注意)。
- config repo(フラットディレクトリ)に mini 用セットを用意:
  `describe-mono-node.xcfg` 派生(Endpoint=127.0.0.1、port 46001、
  `HardwareConfigId=fullCoBoStandAlone`、`batchProcessing=false`)+ 対応する
  `configure-*.xcfg`(`GetBench/data/config/` の実例から派生)。
  **置き場は `reference/_spike/run/configs/` — リポに入れない**(CeCILL/内部情報の既定方針)。

### B. describe 完走

- `getEccClient` の `sm-describe` で **Described 到達**を確認(create の 6594 行 NodeConfig 投入、
  getMapOfDevices、setName、mutantConfig 書き込みまで全部通るか)。
- 落ちた場合: どの呼び出し・どのデバイスで落ちたかを ECC/hwServer 両ログで特定して記録。

### C. シーダ実験(R2 の実測)— **まず「自作ゼロ」の経路から試す**

- **v0 = ECC 自身の CLI で書く**: 038 レーン B の棚卸しで、`EccBackEnd` の
  `writeRegister/writeField`(EccBackEnd.cpp:162-197)は **ECC の CLI(getEccClient)から
  到達可能**と判明している。describe 後に getEccClient のレジスタ書き込みコマンドで
  §4.2 の表の魔法値(`asad`/`monitorID`=0x41、`aget`/`reg5`=0x201、`ctrl`/`asadConnection.PLG0`=1、
  `ctrl`/`asadStatus.alarm`、`asad`/`VDD`、AIM USR)を仕込めるか確認。
  **書ければシーダは「コマンド列」で済み、自作コード不要になる。**
- v0 が塞がっていたら v1: slice 生成物(_spike 内)から 20〜50 行の Ice クライアントで
  46001 の Device servant に直接 `writeRegister/writeField`(readOnly 非強制は確認済み)。
- その後 `sm-prepare` を実走し、**チェックのどれで落ちるか / 全部通るか**を 1 項目ずつ記録。
  **AIM USR が ECC 自身の書き込みか(シード不要か)もここで判明する。**
  `checkPowerSupply=false` 等、prepare xcfg 側で緩和できるチェックはどれかも記録。

### D. レース計測(R1 の実測)

- describe 完了 → 即 prepare(我々の controller の run/start シーケンス相当の最短連打)で、
  シード挿入が間に合うかを実測。
- 間に合わない場合の**実挙動**を記録: prepare のエラー内容、ECC の着地状態
  (`WHEN_PREPARE` 等)、**再試行(シード → prepare 再発行)で回復するか**。
  「初回失敗 → リトライで回復」が成立するなら、040 のシーダ設計は単純化できる。

### E. configure 到達(46004 スタブ)

- 46004 に**聞くだけの DaqCtrlNode スタブ**を置く: `ice_isA` / `connect` / `disconnect` /
  `setCircularBuffersEnabled` / `daqStart` / `daqStop` に成功応答するだけ
  (**encoding 1.0 のリクエストを受けること** — docs §4.2)。slice 生成物 + 数十行、
  **_spike 内に置き、リポに入れない**(リポ持ち込みの判断は 040)。
- DataLinkSet(`type="TCP"`、`DataSender id="CoBo[0]"`)を渡して `sm-configure` を実走し、
  **Ready 到達**を確認。スタブに**実際に届いた呼び出し(メソッド・引数・順序)を全部実録**
  する — これが 040 の仕様確定材料(docs §4.2 の机上の表との突合)。
- 余力があれば `sm-start`/`sm-stop` まで(daqStart/daqStop がスタブに届くことの確認。
  データ送出はスコープ外)。

### 非スコープ

- vcobo-daq の実装(graw 送出・本物のシーダ組み込み)= 040。
- SPEC §8.2 の config_id 3 相化 diff の適用 = 040 と同時(裁定は済み — docs §6 R9)。
- ELITPC プロファイル(registerAccess=zCobo、R8)。
- リポのコード変更(Rust/C++ 本体に一切触らない)。

## 受け入れ

- 結果節に:
  ① describe / prepare / configure(/ start・stop)の**到達点と落ちた箇所**(両ログの該当行)
  ② **シード要否の実測表**(§4.2 の表と 1 対 1 で突合: 各値について「シード必須 / ECC 自身が
  書く / xcfg で緩和可 / チェックされない」のいずれかを実測で確定)
  ③ **レース実測**(最短連打での成否、失敗時のエラーと回復手順)
  ④ **スタブに届いた呼び出しの実録**(メソッド・引数・順序)
  ⑤ シーダ v0(CLI)成立可否 ⑥ ログ・スクリプト一式の場所(_spike 内)。
- `git status` クリーン(リポ内変更は本 md のみ。成果物は全て `reference/_spike/`)。
- reference/ 正本 2 ディレクトリ無変更(038 と同じ `find -newermt` 検証)。
- **発注先想定**: implementer/**Opus**(Ice スタブ + 実走デバッグの判断が要る。
  テストで縛れない探索型)。完了後の docs §4/§6 更新と 040 起票は主対話(Fable)の仕事。
