# 110 — 前の取得を止めずにスタックを落とすと、次の run の頭に古いフレームが混ざり、root-sink がほぼ全イベントを late で落とす

**Status: READY**(2026-10-09 起票 Opus。109 の workers-8 試験で発見。graw は無傷。設計判断あり → Fable キュー)

## 事実(ELITPC、2026-10-09 12:05–12:13)

- きっかけ(Opus のミス): workers-8 試験の列 `~/w8_seq.sh` は前回の列から作ったが、その前の手直しで「α run を run/stop する」行が消えていた。
  → **α run 28 が走ったまま `daq_down.sh` でスタックごと停止**(12:12:29)。ECC(getEccServer)が stop を出さずに SIGTERM で落ちたので、
  **zCoBo は外部トリガでの取得を続けたまま**。
- 直後の run 29(`elinp-per10`、periodic 10 ms、workers 8)の graw: **先頭 19 フレームが run 28 の続き**(eventIdx 2038–2056、CoBo タイムスタンプ
  406.36–409.02 s = run 28 開始からの時刻、frameType 1)。frame #19 で **eventIdx 2056 → 0、時刻 409.016 → 0.016 s** と逆戻りし、以降 2,406 フレームが periodic。
- root-sink: `run 29 closed (batches=2402 events_complete=368 events_incomplete=0 late_fragments=2057)` = **新しい run の大半を late として捨てた**
  (graw は 2,425 フレーム全部ある)。receiver の overflow 0、malformed 0。
- run 28: graw は eventIdx 0–2037 まで(2 本)。ROOT は part 0 が `run0028.root` で確定、part 1 と raw は `run_inprogress_*` のまま(EOS が来ていない)。
  logbook に run_stop なし → 12:2x に controller の comment で経緯を記録(`/api/logbook/comment`)。

## 問題
- 運用ミスに限らず、**ELITPC の停電・クラッシュ**(zCoBo は動いたまま)の後の最初の run で同じことが起きる。graw は無傷でも ROOT(PEventTPC / raw)が大量欠損する。
- root-sink の late 判定は「すでに出したイベントより古い eventIdx」を捨てる設計(eb_core)。run の途中で eventIdx が 0 に戻ることを想定していない。

## 直し方の候補(設計判断 → Fable)
1. run/start の前に ECC を確実に stop / breakup / reset してから describe(controller の walk-back を「前回の状態が不明なとき」にも必ず通す)。
   zCoBo に溜まった古いフレームが新しい接続に流れ込むかは要確認(今回は流れ込んだ)。
2. receiver / decoder で「run 開始直後、eventIdx が 0 に戻る前のフレーム」を前の取得の残りとして別扱い(数えて INFO、graw には残す)。
3. root-sink の event builder が eventIdx の大きな逆戻りを「取得のやり直し」として扱う(late にしない)。
- いずれも silent にしない(CLAUDE.md)。まず合成フィクスチャ(古い eventIdx の塊 → 0 から)で red テストを書く。
