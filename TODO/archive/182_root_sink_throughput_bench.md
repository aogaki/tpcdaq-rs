# 182 — root_sink の持続レートを実機で測る(181 の論点 1、2 年以内の 100 Hz 実験に向けて)

**Status: COMPLETED**(2026-10-24。起票 Opus。ユーザー「始めてください。11 月も別に本番とまでは言いません。レートも未計算です。ただ、2 年以内に 100 Hz の実験をやる気です」)

## 0. 目的
- run 197(全 ch・12.5 MHz・≈ 110 Hz)で root_sink が ≈ 18 Hz しか出なかった。064 の「mini 152 events/s(workers 4)」は raw ツリー(v1.28)以前・Mac の数字。
  **今の ELITPC で、どの設定なら何 Hz 出るか**を測り、181 の設計判断(graw を ROOT から切り離す / root_sink を速くする)の材料にする。

## 1. 方法(実機の zCoBo、検出器の信号は不要)
- fake_ecc は ELITPC に無い(Ice C++ 無し)ので soak_harness は使えない → **実機の周期トリガ**で入力を作る。
- 入力: params = 物理(`configure-elinp.xcfg`、12.5 MHz・delay 2500・502 ns・120 fC・full、= `elinp__6cf41a72`)+ `triggerMode periodically`・`triggerPeriod_ms 10`(≈ 110 Hz、run 197 と同じ)。
- 1 本 = **60 s**(≈ 6,600 事象 — 197 で溢れたのは 4 分後なので溢れない)→ stop → root_sink が run を閉じる(`run N closed`)までの時刻を 1 s ごとに見る。
  **root_sink の速さ ≈ 事象数 / (閉じた時刻 − start)**(入力 110 Hz より遅ければ、閉じるまでの時間が速さで決まる)。60 s 時点で root_sink の CPU をスレッド別に採る(`top -H`)。
- 変える設定(daq.env の `ROOT_SINK_RECORDER_WORKERS`、daq.toml の `raw_tree`、daq_down / up で切り替え):
  - V1 workers 1 + raw ツリー(**今の本番**)
  - V2 workers 1、raw ツリーなし
  - V3 workers 4 + raw ツリー
  - V4 workers 4、raw ツリーなし
- run 番号は通常どおり進む(comment `bench182 Vn`)。データは /data/tpcdaq に残す(消すかはユーザー)。
- 最後に必ず元へ戻す(daq.env / daq.toml / params を退避から戻し daq_down / up)— スクリプトの `trap EXIT`。スクリプトは ELITPC で `setsid nohup`(VPN が切れても完走)。

## 結果
### 途中の観察(V1、run 198、60 s 時点の `top -H`)
- root_sink のスレッド 1 本が **91 % CPU**(1 コア飽和、55 s 中 44.5 s)、他は raw 書き 5.7 s・集計 2.0 s・IMT 2 本 5–6 s。
  スレッドの生成順(`root_sink.cxx` 1284 record → 1316 raw → 1342 publish → 1345 aggregate)から、飽和しているのは **PEventTPC の record スレッド**
  (`pevent_fill.hpp` = TPCReco の `fillEventFromFrame` の移植: 全 ch × 全セルの減算・strip 射影・chargeMap への加算 + PEventTPC の直列化)。raw ツリーの書き出しではない。
- perf / gdb attach は ELITPC の一般ユーザーでは不可(`perf_event_paranoid 4`、`ptrace_scope 1`)— 関数単位の内訳は sudo か Mac での再現が要る。
- root_sink には `--no-root`(PEventTPC を書かない、raw ツリーは独立)がある。daq_up.sh は渡せないので V1–V4 の後に別途。
### V1–V4(2026-10-24 18:08–18:21、ELITPC = i9-10900X 10 コア / 20 スレッド、governor powersave(intel_pstate)、スクリプト `~/test/bench182/`)
| 設定 | run | 事象 | 溢れ | 最後の書き込み(start から) | root_sink の速さ |
|---|---|---|---|---|---|
| V1 workers 1 + raw ツリー(本番) | 198 | 4,665 | 0 | 280 s | **16.6 Hz** |
| V2 workers 1、raw なし | 199 | 4,666 | 0 | 276 s | **16.9 Hz** |
| V3 workers 4 + raw ツリー | 200 | 4,667 | 0 | 82 s(stop の 10 s 後) | **≥ 57 Hz**(入力に律速) |
| V4 workers 4、raw なし | 201 | 4,667 | 0 | 82 s | **≥ 57 Hz** |
- 入力 = 周期 10 ms + 読み出し 2.9 ms ≈ **78 Hz**(4,666 事象 / 取得 60 s。表の acq_s 71 s は run/start の configure ≈ 7 s と stop を含む)。
- 全 run が正常に閉じた(`eos_closed true`、`run N closed … events_incomplete=0 late_fragments=0`)。
- `top -H`(60 s 時点): V1 / V2 は 1 スレッド 91–100 %、V3 / V4 は **4 スレッドとも 99.9 %**(各 52.6 s)= 書き手は 4 本でも飽和。
  → **1 本 ≈ 16.6 events/s(≈ 60 ms CPU / 事象)、本数にほぼ比例**: 4 本 ≈ 66 Hz(stop 時の残り ≈ 700 を 10 s で吐いた = 66 Hz と整合)。100 Hz には 6–7 本。
- **raw ツリーの費用はほぼ無い**(V1 vs V2: 16.6 vs 16.9、V3 vs V4 同じ)。重いのは PEventTPC(TPCReco 形式)の充填 + 直列化。
  ファイルも PEventTPC が 555 kB/事象(graw 279 kB の 2 倍、raw ツリー 120 kB の 4.6 倍)。
- 064 の Mac(16 ms/事象)と比べ ELITPC は ≈ 60 ms/事象 — CPU の差(と powersave)だけで説明できるかは未確認。
- 出力(消していない): run 198–201 = 各 2.7–3.2 GB(/data/tpcdaq)。
### 結論(181 の設計判断の材料)
1. 今の本番(workers 1)の上限は **≈ 17 Hz**(全 ch)。CLAUDE.md の最低目標 100 Hz の 1/6。
2. workers を増やせば比例して伸びる(4 本で ≈ 66 Hz)。ただし PEventTPC のファイル名が `runN_wK.root` になる。
3. PEventTPC を online で書かない(`--no-root`、raw ツリー + graw だけ。PEventTPC は必要なら grawToEventTPC で後から)なら record スレッドの負荷が消える — 未計測(daq_up.sh が渡せない)。
4. 速さと別に、**graw が ROOT の遅さの巻き添えになる構造(receiver の送信スレッド共用)は直す必要がある** — 181。

### V5–V7: PEventTPC を書かない(183 の `pevent = false` = root_sink `--no-root --raw-tree`、2026-10-24 19:24–19:28、`~/test/bench183/`)
| 設定 | 周期 | run | 事象 | 実入力 | 溢れ | 最後の書き込み | root_sink |
|---|---|---|---|---|---|---|---|
| V5 | 10 ms | 202 | 4,667 | ≈ 78 Hz(60 s) | 0 | stop と同時 | **入力に完全追従** |
| V6 | 3 ms | 203 | 10,159 | ≈ 169 Hz(60 s) | 0 | stop と同時 | **入力に完全追従** |
| V7 | 1 ms | 204 | 7,882 | ≈ 263 Hz(30 s、FW の読み出し 2.94 ms の上限近く) | 0 | stop の ≈ 6 s 後 | ≈ 220 Hz 前後で頭打ち |
- 全 run `eos_closed true`、`run N closed … incomplete 0 late 0`、`run{N}_raw.root` と **`run{N}_monitor.root`(raw 側で書いた)** がある、`run{N}.root`(PEventTPC)は無い。
- `top -H`: raw の書き手 1 本が律速 — V5 22 s / 55 s(≈ 5 ms CPU / 事象)、V6 42 s(≈ 4.5 ms / 事象、80 %)、V7 91 %。**raw ツリー 1 本の上限 ≈ 200–220 Hz**(全 ch・mini)。
- **結論: PEventTPC を書かないと、全 ch・12.5 MHz で 100 Hz を 2 倍の余裕で通る**(PEventTPC あり workers 1 の 16.6 Hz の 10 倍以上)。次の上限は raw ツリーの単一書き手(≈ 200 Hz)と FW の読み出し(≈ 340 Hz)。
  ELITPC(4 AsAd)は事象あたり 4 倍なので raw 1 本で ≈ 50 Hz 見込み(ELITPC 100 Hz は後回しの裁定のまま)。
- 構造の問題(graw が ROOT の遅さの巻き添え、Reset の数え漏れ、EOS 喪失)は 181(Fable)に残る。
