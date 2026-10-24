# 181 — run 197: 周期 10 ms・全 ch(76 Hz)で root_sink が追いつかず、receiver の内部キューが溢れた

**Status: DRAFT**(2026-10-24 起票 Opus。ユーザー「止められないです」「いいえ、止まってません」「現実に信号は来てます」)

## 0. 何が起きたか(run 197、2026-10-24 17:31–17:41)
- params `elinp__a6cb1018`: 物理(`configure-elinp.xcfg`、12.5 MHz・delay 2500・502 ns・120 fC・**full**)+ `triggerMode periodically`・**`triggerPeriod_ms 10`**(179 の新ノブで初の run)、comment「pedestal」。
- 取得のレート = 44,995 / 591 s ≈ **76 Hz**(= 周期 10 ms + 読み出し 2.9 ms。receiver の `frames` は溢れた分を含む受信数。**当初「110 Hz」と書いたのは溢れを二重に足した誤り** — 182 の計測で 10 ms → 78 Hz を確認、2026-10-24 訂正)。
- **root_sink の書き出しは ≈ 17.6 Hz**(PEventTPC 1 GB 分割 1,936 entries ≈ 110 s ごと、raw ツリー同時、`compression=101 imt=4 workers=1`)。
- 背圧が decoder → receiver に逆流(「blocking on backpressure (lossless)」の繰り返し)→ 17:35:11 に receiver の内部キューが満杯「dropping frames and reporting Error (SPEC §1.4-3)」→ 17:40:49 Error(`overflow_frames=19855`)。
- ユーザーの Stop(17:41:02): `reason abort:component-error:receiver0`、`eos_closed false`、receiver「illegal transition Error -> Configured」→ Reset で Idle。decoder `batches_abandoned 1`、receiver `messages_abandoned 2`(graw-writer への EOS を含む)。
- **失われたフレーム = 19,855**(receiver の分岐の手前なので graw にも無い)。graw = 7,008,629,760 B = 278,784 B × **25,140** = 44,995 − 19,855(完全一致)。
- Stop の後も root_sink は溜まった分を書き続け、UI のステータスバーは STATE running / RUN 197、Monitor に事象が流れ続けた → ユーザーには「止まっていない」と見えた。ECC は Ready、receiver / decoder / graw-writer は Idle、zCoBo との TCP 接続なし。

## 0b. 復旧(2026-10-24 17:58、ユーザー了解「進めてください」)
- Stop の EOS が root_sink に届かず(receiver Error → Reset で abandon)、root_sink は 17:56 に溜まった分を書き終えた後も run 197 を開いたまま(UI の STATE running)。
  この状態で次の run を始めると、そのデータが run 197 のファイルに入る(`RunState::on_data` は開いている run に Continued + mismatch を数えるだけ)。
- `daq_down.sh` → root_sink は SIGTERM で吸い切って inprogress のまま閉じた(`stopped mid-run — kept … NOT finalized`、最終 JSON: fragments 24,042、events_complete 24,042、incomplete 0、late 0、pending 0、publish_drops 13,150)。
- 手で最終名に: `run_inprogress_1792853590.root` → `run0197_0012.root`(831 entries)、`run_raw_inprogress_1792852270.root` → `run0197_raw.root`(24,042 entries)。`mv -n`(既存の上書きなし)。
- `daq_up.sh` → status 200・`/` 200・Idle・8 プロセス + root_sink。ログブックにコメント(author claude)。
- **graw 25,140 vs ROOT 24,042(差 1,098)**: raw ツリーの eventId = 0..43,896、不連続 3 か所(= キューの溢れの塊)。→ **末尾の ≈ 1,098 事象(eventId 43,897 以降)が graw には在り ROOT に無い** =
  Stop / Reset の時点で receiver → decoder → root_sink の途中(ZMQ のバッファ・decoder の入力)にあったもの。数えられたのは receiver `messages_abandoned 2`・decoder `batches_abandoned 1`(fragments 1)だけ →
  **約 1,097 フレームが数えられずに消えた**(派生物の ROOT 側。graw には在るので graw から作り直せる)。

## 1. 論点(未決 — Fable キュー候補)
1. root_sink の持続レート(全 ch・raw ツリーあり)≈ 18 Hz は最低目標 100 Hz(CLAUDE.md)に遠い → **182 で計測: workers 1 = 16.6 Hz、4 = ≈ 66 Hz(比例)、raw ツリーの費用はほぼ 0、重いのは PEventTPC**。150 の最大レート試験は短い run(2,333–7,674 事象)でキューが吸収していただけ。workers > 1・raw ツリーの圧縮・PEventTPC の書き方のどれが効くかを測る。
2. 周期トリガのノブ(179)で誰でも高レートを出せるようになった → UI に「全 ch の持続レートの目安」を出すか、周期の下限を設けるか。
3. Stop の後に root_sink の残りを書いている間、UI がそれと分かる表示(「draining run 197: 残り N」)。今は Running と区別がつかない。
4. ~~周期 10 ms で 110 Hz になった理由~~ → 誤算だった(実際は 76 Hz、上の訂正)。
5. **Stop / Reset のときに途中にあるフレームが数えられずに消える**(上の 1,097)= silent failure 禁止(CLAUDE.md)に反する。どこで消えたか(receiver の送信キュー / ZMQ HWM / decoder の入力)を特定し、数えるか吸い切る。
6. EOS が失われたとき root_sink の run を閉じる正規の手段が無い(今回は daq_down / up + 手で rename)。次の run/start で root_sink が古い run を開いたままなら拒否する、または強制 EOS を root_sink に直接送る経路。

## 2. 当面の運用
- 周期トリガで全 ch を長く取るときは周期 ≥ 60 ms(≲ 16 Hz)にする。ペデスタルは数千事象なら 10 ms でも 1 分以内に止めれば溢れない(キュー 1,000 + 1,000 メッセージ)。
