# 054 — root-sink スループット天井の引き上げ(21 ms/event → 100 Hz 目標へ)

**Status: READY**(起票 2026-08-15 Fable — 053 の計測で原因確定済み。**一晩 soak(45 Mbps)
とは独立に進められる**が、soak 走行中は root_sink バイナリを上書きしない運用に注意
(soak はバイナリのスナップショット複製で走らせてある))
**仕様**: SPEC §12-5/12-6(v1.15/16)の残受け入れ / **§6.4(PEventTPC 内容一致が正 —
run.root のバイト一致は要求ではない。021 オラクル「全イベント全 key 一致」が受け入れ**)
**証拠・計測**: `reference/_spike/soak_evidence_031/prof_053_*`(sample 内訳・heap)
**発注先想定**: implementer/**Opus**(性能工学。ただし攻め手は下記に限定)

## 事実(053 計測 — 再掲)

Recorder 単スレッド 21 ms/event(隔離 47 /s、実稼働 32 /s)。内訳:
`tree_->Fill()` 45%(うち zlib deflate 25%)/ `PEventTPC::AddValByStrip` = std::map insert
29%(131,072 insert/event)/ GDataFrame 中間表現 15%。目標 = **≥100 events/s(mini)**。

## 攻め手(v2 改訂 2026-08-15 — **ユーザー裁定「GDataFrame は graw2root のもので全く不要」
→ SPEC v1.17 適用済み**。攻め手は 3 つ = 計 59% を狙う)

- **C. GDataFrame の全撤去(15% + 保守負債)** — **最初にやる**(A/B の計測対象が変わるため):
  ①中間表現の撤去: `build_frame`(Fragment → GDataFrame)を廃し、**Filler が
  OwnedFragment を直接読んで PEventTPC を充填**(ch→strip 変換は既存 geometry 経路)。
  等価性の担保は §12-3 の内容一致オラクル(SPEC v1.17 §6.4)。
  ②`--format gdataframe` テスト専用モードと専用回帰(test_recorder の GDataFrame 読み戻し
  群 = 011/012 由来)を撤去 — **撤去したテストの一覧と理由(v1.17 削除条件成立)を結果節に
  明記**(「テストを消す」のではなく「役目を終えたオラクルを SPEC の予定どおり退役させる」)。
  ③不要化した `third_party/get/` の GDataFrame 系クラスを削除(CeCILL 隔離ごと縮小。
  053 が直した TRefArray リークのクラス自体が消えることも記録)。
- **A. map insert の hint 化(29%)**: `AddValByStrip` 呼び出し列の整列性を活かして
  挿入 hint / 順序を工夫(TPCReco 側コード**無改変**、`chargeMap` の最終内容同一)。
  C 後の Filler 直読設計に合わせて実施。
- **B. `ROOT::EnableImplicitMT()` の試行(25%)**: バスケット圧縮の並列化。
  **受け入れは 021 オラクルの内容一致**。証明できなければ採用しない。スレッド数設定可。
- 相乗り可(小): 053 未決④ = soak_harness の RSS 単調性判定に**絶対値フロア**
  (例: 上昇幅 < 32 MiB は OK 扱い)。

## C の実装指針 — 意味論の正本(2026-08-15 調査で確定。docs 調査 3 レーンの結論)

GDataFrame は TPCReco にとっても**入力アダプタ層**であり(WITH_GET ガード内のみ・ROOT に
1 バイトも入らない・解析/GUI は不知)、撤去後の Filler 直読が鏡写しにすべき**正本は次の
2 か所だけ**(`reference/TPCReco/latest/`):
1. **`EventSources/src/EventSourceGRAW.cpp:301-323`** — ループ順(aget 外・normal chan 内)/
   normal→raw リオーダ(`Aget_normal2raw` = GeometryTPC.cpp:1321-1331)/ signal 窓 /
   減算 / strip 射影 / chargeMap への **`+=` 加算**。
2. **`GrawToROOT/src/PedestalCalculatorGRAW.cpp:127-205` + `DataFormats/src/
   PedestalCalculator.cpp:255-262`** — (cobo,asad) フレーム毎リセット / FPN 平均 2 本
   (窓別)/ チャンネルオフセット / `correction = offset + FPN_ave_signal[cell]`。

**移送チェックリスト(build_frame にだけ載っている意味論 — 落とすと壊れる)**:
①(aget,chan) 重複時**先勝ち**(SearchChannel 由来 — pevent_fill.hpp:234)
②(aget,chan) 昇順 + チャンネル内 bucket 昇順(root_recorder.hpp:597-615)
③`chan >= 68` の item を落として数える(`items_out_of_range_` — 我々独自の防御、
build_frame と一緒に消さない)
④件数 0 の cell のペデスタルは 0.0 のまま ⑤同一 eventId は 1 回だけ書く
⑥signal 窓は PEventTPC 充填時適用(EventSourceBase の filterTimeCells とは別物 — 混同注意)。

## 受け入れ

- **実測**: 隔離プローブ(053 の mem_probe 相当 — `prof_053_mem_probe.cxx.txt` に写しあり)で
  ms/event の before/after。soak_harness 30 分 @224 Mbps で **events_built ≥ 100/s +
  全ロスレスカウンタ 0 + RSS 平坦**。届かない場合は**到達値と残りの内訳**を計測で報告
  (それ自体が成果 — 100 Hz が単スレッドの物理限界なら、その事実が (b) 並列化裁定の材料)。
- 挙動: root_sink 全スイート + conformance + **021 オラクル(内容一致)** 無変更 green。
  cargo 無影響(相乗り分除く)。
- ELITPC 換算(2.2 MB/event、524k insert/event)の外挿見積もりを結果節に。

## 非スコープ

- chargeMap・圧縮形式の変更(出力形式の定義)/ キュー単位のバイト建て化
  (SPEC 検討 — CURRENT.md 保留節)/ Recorder の複数スレッド化(A/B/C で 100 Hz に
  届かない場合の次の裁定材料として実測を残すこと)。
