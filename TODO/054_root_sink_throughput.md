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

## 攻め手(裁定済み — この 2 つに限定。GDataFrame 廃止は SPEC の柱に触るため**やらない**)

- **A. map insert の hint 化(29% を攻める)**: `AddValByStrip` 呼び出し列が strip/時間で
  ほぼ整列しているなら、TPCReco API の範囲で挿入順・hint を工夫(`chargeMap` の最終内容が
  同一なら手段は任せる。TPCReco 側のコードは**無改変** — 呼び方だけで削る)。
  併せて Filler 側の一時確保・再確保も点検(1 イベント毎の map 再構築コスト)。
- **B. `ROOT::EnableImplicitMT()` の試行(zlib 25% を攻める)**: バスケット圧縮の並列化。
  **受け入れは 021 オラクルの内容一致**(`compared 3852 events, 0 differences`)であって
  バイト一致ではない(§6.4 — 053 結果節の補足どおり)。IMT で内容が変わらないことを
  オラクルで証明できなければ**採用しない**。スレッド数は設定可能に(既定は控えめ)。
- 相乗り可(小): 053 未決④ = soak_harness の RSS 単調性判定に**絶対値フロア**
  (例: 上昇幅 < 32 MiB は OK 扱い)を追加(decoder 13.6→14.5 MB を「上昇」と誤判定した件)。

## 受け入れ

- **実測**: 隔離プローブ(053 の mem_probe 相当 — `prof_053_mem_probe.cxx.txt` に写しあり)で
  ms/event の before/after。soak_harness 30 分 @224 Mbps で **events_built ≥ 100/s +
  全ロスレスカウンタ 0 + RSS 平坦**。届かない場合は**到達値と残りの内訳**を計測で報告
  (それ自体が成果 — 100 Hz が単スレッドの物理限界なら、その事実が (b) 並列化裁定の材料)。
- 挙動: root_sink 全スイート + conformance + **021 オラクル(内容一致)** 無変更 green。
  cargo 無影響(相乗り分除く)。
- ELITPC 換算(2.2 MB/event、524k insert/event)の外挿見積もりを結果節に。

## 非スコープ

- GDataFrame 中間表現の廃止(SPEC §6.4 の等価性担保 — ユーザー/Fable の SPEC 裁定なしに
  触らない)/ chargeMap・圧縮形式の変更(出力形式の定義)/ キュー単位のバイト建て化
  (SPEC 検討 — CURRENT.md 保留節)。
