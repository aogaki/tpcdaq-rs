# 054 — root-sink スループット天井の引き上げ(21 ms/event → 100 Hz 目標へ)

**Status: COMPLETED**(2026-08-16 — 結果は末尾の「## 結果」節。起票 2026-08-15 Fable、
発注 2026-08-16 implementer/Opus、発注側一括レビュー PASS 同日)
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
- **§12-6 再実測(031 からの移管)**: burst 672 Mbps × 10 分で保存系 drop 0
  (031 実測は recv_overflow=94,544 で未達 — 原因が本チケットの天井そのもの。
  100 events/s に届いた場合のみ挑戦し、結果を ✔/✘ どちらでも結果節に記録)。
- 挙動: root_sink 全スイート + conformance + **021 オラクル(内容一致)** 無変更 green。
  cargo 無影響(相乗り分除く)。
- ELITPC 換算(2.2 MB/event、524k insert/event)の外挿見積もりを結果節に。

## 非スコープ

- chargeMap・圧縮形式の変更(出力形式の定義)/ キュー単位のバイト建て化
  (SPEC 検討 — CURRENT.md 保留節)/ Recorder の複数スレッド化(A/B/C で 100 Hz に
  届かない場合の次の裁定材料として実測を残すこと)。

---

## 結果(2026-08-16 — implementer/Opus 実装、発注側(Fable)一括レビュー PASS)

### 変更の骨子

- **C. GDataFrame 全撤去 — 完了**: `Filler::add_frame(GDataFrame&)` →
  `add_fragment(const OwnedFragment&)`(items を (aget,raw_ch) 固定長マスへ配ってから
  本家と同一のループ順で走査。ビット抽出は旧 build_frame から逐語移設 = 一致を diff で確認)。
  `OutputFormat` / `build_frame` / `--format` / GDataFrame 遅延追記経路 / GET 辞書を撤去、
  **third_party/get/ をディレクトリごと削除**(053 が直した TRefArray リークのクラス自体が
  消滅)。カウンタはホットパス素の整数 → Recorder が atomic へミラー → Publisher が読む
  (設計妥当と裁定)。11 files, +549/−1,276。
- **A. map insert hint 化 — 実測して棄却(発注側承認)**: key 昇順走査は libc++ で
  **悪化**(Filler 6.25→15.67 ms/event。原因は tuple 比較の全要素評価 — micro ベンチで確定、
  pevent_fill.hpp のコメントに記録)。真の `emplace_hint` は −0.76 ms(全体の 4%)のみで、
  かつ `PEventTPC::myChargeMap` は private・const 参照のみ = **TPCReco 無改変では口が無い**。
  const_cast による迂回は棄却(チケット制約「TPCReco 無改変」+ KISS)。**現状の走査順が最善**。
- **B. ImplicitMT — 採用(既定 `--root-imt 4`、発注側承認)**: 隔離 −6.3%(4 本で頭打ち —
  splitlevel 2 では payload が chargeMap 1 ブランチに集まるため 25% 想定は不成立)、
  live A/B +2.5%。**021 オラクルは IMT 既定 ON のまま green** = 内容一致で受け入れ成立。
  `--root-imt 0` で無効化可。
- 相乗り: soak_harness RSS 判定に 32 MiB 絶対値フロア + 単体テスト 3 本(053 未決④ 解消)。

### ゲート(発注側で追試済み 2026-08-16)

- `cargo test` **450 passed / 0 failed / 1 ignored**(基準 448 +3 soak −1 E2E-A 退役)。
  clippy -D warnings / fmt クリーン。
- C++: tpc_wire 68 / rs_core 71 / eb_core 175 / geo 426 / monitor_hist 202 /
  monitor_pub 92 / **test_recorder 101 / test_pevent 99**(撤去前実測 233/104 —
  差分は下記撤去一覧)。conformance 49。
- env 付き: root_sink_intake 11 / monitor_pub 6+1 ignored / p2_e2e E2E-B
  `compared 108 events, 0 differences`。
- **021 オラクル(無変更)**: `compared 3852 events, 0 differences`(1697.7 s、IMT 既定 ON)。

### 実測

| 指標 | before | after | 差 |
|---|---|---|---|
| 隔離 ms/event(mini) | 20.333 | **16.057**(C −15.7% + B −6.3%) | **−21.0%** |
| 隔離 events/s | 49.2 | **62.3** | +27% |
| 実稼働 events/s(224 Mbps、1 GiB part 間隔) | 32(053) | **41.5** | **+29%** |
| ELITPC(4 AsAd、実ジオメトリで実測) | — | 隔離 15.5 /s → 実稼働換算 **≈10 /s** | 100 Hz に 10× 不足 |

内訳(C 後): Filler 6.25 ms(map insert 支配)/ tree Fill 以降 ≈10.3 ms。

- **soak 30 分 @224 Mbps は未達(41.5 events/s で 2.4× 不足)**。ロスレスカウンタは全 0
  (データは落ちていない)が、stop 時に在庫 ≈8,000 イベントの吐き出しが EOS 予算 5 s を
  超え `error:eos-timeout` — **CURRENT.md 保留②の構造問題そのもの**(054 範囲外、逸脱受理)。
- 代替の **80 Mbps × 30 分は完走合格**(10/10 run・全カウンタ 0・RSS 8 プロセス OK、
  root_sink 後半 +11.2 MiB = 絶対値フロア内)。
- **§12-6(672 Mbps burst)は未実施**(発注書の「100 /s 到達時のみ」に該当せず)—
  **次の並列化チケット(055)へ移管**。

### チェックリスト①〜⑥(build_frame にだけ載っていた意味論の移送)

①先勝ち = マス集約により状況自体が不発(旧実装でも同様と確認)/ ②(aget,chan) 昇順は
本家と同一ループ順・bucket 昇順は unsorted フラグ + stable_sort / ③chan≥68 counted drop は
Filler へ移設(アクセサ据え置き、Rust 側 assert 5 箇所生存)/ ④件数 0 cell = 0.0
(compute_pedestals 無改変 + 手計算オラクル)/ ⑤同一 eventId 1 回(常時有効化 +
test_pevent)/ ⑥signal 窓は充填時(test_default_signal_window_bounds)。

### 撤去テスト一覧(v1.17 削除条件 = 021 オラクル成立で退役)

compare_gdataframe.cxx(012 の GDataFrame 全値比較)/ test_recorder の GFrameHeader
読み戻し・「遅延分も 1 エントリ」(v1.8 で意味論が既に変更済み)/ test_pevent の
`--format` ツリー名切替 / p2_e2e **E2E-A**(§12-3 旧オラクル — v1.17 で SPEC 側も撤去済み)。
E2E-B の決定性照合は compare_pevent + `--run-id` 固定へ移行(108 events, 0 differences)。

### 残課題(→ 055 起票)

**100 Hz は単スレッド Recorder では届かない**(mini 41.5 /s、ELITPC ≈10 /s)。
並列化の設計裁定 + §12-6 再実測 + 保留②(過負荷 stop の EOS 予算)を 055 に集約。

- 実行環境: macOS Darwin 25.5.0 / 14 CPU / ROOT 6.36.10 / 実 mini・ELITPC ジオメトリ。
- 証拠: `reference/_spike/soak_evidence_054/`(プローブ・micro ベンチ・オラクルログ・
  soak レポート 2 種・IMT A/B ログ)。

**Status: COMPLETED**
