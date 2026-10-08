# 105 — イベント選別の総点検(multiplicity 以外も含む、検出器 → 解析の全段)

**Status: COMPLETED**(2026-10-08 起票・調査 Opus。ユーザー「multiplicity 以外でも何らかの形でイベントのフィルタリングが行われていないか精査して」)

## 方法
- .graw の全フレームヘッダを直接読む(`reference/_spike/stop_0929/grawscan.py`、CoboFormats-Rev-5-Compact.xcfg のオフセット)。対象 6 run(09-25 / 09-29 / 09-30 / 10-01 / 10-02 / 10-07)、計 162,948 フレーム。
- GET ソース(`reference/20190315_patched/GetBench/src/get/rc/CoBo{Node,ConfigParser}.cpp`)で設定値がどのレジスタに効くかを確認。
- 解析マクロ(graw2root / analyzeUVW / makeTracks)の事象数の保存。

## 結果(段ごと)
| 段 | 選別の可能性 | 判定 | 根拠 |
|---|---|---|---|
| 検出器 → フィルタ箱 → CFD(Vth)→ ゲートジェネレータ | 波高(電荷)閾値、出力幅・再トリガ禁止、veto 入力 | **ここが唯一の内容依存の選別** | 総電荷の下限 3.1e5 → 1.5e5(Vth 500 → 300、103)。幅・veto 配線はデータから見えない → ユーザー確認 |
| CoBo のトリガ受付 | モードの選択、busy 中の取りこぼし | 内容非依存の **約 1 s の不感時間のみ**(104) | ソース: `setTriggerMode` は選んだモード 1 つのマスクだけ立てる。`external` なら multiplicity / timing / slowControl は無効。`triggerPeriod` はレジスタに書かれるが、不感時間の端が 0.84〜1.0 s でばらつく(10 ns 精度の時刻で 1.000 s ちょうどの壁ではない)ので固定のファーム hold-off ではなく、転送・保存側の可変な待ちと判断 |
| AsAd / AGET の読み出し | 部分読み出し(readIfHit / readAlways マスク)、ゼロサプレッション、FPN 省略 | **無し** | 全フレームが frameType 1、itemCount 139,264 = 272 ch × 512 セル(全読み出し)、readOffset 0、status 0、windowOut 0xFFFFFFFF。AGET の hitPat は 16〜46 本と事象ごとに変わるが読み出しには使われていない。multip 欄は AGET 0〜3 で 1 / 2 / 4 / 8 の定数(実際の多重度ではない) |
| CoBo → dataRouter → 保存(VM) | フレームの破棄・間引き | **無し** | eventIdx は全 run で 0 から欠番なし(チャンク境界も連続)、ファイル末尾の切れ無し、トポロジーフレームは run 先頭の 1 個だけ |
| graw2root.C | データ以外のフレームと item サイズ異常のみ捨てる(警告付き) | **無し** | 09-30 / 10-01 / 10-02 / 10-07 はフレーム数 = raw の事象数。09-25(94,800 対 93,953)と 09-29(19,224 対 14,203)は run 継続中に変換した分で、末尾チャンクが未変換なだけ(連番の途中で切れている)|
| analyzeUVW / makeTracks | — | **無し**(事象は全部ツリーに残る。再構成失敗は ok = 0) | tracks の事象数 = raw の事象数 |
| 解析マクロのカット(stoppedAlphas 等) | 線源方向・左壁・停止判定 | 明示的なカットのみ | 各マクロ先頭の定数 |

## 内容に依存しない選別だが物理に効くもの
- **約 1 s の不感時間は時間相関のある崩壊を選択的に落とす**: ²²⁰Rn → ²¹⁶Po(T½ 0.145 s)は 99.2 % が 1 s 以内に崩壊 → ガス中の Rn–Po 遅延同時計数(083 で狙っていたもの)は現行 DAQ では原理的に見えない。²¹²Bi → ²¹²Po(0.3 µs)は同じ 20 µs の窓に入るので別問題(同一フレーム内の 2 本)。

## ユーザー確認事項
1. ゲートジェネレータの出力幅・モード(再トリガ可否)と、veto / inhibit 入力に何か(CoBo の busy 等)が繋がっているか。LED が CFD 出力かゲート出力か。
2. VM の物理設定で `triggerMode` / `secondaryTriggerMode` / `readoutDepth` / `enableZeroSuppression` と、LEMO I/O の `function`(busy_output の割り当て)。
