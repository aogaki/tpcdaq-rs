# 133 — 取得の再開の判定: 4 AsAd で古い頭が全部 incomplete のときの見落とし / 多 CoBo での `frames_before_restart`

**Status: READY**(2026-10-13 起票 Opus。SPEC v1.46 確定時に Fable が挙げた実装側の疑義 2 点。**今の mini(1 AsAd、1 フレーム = 1 イベントで即 complete)では起きない** — Warsaw ELITPC(4 AsAd、2 枚の zCoBo)と多 CoBo に効く。設計判断あり → Fable。**2026-10-14 Fable レビューで「端」から「Warsaw 前の必須」に格上げ — §1b**)

## 1. 4 AsAd で再開を見落とす端(root-sink、`tools/root_sink/eb_core.hpp`)
- いまの判定(SPEC v1.46 §6.3)は `event_idx == 0 && (この epoch に complete のイベントがある || emit 済み) && max_seen > 8 && event_time < time_floor`。
- 4 AsAd で、古い頭がどのイベントも incomplete のまま、新しい idx 0 が同じバッチで来ると次のように崩れる。
  - complete も emit 済みも無いので、再開と判定しない。
  - 新しい idx 0 が pending に入り、`time_floor` を下げる。
  - 以後の新 epoch の idx 0 も発火しない。
  - 古い incomplete がタイムアウトで emit されると `emitted_upto` が古い値になり、新しい取得が late で落ちる(原事故の形)。
- どういうときに起きるか:
  - getHwServer の割り込みタスクが AsAd ごとのリングを順に流す作りだと、起こり得る。たとえば AsAd0 の古い頭 → AsAd0 の新しい idx 0 → AsAd1 の古い頭 … の順。
  - Fable 裁定の「境界で千切れたフレーム(malformed)」と重なると、さらに起こりやすい。
- 調べること: 一次資料(`reference/20190315_patched` の `MemRead::flushData` / intrTask)で、複数 AsAd のリングを流す順序を確かめる。
- 直し方の候補(Fable):
  - ① ソース(AsAd)ごとに「その AsAd で idx > 8 を見た後に idx 0 が小さい time で来た」を数え、過半(または全 AsAd)で再開とみなす。
  - ② 古い epoch の incomplete を、新 epoch の idx 0 が来た時点で「別 epoch」として隔離する。
- まず合成テスト(4 AsAd、AsAd 順に流れる古い頭 + 新しい idx 0 が同じバッチ)で red を確かめる。

## 1b. 再開を検出できた場合でも 4 AsAd では原事故が再発しうる(2026-10-14 追記、Fable)
- 送出が AsAd 毎のリング順(AsAd0 の古い頭 → AsAd0 の新 idx 0 → AsAd1 の古い頭 → …)だと、再開検出(epoch +1、高水位リセット)の**後**に AsAd1 の古い idx 2057 などが来る。
- それは新 epoch の pending に入り(`retired` は emit 済み番号だけなので弾かれない)、build timeout で incomplete のまま emit される → `emitted_upto = 2057` → **新しい 1..2056 が全部 late** = 原事故そのもの。
- つまり 110 の実装は 1 AsAd(mini)では完結しているが、4 AsAd では送出順次第で効かない。§1 の「見落とし」と合わせ、**idx ではなく時刻で世代を分ける**(旧 epoch の時刻帯 [floor_old, max_old] に入るフラグメントは旧 epoch に振る、134 の閾値と同じ尺度)のが筋。
- まず一次資料(`MemRead::flushData` / intrTask)で AsAd 毎のリングの送出順を確定し、合成テスト(4 AsAd、AsAd 順の古い頭)で red を作ってから設計。

## 2. decoder の `frames_before_restart` は全ソース合算(`src/decoder.rs`)
- 単一ソースでは root-sink の `events_before_restart` と一致する(実機 run 76: 17 = 17)。
- 多 CoBo(複数 receiver)では、「最初に再開したソースまでに全ソースから来たフレーム数」になり、単位も意味も root-sink とずれる。
- 候補: ソースごとの値にする(`run_stop.counters` に配列 or 最大値)。台帳の読み方が紛れないかで決める(Fable)。

## 参照
SPEC v1.46 §1.3 / §2.3 / §6.3 / §9.2、archive/110(実機 run 72–76)。
