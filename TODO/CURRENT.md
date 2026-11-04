# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-10-13(棚卸し — 止まっていた 069 / 073 / 074 / 078 / 079 / 080 / 084 / 101 / 107 を処置して archive、~~092~~ を修正して archive([archive/092](archive/092_receiver_abandoned_after_eof.md) — EOS の後も Heartbeat を出し続けていた)、131 を起票。続けて ~~124~~(全 ch の ON/OFF をユーザーが実機確認)・~~132~~(run/start 応答に params_id / notes)・~~110~~(未停止で落とした後の古いフレーム = 取得の再開 epoch、ELITPC で再現・確認)を完了 → archive)。**
2026-08-18〜10-13 の作業ログ(Wave 1〜3、ELITPC 移行、125–129)は [archive/CURRENT_2026-10-13_wave_log.md](archive/CURRENT_2026-10-13_wave_log.md)、
その前は [archive/CURRENT_2026-08-14_p2_p3wave.md](archive/CURRENT_2026-08-14_p2_p3wave.md)。

## いま(1 分で読める要約)

- **期限 2026-11-09**: 隣の中性子検出実験の横に TPC を置く(ユーザー「山ほどイベントが来る」)→ 高レート対策 184–188(181、SPEC v1.53)を仕上げて ELITPC に反映・実機受け入れ(run 197 の再現)まで。184 / 185 は 2026-10-30 に implementer を再開。**2026-11-03: 184 コミット・185 / 186 マージ・push(ユーザー了承)→ 14:3x ELITPC へ反映済み(`663b00d`、bundle → ff → cargo build --release 16 s → root_sink make + make test 全 0 failed → daq_down / up、/api/status・/ = 200、7 プロセス + root_sink、新しいログに WARN / ERROR なし、receiver の状態に `decoder_link_dropped_*`)。**実機の受け入れ(14:52–14:55): run 211(10 ms、4,652)・run 212(1 ms、15,267 ≈ 250 Hz)とも受信 = graw = decoder で一致、落とし 0、root_complete true(archive/184 末尾)。+ pevent = true の run 213(60 s)・**run 214(300 s): graw 23,252 = 受信(喪失 0)、decoder リンクで 6,251 を数えて落とした、run は止まらない = SPEC v1.53 の核を実機で確認**。停止が error:eos-timeout になる件は 204(Fable キュー)。run 214 の未完 ROOT は削除・ログブック comment 済み。** 残り = 188・187。**週末(10-30〜11-02)は PuBe の run 209 系(173 §4)— 反映はその後、ユーザーの OK で。 **2026-11-04: 204(停止は root_sink の close まで待つ)・187(UI)・188(watchdog)を main にマージ・ELITPC に反映(`81cbe9f`)・実機受け入れ合格(run 217 = run 197 の再現で ok / normal、218 = graw_writer SIGSTOP で自動異常停止、219 = 対照)→ 181 の高レート対策 184–188 は 187 の目視を残して完了。** 残り: 209(ディスク詰まり中の守り — ユーザー判断)、206 / 208 / 202(11/9 の後)。
- **本番**: ELITPC(旧 puppex、Ubuntu 26.04)で mini eTPC を運用中(2026-10-08〜、[archive/107](archive/107_elitpc_host_setup.md))。
  - 機器: 実 zCoBo + 実 ECC(getEccServer ネイティブ)+ Rust ecc_bridge。
  - 保存先: データは ZFS `data/tpcdaq`。
  - 記録: ELOG(TPC 用)、InfluxDB(HV / LV)。
  - マニュアル: 日英 PDF が `reference/_manual/`。
- **実装の正本 = docs/SPEC_ja.md v1.54**(2026-11-04、停止は root-sink の close まで待つ + ロスレスの守り = 204。v1.53 = 2026-10-24、保存系 = graw-writer のみ = 181、実装は 184–188 で進行中。v1.52 = PEventTPC をオプションに = 183。v1.51 = params に `triggerPeriod_ms` = 179。v1.50 = 2026-10-23 status カウンタを run ごとに = 175。v1.49 = 2026-10-16 ZS 対応)。ゲート(main `22e9284`)は cargo 1078 passed / 0 failed / 1 ignored、UI 393 passed / 10 skipped、root_sink の C++ テスト全 passed(make test / test-root)。適合 402(127 時点)。ELITPC は main `2911209` で稼働(2026-10-24 19:24 反映 = 183、Rust + root_sink、**daq.toml `pevent = false`**。その前 = 17:10 179 の Rust + UI。その前 = 2026-10-23 11:51 175 の root_sink 再ビルド、RUNBOOK §7.1。Rust は 04c9ad1 以降変更なし、UI は 172 を 10-22 に反映)。
- **できること**:
  - 保存二系統: graw はロスレス、root-sink は PEventTPC と raw ツリー。
  - 取得の再開: 未停止で落とした後の古いフレームを epoch として扱う(110)。
  - run/start の歩き戻し: ECC とコンポーネントの両方。前の run の run_stop も書く(113–116)。
  - Web UI:
    - Monitor
    - Event Display: XY / XZ / YZ。macros の移植で再構成し、monitor の `src/reco/` → 0x04 Track で配る。
    - Waveform
    - Parameters: JSON が正。テンプレートから抽出、Apply & restart。
    - Slow control: HV / LV の読み出しと ch ごとの ON/OFF。
    - Logbook
  - `GET /api/last-event`(123)。
- **実測**(069 処置表、109):
  - zCoBo の読み出しは 2.931 ms / イベント。
  - 1 GbE では 182 Hz で頭打ち(101.7 MB/s、全数・連番)。
  - root_sink は workers 8 で ≈ 122 events/s。
- **取得条件**: mini の現用は **25 MHz / triggerDelay 1750**(params `elinp__e090134b`、2026-10-14 の run 83 から。10-11 の run 57〜82 は 1500 = `elinp__dcdaf9ca`。それ以前の α run は 100 MHz で、時間窓 ≈ 24 mm)。

## 今日の進行(2026-10-14 午後 — compact 後はここから再開)

**背景**: ユーザー「ソースと検出器が自由に使える今のうちにガンガン」。13:45 時点の合意プラン。ユーザーは長いミーティング → 16:20 に帰宅 → VPN から続ける。
**ELITPC**: `daq@192.168.144.220`。現用 params = `base_1750.json`(`elinp__e090134b`: 25 MHz / triggerDelay 1750 / 232 ns / 120 fC / partial)。作業場 `~/test/scan/`(scan.py、run_metrics.C、metrics_for_run.sh、pulser_kernel.sh、ログ・JSON)。指標は `~/test/scan/metrics/runNNNN_metrics.json`、macros = `~/test/ana/ana_107`、`--pressure 91`。
**済み**: triggerDelay スキャン(run 78–82、archive/135/136)、peaking スキャン(run 83–86、archive/137)。gain スキャン(run 87–90: 502×120fC / 502×240fC / 502×1pC / 1014×1pC、`scan_gain.log`)は 13:52 頃完了予定。
**自動で走るもの**: `pulser_kernel.sh`(138、gain スキャンの終了を待って自動開始 → 8 run、各 30 s、`pulser_kernel_runs.json` と `pulser_kernel.log`、終了後 base 復元)。
**走っている implementer**: 138 の解析マクロ `tools/scan/pulser_kernel.C`(Opus、worktree)。戻ったら照合 → マージ → ELITPC で 8 run に適用 → α の「揺れ」(137: 信号 ch のみ・FPN に無し・電荷と同じ比で peaking に増える)と比べて出どころを決める → **139 デコンボリューション試作の発注書**。

| 順 | 時刻 | やること | 状態 |
|---|---|---|---|
| 1 | 〜13:52 | gain スキャン完了 → 4 点の表(揺れの列つき)を 137 の形で記録 | **済 → [140](140_gain_scan.md)**(run 87–90。E 候補 = 502 × 240 fC、1014 × 240 fC を 1 run 追加中 = run 101) |
| 2 | 〜14:05 | 138 パルサー核 8 run(自動) | **済 → archive/138**(run 103–110、18:31–18:38、template `configure-elinp-pulser.xcfg`)。核は無振動(1.8 µs 成分 ≤ 0.03 %)→ **α の揺れは電子回路由来ではない**。139 起票 |
| 3 | 14:05 | **D 較正表**: pedestal(`triggerMode: periodically`、`readout: full`、150 s ≈ 150 events — テンプレートの triggerPeriod 1 s は非管理ノブ)を 232 / 502 ns = run 99 / 100、pulser は 138 の取り直し run を流用 → `calib pedestal|scan` で `~/test/scan/calib/*.json` | run 99 済(signal σ 中央値 7.6 ADC、FPN 3.3)、run 100 取得中 |
| 4 | ~~14:15~~ | ~~**B 自己トリガ**(onMultiplicity)~~ | **中止**: zCoBo FW はマルチプリシティトリガを扱えない(archive/078 §: 「The current firmware version does not handle triggering on MULTIPLICITY」、ユーザー裁定 2026-09-14 で API からも除外)。私の計画ミス |
| 5 | 14:50 | まとめて報告。**E の設定を決める規則**: 飽和 ch を含むイベント < 2 % の中で最長 peaking とその gain(1 pC で 1014 が収まればそれ、無理なら 502) | 未 |
| 6 | ~~15:00~~ **18:37 開始(run 111、`elinp__f591e65b`)** | **A 12.5 MHz ドリフト端 run**: knobs `writingClockFrequency_MHz: 12.5`、`triggerDelay: 3800`(ルール: (窓 40,960 − 2,800 − 160)/10)、ほか現用。**帰宅後まで走らせたまま**(2–3 h)。16:20 以降も走っているのはこれだけ | 未 |
| 7 | 〜20:40 | **A 途中確認(19:38、16.5k events)**: last_hit が窓末尾 495–508 に 24 % 積み上がり(p90 = 508)→ 端が cell ≈ 500(v ≈ 0.52 cm/µs)か切れているか決められない → 20:40 に A stop → gain2(1014 × 240 fC)→ **A2 = delay 4000 を 1 h**(自動、`start_A2.sh`)→ 落ち際が見えるかで判定 → 22:00 頃 E。**20:40 A 停止(run 111、33.9k events)→ run 112(1014 × 240 fC、飽和 2.1 % → 140 完了・archive)→ 20:47 A2 = run 113(delay 4000)開始。** run 111 の last_hit: cell 508 単独のスパイク(19 % = 窓端の跳ね → drift_end の上限を 508 に下げる発注、Sonnet)、**cell 330–345 に山(ピーク 340–344、基線の 5 倍)= ドリフト端の候補: t_max ≈ 318 cell × 80 ns = 25.4 µs → v ≈ 0.79 cm/µs(公称 0.48 より速い — A2 で山が 25 cell 動けば実体、動かなければアーティファクト)。**21:51 A2 停止(run 113、17.6k events)→ 判定: α 線源ではドリフト端は測れない。** イベントごとのヒット分布(16 cell = 1.28 µs 刻み)は run 111 / 113 とも**トリガ帯から 2–4 µs の範囲に全部収まる**(α の飛跡は z 方向に数 cm しか無く、線源は読み出し面の近く)。run 111 の「cell 330–345 の山」と窓末尾の積み上がりは run 113(窓を 25 cell ずらした)で同じ位置に現れず、イベント当たり 4 倍少ない → 物理のドリフト端ではない(第 2 クラスタ = pile-up か、窓末尾に相関する何か。未解明、急がない)。**v_drift = L / t_max は 069 C-4 の言うとおり宇宙線(全ドリフト長を跨ぐ飛跡)か、線源を陰極側に置く run が要る** → 131(現地)に積む。副産物: `drift_end.C`(ヒット定義 [2,508) + run_metrics の ch 条件)は 25 MHz でも使える道具。A/A2 の run 111 / 113 の指標は `metrics/`(macros は 25 MHz 前提なので z は 2 倍ずれる)。
**21:55 E 開始 = run 114**(`elinp__b82dddd2`: 502 ns × 240 fC、25 MHz、delay 1750、external)。朝まで走らせる。停止は `~/test/scan/stop_run.sh`(base 復元まで)。A2 を stop → ドリフト端(t_max → v_drift = L / t_max、σ_L(z))の解析(小マクロ) → **E 一晩 α run**(25 MHz、5 の設定)を開始(開始前にユーザーに一言) | 未 |
| 8 | 10-15 | **C 済 → [archive/141](archive/141_readout_threshold_scan.md)**(run 115–121): 現用 partial は `Reading=always` で全 ch を読み、full の **2 倍**のデータ(557 vs 279 kB/event)、閾値は無効。ZS 600 は端まで通り 14.8 kB/event。次 = base を full に(了解後)、`Reading=only_if_hit` テンプレートで再スキャン、ch ごとの ZS 閾値(D の表)。**07:55 E 停止(run 114、10 h、19.6 GB raw、指標を計算中)**。**13:12 ユーザーが現地を離れる前に HV 全 ch と AsAd LV を OFF(UI の 124 経路、audit に記録)。** 以後 ELITPC で run は取れない(ECC prepare が「AsAd が電源から切り離されている」で失敗 = 正常な拒否)。**156 済 → archive(SCANS S18): ZS は品質を落とさず 1/9(B)〜1/12(hit g4)、FPN 残しの利得なし → 抑制、ビーム既定候補 = hit g4 + ZS。root_sink / monitor の ZS カウンタは run ごとに見えない → 159 で close 行 / status へ(157 と同時)。** **11:00 ZS 一式を ELITPC に反映(ユーザー承認: daq.toml `[system] ped_table`、root_sink make、daq_down/up)— root_sink / monitor とも表を読み込み。11:01–11:24 検証 run 170(B+ZS)/ 171(B+ZS FPN 残し)/ 172(hit+ZS)/ 173(hit+ZS FPN 残し)、frameType 2 = 0、compact_mismatch = 0。B に復元。11:26 週末 run 1 = run 174(B、gas 100 ml/min、流量を変えたら新 run — ユーザー裁定)。** **09:30 ZS 対応の計画をユーザー承認(plan ファイル `~/.claude/plans/lexical-plotting-fox.md`、範囲 = 全部、FPN は 156 で判定)→ 151–158 を起票。第 1 波 = 151 / 154 / 155 / 158 を並列発注。** **08:25–09:05 150 最大レート 2 → archive(SCANS S17)**: 上限 = FW 読み出し 2.94 ms(272 ch、≈ 0.11 ms + 10 µs/ch)、網は full なら余裕、読み出し中のトリガは失われる(不感 = レート × 2.94 ms)、hit partial だけが読み出し時間を縮める、ZS はバイトのみ(1/4.3〜1/8.8)、partial + ZS 両立、光/10 GbE は zCoBo リンクに効かない(FW の 1 GbE MAC)。ビーム 100 Hz の設計 = hit partial + ZS。DAQ は Idle、params B、workers 1。 **07:50–07:58 133 を ELITPC に反映(ユーザー了解)**: root_sink / decoder / controller 新版 + UI(Mac ビルド)。再開の再現 run 146: `restarts=1 events_before_restart=10 late_fragments=0 restart_duplicates=0`、`run_stop.counters.frames_before_restart = {"0": 10}`(辞書)、run 147 通常 `0 / {}`。1 回目は PATH の罠で Rust が旧版のまま(RUNBOOK §7.1 注意に記録)。DAQ は Idle、params B。 **06:33 run 141 停止(ユーザー「止めてください」)→ 集計(SCANS S16): 149,795 events / 9.6 h / 104 GB、ヒット ch 121、飽和 0.92 %、ok 0.999、σ_z 0.20、σ_2D 0.321 = run 114(A 一晩)・run 130(B 確認)と一致 → B 確定。** **06:08 run 141(B 一晩)健康確認: 22:07 / 00:07 / 02:07 / 04:07 / 06:07 とも Running、HV ステータス正常(ch0 は既知の UNV)、root_sink 警告 0、/data 23 TB 空き。** **20:19–20:55 141 続き(hit テンプレート、ユーザー承認)= run 135–140、SCANS S15: hit register は効くが frameType 1 なので full 比 −14 % 止まり → 運用は full のまま。一晩 run は run 141 として 20:55 に再開。** **133 済 → archive(20:50、SPEC v1.48)**: ソースごとの世代 + 鍵 (run, 世代, idx) + 追従 + 付け替え(中点)+ decoder ソース別辞書。eb_core 391 / cargo 1081、1 ソースのオラクル不変。**実機反映(bundle → build → daq_down/up)と再開の再現は明日ユーザー了解後**。 **20:04 運用点 B の一晩 run = run 134 開始(`start_B.sh`、HV 240 × 3 / full / 502 × 120 fC、2 時間ごとに健康確認)。133 の設計を一次資料(MemRead::flushData = AsAd-major 全量)で確定し、Opus implementer に発注(root_sink ソースごと世代 + decoder ソース別カウンタ)。** **148 済 → archive(15:10): 2D 重心精度 σ/view 0.30 mm(232–502 ns)、1014 ns 0.4、HV 依存なし。ヒットの誤差 = (σ_s 0.30, σ_z 0.20) mm(SCANS S14)。** 14:40 ユーザーが UI から HV/LV を再投入 → **149 済 → archive(run 133: 31.8 kB/event、--ped で指標一致、ユーザー macros は ZS で壊れる → 本番は ZS なしのまま)**。**電源が要るものは全部実施済み**(② 141 続き = run 135–140、③ B の一晩 run = run 134 → 141)。**147 済 → archive(13:06、run 130–132): v ∝ E、拡散 −11 % @ +40 % E、電荷不変、σ_z は mm で一定 0.21(幾何で決まる)。ドリフトは 1,200 V に戻す(SCANS S12)。** **146 済 → archive(12:30): ヒット 1 本の z 精度 σ_z ≈ 0.2 mm ≈ 1 cell(232–502 ns)、1014 ns で +30–60 %、HV に依らず。幅を誤差に使うのは 10 倍過大(SCANS S11)。** **142 済 → archive(12:17、run 122–129): ln G = Σ a_i ΔV_i(a ≈ 0.014 ± 0.0015 /V、箔ごとの差は誤差内、加法性 OK)、−30 V まで飛跡無傷、推奨 240 × 3 + 502 × 120 fC(採否・base 変更はユーザー)。**運用点 B 採用(12:50): HV 240 × 3、現用 params = 502 ns × 120 fC × readout full(`base_1750.json` を B に置換、旧 A は `base_A_250x3_502x240fC.json`)。確認 run 130: 278.8 kB/event、飽和 0.6 %、ok 0.998、σ_z 0.21。**** GECO の接続は archive/145、RUNBOOK 末尾。**旧 TPC 機の PukiWiki(ELI-TPC Wiki)を ELITPC `http://192.168.144.220/` に移設(archive/144)。** **スキャン台帳 = `docs/SCANS_ja.md`(S1–S9)+ `tools/scan/ledger.py`(archive/143)— 以後のスキャンは表を生成して貼る。**E の解析(エネルギー較正 C-5、130 の素材) | 未。**139 は済 → archive**(19:15): 逆畳み込み後の電荷比 1 : 1.05 : 1.21 = 総電荷はどの peaking でも和で取れている(137 の読みを訂正)、幅は 0.55–0.75 µs の固有幅に収束。短整形 + 逆畳み込み運用は採らず、E は 502 × 240 fC のまま |

**17:33 復帰(ユーザー帰宅、Mac は朝まで sleep させない)**: run 101(取り直しの 20 s 試験)が **token 競合で stop 失敗 → 3.5 h 走りっぱなし**(データは 14:06 で止まっていた)→ 17:34 停止・base 復元・待機スクリプト 2 本 kill。**A は未開始**。138 の真因 = pulser テンプレートの `TestModeSelection` / `isSelectedforTestMode`(138 経過参照)→ 共有 `GetSoftware_config` へのテンプレート追加はユーザー承認待ち。D: run 100 の `calib/pedestal_502ns.json` 済。`drift_end.C` を run 84 に当てたら last_hit の 96 % が 510/511(窓端の跳ね)→ ヒット定義を [2,509) + run_metrics の ch 条件に直す発注(Sonnet、17:40)。
**14:07 VPN 切断(FortiClient が Disconnected。私からは再接続不可)→ ELITPC に届かない。** 実機で**自動で進むもの**: `pulser_kernel2.sh`(138 取り直し: run 101 = 232 ns / −300 mV / delay 750 の 20 s 試験 → ピークを cell ≈ 80 に置く delay で 8 run → base 復元・token 解放、14:25 頃完了)→ その直後に `start_A.sh` が **A(12.5 MHz / delay 3800、run ≈ 110)を自動開始**して走らせたままにする(予定より早いが内容は同じ)。**失われたもの**: gain2(1014 ns × 240 fC、140 の追加 1 run)は 14:05 に取り直しと競合して中断(base は取り直しの最後に復元される)。VPN 復帰後: `~/test/scan/{pulser_kernel2.log, start_A.log, A_run.txt}` を見る → 取り直し 8 run に `pulser_kernel.C` → 138 結果 → A を止めるときに gain2 を 1 run 挟むかはユーザーと相談。ドリフト端マクロ `tools/scan/drift_end.C` は Opus implementer に発注済み(14:10)。
**A の窓の見積もり(14:15)**: macros の L = 20 cm / 1200 V、91 mbar → E/p = 0.66 → 公称 v = 0.7233 × E/p = **0.48 cm/µs → t_max ≈ 42 µs**。delay 3800 の窓はトリガ後 38 µs(cell 33 以降)なので**公称では末尾が切れる**。HIGS 130 mbar の実測(0.390)は公称(0.334)より 17 % 速かったので、同じ比なら 0.56 cm/µs → 36 µs = cell ≈ 485 で収まる。復帰後に `drift_end.C` の `clip_back_frac` と last_hit の上端で判定し、切れていれば **delay 4000**(トリガ帯を cell ≈ 10 に、後ろ 40 µs)で取り直す。`drift_end.C` は main に取り込み済み(`7e9ae14`、83 チェック green、Opus)。implementer の指摘: ヒット定義が「閾値 35 を越えた cell」だけなので単独セルのノイズ(σ 7.6 で ≈ 0.3 個/event)が first/last を引っ張りうる → 実データのヒストの裾が平らなら run_metrics 型の ch 条件(ピーク > 35 かつ q ≥ 2000)に切り替える(まず run 84 で試す)。monitor / root-sink の QA 再構成は 25 MHz 固定(`src/reco` の MM_PER_CELL)なので A の z 表示は 2 倍ずれる(表示のみ、保存データは無関係)。

守ること: HV / LV / ガスには触らない(web からは不可、方針)。run の開始・停止はこの表の範囲で了承済み(ユーザー「進めてください」)。各 run の設定は台帳の `run_start.params` に自動記録。反映は RUNBOOK §7.1。

### 2026-10-16(週末 run と ZS の締め)
- **run 174 = 週末 run 1**(11:26 開始、B = `base_1750.json`、`[system] ped_table` 設定済み、ZS なし、comment「gas 100 ml/min」)。
  ユーザー: **ガスの流量が変わったら新しい run**(12:00 に 100 → 50 ml/min の予定)。合図が来たら `~/test/scan/stop_run.sh` → 同じ形で
  「weekend run 2: gas 100 → 50 ml/min」を開始(`start_W1.sh` の comment を変えたもの)。流量が変わるたびに同じ手順。HV / ガスには触らない。
  **12:08 実施**: run 174 停止(normal、eos_closed、11:26–12:08 = 42 min)→ **run 175 = 週末 run 2**(B、`elinp__0eb4f039`、comment「gas 100 -> 50 ml/min」、`start_W2.sh`)。次の流量変更も同じ手順(`start_W3.sh` を作る)。
- **157 完了 → archive**: SPEC **v1.49**(ZS 表モード・0x02 復元・ped_table・decoder カウンタ・§13-9)、WARSAW_PLAN §7 の ZS 行、RUNBOOK のカウンタ名訂正。
- **159 起票**(Opus): ZS カウンタの run ごとの経路 + PEventTPC 電荷和 + UI 基線。
- Fable キュー: 176(ML の方針)、**185 の申し送り = 歩き戻し(114)の第二段にも v1.53 の「下流が進む限り待つ」を適用するか**(SPEC v1.39 は「停止と同じ 2 段」、185 は stop_run だけ変えた)。181 は 2026-10-24 にスポット Fable でレビュー済み → v1.53。
- **160 起票**: スキャン報告書 v1(`docs/reports/2026-10_scan/scan_report_2026-10.pdf`、11 ページ、日本語)。ユーザー「自分用 + mini TPC メンバー、来月 Warsaw の資料にも」。

### 2026-10-19(月)
- run 175 を 07:52 に停止(67.7 h、1.06 M events、732 GB)。**161 完了 → archive / SCANS S19**: 流量 50 ml/min で電荷 −3.5 %、6 h で平衡、以後 60 h 平坦。HV 不変。
- 次の計画(ユーザー): **圧力スキャン 300 mbar まで、線源は端置き(中心から 70 mm、ケージ内)**。飛程表(TPCReco SRIM)と MC で、収納は 220–300 mbar、5 線(5.42–6.29)が 300 mbar で各 10–20 %、216Po 7 %、8.78 は常に突き抜け。GEM は Warsaw の圧力ごとの運用点(130: 245–255 / 190: 260 / 250: 270 → 300 で ≈ 280–295 V)を出発点に E 規則で決め直す。DRIFT は 2 kV 上限(E/p は 150 mbar まで維持)、12.5 MHz(6.25 は無い)。→ 票(162: 計画 + `alpha_lines.C` 発注)を起こす。
- **10:38 run 176 開始**(B、`elinp__116479fa`、comment「gas 50 -> 150 ml/min」、`start_W3.sh`)。ユーザーが流量を 150 ml/min に変更(161 の可逆性 + 感度係数の確認。**単位訂正: 流量は ml/min、チャンバー 54 L** → 体積交換 τ ≈ 0.54 h @150、平衡は 2 h 以内。期待 2.10–2.12 M。logbook の run 174–176 の comment は「ml/s」表記のまま = 誤記、正は ml/min)。停止は `stop_run.sh`、解析は `gas_trend.C`(1 h ビン)+ `metrics_for_run.sh --pressure 91`。
- **15:21:25 HV インターロック**(InfluxDB 1 s サンプル): 直前まで全 8 ch の vmon / imon は 1 LSB も動かず(GEM 8.68–8.86 µA、DRIFT 3.70 / 5.91 µA)。15:21:25 に **board の `bdilk` = 1 と同時に全 ch が KILL(status 2053 = ON+RDW+KILL)**で ≈ 10 V/s のランプダウン。電流スパイク・OVC・TRIP なし = 検出器の放電ではなく **外部インターロック入力**(ガス系 → HiVolta の線、131 B-1 / 2022 logbook「Interlock for HiVolta set to 80 mbar」)。15:49:54 に bdilk 解除(status 6144 = KILL+ILK → 1024 DIS → 0)。run 176 は 15:44 停止(176 の平衡値は 1.5 h 以降 2.08–2.10 M で確定)。原因はガス系側の記録(BPC / FlowView のアラーム)で確認する。
- 気づき: `/api/status` の `slow` が null(slowctl 稼働・Influx 書き込みあり)。要確認(別票)。

### 2026-10-21
- **167 完了 → archive**: DRIFT の電流差(ch6 3.70 / ch7 5.91 µA)は **ch6 の電流モニタのゼロ点 −2.38 µA**(OFF 時、10 日間一定)でほぼ全部説明できる。補正後は 6.08 / 6.09 µA で ±0.07 µA 一致、500–700 V で I ∝ V(≈ 99 MΩ / 600 V)= ドリフトケージは健全。ch6 は vmon も OFF で +1.85 V、600 設定で 596 V。漏れの監視は ch ごとにゼロ点を引いてから。**現地試験(131 B-6 ②、ビーム前)で ch6 の入力・出力を外しても読みが不変 → ずれは HiVolta ch6 の測定回路そのものと確定**。
- **168 起票(IN PROGRESS)**: mini TPC 動作実証レポート(読み手 = mini TPC メンバー、Warsaw は最終段階だけ)。§2 線の同定: run 175 / 178 で **止まる 3 本 = ²¹²Bi / ²²⁰Rn / ²¹⁶Po(100 が正しい)、161 / S19 の名前は 1 本ずれ**(中間 : 深い = 1.02、²¹²Bi が予言 +0.4 mm、E は表 +0.02 MeV・FWHM 1.5 %)。訂正はユーザーの独立解析の後。

### 2026-10-23
- **run 180 終了 08:35:15**(51,859 s、5,701 事象 ≈ 0.11 Hz、graw 1.6 GB、raw.root 738 MB)。**ビームは 10-23 00:00 まで、以後なし**(ユーザー)。
- **今朝の変更(ユーザー)**: CFD しきい値 3 V(300 mV)→ 2 V(200 mV)、triggerDelay 3500 → 3000(トリガ前 11 µs / 後 30 µs、173 §2h)。run 181–185(試験)→ **run 186(10:17:43 〜)**。params の note はまだ「delay 3500」のまま(要更新)。
- **174 の結果(臨時の解析)**: 共通モードの振動は **FPN に無く信号 ch にだけ乗る = 検出器側**、**ビームと無関係**(5.1 % / 4.8 %)、**トリガの ≥ 15 µs 後に立ち上がり窓の終わりへ育つ**(+4〜+14 µs は 0)。
  仮説 = イオンの転送ギャップ通過(10–25 µs)が HV のフィルタボックスの共振(≈ 600 kHz)を蹴る。25 MHz の窓(トリガ後 17.5 µs)では見えていなかった。予言: 圧力 ↑ で遅れ ∝ p、12.5 MHz のパルサーでは出ない。ユーザー: しきい値を下げるとノイズが乗る = 検出器側、フィルタボックスが怪しい。
  **→ パルサーで試験(run 187 / 188 / 189 = delay 3500 / 2500 / 4500、各 60 s、ユーザー了解、設定は `elinp__5b1ba034` に復元)**: パルサーでも 3.4 % 出る = **イオン説は外れ**。
  振動はどの delay でも**トリガ後 ≈ 20 µs までは出ず、以後に立ち上がって育つ**(割合 1.6 / 3.4 / 5.8 % ∝ 窓の終わり − 20 µs)、FPN には常に無い。
  = **各トリガの ≈ 20 µs 後に DAQ / トリガ側で始まる何かがストリップ側へ飛び込む**(入り口の最有力 = フィルタボックス)。飛跡は 0〜8 µs で実害は限定的、**delay 2500 なら ≈ 1.6 %**(頭切れ対策とも両立)。源はスコープ(トリガ出力でトリガ、フィルタボックスの出力を見る)で。

### 2026-10-22
- **HiVolta の較正(167 / 131 B-6)**: DMM(HMP2020 で 3.5 / 30 V 検定済み)で ch6 / ch7 単体 5–50 V の実出力 ≈ **0.934 × 設定**(ch6 はさらに −1.2 V)、vmon は設定どおり。GEM の ch と 600 V は未確認。
  **棚上げ**(ユーザー: 隣のビームライン・同じ部屋で U ターゲット実験 → 中性子で何か見るため、しばらく TPC を使う)。再開時は 131 B-6 ② GEM ch 単体 50 V → ③ 定格リード(未入手)で 600 V → CAEN。それまで HV の値は「設定値(実出力は未較正)」として扱う。
- **18:10 run 180 開始(ユーザー)= 湿度の推移 + 中性子プレ実験の設定**: params `elinp__88317fde`(12.5 MHz / triggerDelay 3500 / 502 ns × 120 fC / full / external、note 更新済み)、ドリフト 1,600 V(GECO)、CO₂ 目標 100 mbar(ビーム中で入室できず開始時の圧力は未確認、最後に見たのは 178 mbar、HV インターロックは 200 mbar 未満で解除)、²²⁸Th 線源は外してある(線源を外すときポートを開けて湿気が入った)。
  ユーザー: 信号は綺麗、ときどき信号後にオシレーション(137 / 138 の「揺れ」= 電子回路のリンギング、≈ 1.6–1.8 µs → 12.5 MHz では ≈ 20–22 cell の見込み)が来て減衰。入室できたら 100 mbar 到達時刻(BPC / FlowPlot)で run の頭を区切る。173 参照。
- **PC–zCoBo を光ファイバに(ユーザー)**: ELITPC の `enp26s0f0`(ixgbe、1 Gb/s)に NM 接続 `zcobo-fiber` = 192.168.4.1/24・MTU 9000・ゲートウェイなし(旧 `Wired connection 2` の DHCP を書き換え、sudo はユーザー)。銅線側 `netplan-enp179s0f0`(107)は autoconnect no で保存。
  確認(読み出しのみ): ping 192.168.4.81 3/3 0.19 ms、DF 1472 / 8972 とも 3/3(0.46 ms)、4.x のデフォルト経路なし、DAQ 各部 Idle・到達可、スタック再起動なし。ECC → zCoBo の制御は次の run 開始で確認(共有スタックなので run はユーザー)。
  **パルサーで確認(ユーザー依頼)**: 11:09 の 1 回目は LV OFF(HMP2020 を DMM 検定に使った後)で ECC prepare 失敗「AsAd … disconnected from its power supply」→ 設定復元・token 解放。ユーザーが LV ON(ch1 3.6 V、ch2 は未接続で 3.0 V のまま問題なし)。
  **run 179**(11:12:07–11:12:31、template `configure-elinp-pulser.xcfg`、−300 mV / 50 ms、onSlowControl、delay 1750、他は B のまま): 403 frames(type 2)、EOS 正常、graw 112 MB、256 信号 ch × 403 = 103,168 波形・飽和 0。
  振幅中央値 999 ADC・立ち上がり 7.46 cell・FWHM 19.8 cell・peak cell 183 = 138 の run 107(989 / 7.4 / 19.7 / 182)と同じ。LV ch1 は prepare で 0.017 → 1.24 A。**光ファイバでの取得は問題なし**。設定は `elinp__116479fa`(B)に復元、token 解放。保存した直前設定 `~/test/scan/before_pulser_check_20261022_*.json`。

## 開いているチケット

| # | 中身 | 状態 | 次の一手 |
|---|---|---|---|
| ~~151~~ | root_sink: `[system] ped_table` + ZS 対応 PEventTPC | 済 → archive(86792dd、§12-3 不変を実 graw 3 組で確認) | — |
| ~~152~~ | monitor: 0x02 の ZS 復元 | 済 → archive(a46c8be) | — |
| ~~153~~ | uvw_closure.C に pedJson | 済 → archive | — |
| ~~154~~ | zs_restore.C(macros 無改造の橋)+ `--zs-fill` | 済 → archive(ba2d332、run 133 で macros 復活: 85.4 mm / σ_z 0.20) | — |
| ~~155~~ | zs_params.sh + RUNBOOK「ZS 運用」 | 済 → archive(72f0703) | — |
| ~~156~~ | 検証 α run → S18 | 済 → archive(run 170–173: ZS は品質不変で 1/9〜1/12、FPN は抑制) | — |
| ~~157~~ | SPEC v1.49 + WARSAW_PLAN | 済 → archive(2026-10-16 Fable) | — |
| [160](160_gas_flowbus_second_host.md) | ガス(Bronkhorst FLOW-BUS)を ELITPC から読む: メーター PT104/Inlet の D-sub(キャップ付き未使用)を第 2 local host に。方式・配線・手順は確定、コード変更なし | READY(現地作業、ユーザー) | 現地に行けるとき。RS-232 専用ケーブル(7.03.367 相当)+ USB-RS232 を用意 |
| [159](159_zs_counters_per_run.md) | ZS カウンタを run ごとに(root_sink close 行 / status 14・15 キー / run_stop)+ PEventTPC 電荷和の照合 + UI 基線の目視 | READY(Opus) | ZS run の合間に |
| ~~158~~ | decoder: frameType カウンタ + compact サイズ検査 | 済 → archive(1415c23) | — |
| [189](189_preamp_al_foil_shielding.md) | トリガ用プリアンプ(フィルタボックスの後)にアルミフォイル — 主 = 物理の取り方で一定時間のトリガレートと振動起動の割合、副 = ペデスタル(前 = run 202) | IN PROGRESS(巻くのはユーザー) | 後の run を取ったら解析 |
| ~~190~~ | 全事象の UVW(web UI と同じ描き方)を 1 事象 1 ページの PDF に(`macros/drawUVW.C`) | 済 → [archive/190](archive/190_uvw_event_pdf.md)(テスト 32 PASS、run 209 = 7,946 ページ 4.18 GB を `macros/data/run0209_uvw_events.pdf` に。2 GiB 超は ROOT の xref が壊れる → `mutool clean`) | 173 §4 の目視分類(ユーザー) |
| ~~191~~ | drawUVW に XY / XZ / YZ(模型なしの 3D 点)、既定の出力 = 事象ごとの PNG ディレクトリ | 済 → [archive/191](archive/191_drawuvw_3d_png.md)(テスト 59 PASS、run 209 = 7,946 枚 281 MB を `macros/data/run0209_uvw_events/` に) | 173 §4 の目視分類(ユーザー)、190 の PDF を消すかはユーザー |
| [192](192_tpcreco_reco_display_survey.md) | TPCReco の再構成と表示の調査: 人のクリック(当てはめなし)/ 自動 = 直線 + α・¹²C 一直線の仮説 / 旧 Hough + 折れ線 / **順方向の模型による N 本の当てはめ(Cwiok 2023、頂点 + 終点、ブラッグ + 拡散 + 整形で UVW を描いて χ²)**。提案 = mm・自動ズーム・縦横そろえの表示 → 3 段(データ / 模型 / 残差)→ 順方向の当てはめを芯に ML と共有 | DRAFT(調査済み、Fable 所見あり 2026-11-02 → 194–198 を起票) | 198 は 197 の後 |
| ~~193~~ | drawUVW: 上段 UVW を mm・自動ズーム・3 面共通の縮尺で縦横そろえ | 済 → [archive/193](archive/193_drawuvw_mm_zoom.md)(テスト 99 PASS、244 が塊いっぱい、27 はズームせず → 199) | — |
| ~~194~~ | 暫定選別 v1 `selectNeutron.C` | 済 → [archive/194](archive/194_select_neutron_v1.md)(道具は完成。**片端だけ内側の組は線源の超過 ≈ 3.5 /h が背景 ≈ 50 /h と形・出る側・q/mm で区別できない**、きれいなのは contained + 4.0 /h と blob + 3.2 /h) | 選別 v2 = Fable キュー |
| ~~195~~ | 短い腕は空間か時間波形か | 済 → [archive/195](archive/195_short_arm_waveforms.md)(27 は artifact ではない = z に ≈ 7–15 mm の実の長さ、5 は決まらず、飽和 strip は窓の終わりまで尾を引く → 197 で χ² から外す) | 198 で当てはめ |
| [196](196_vdrift_absolute_calibration.md) | v_drift の絶対較正(198 mbar / 1,600 V / 12.5 MHz で α 線源 1 h、dip 角で mmPerCell) | WAITING(チャンバーを開ける日に、ユーザーの run) | E_n・開き角を数字で出す前に |
| ~~197~~ | 順方向の模型による当てはめ v0(α 1 本)`fitProngs.C` | 済 → [archive/197](archive/197_forward_model_fit_v0.md)(テスト 107 PASS、閉包 0.06 mm、TPCReco と 3.85 %(**TPCReco の σ_z の不具合を発見**)、壁 α 20 / 20 で頭尻が物理と一致、ただし χ²/ndf 中央 23–107 ≫ 1) | 198(Fable に設計を相談) |
| ~~199~~ | drawUVW のズームを連結成分で強く | 済 → [archive/199](archive/199_drawuvw_zoom_components.md)(テスト 109 PASS、27 もズーム、run 209 全 7,946 枚 236 MB を作り直し) | 目の偽陽性率の試験(ユーザーに確認中) |
| ~~200~~ | 目の偽陽性率の試験 | 済 → [archive/200](archive/200_blind_eye_test.md)(**偽陽性 2.0 %**、片端だけ内側では目も見分けられない、contained は効く、reject で差 6 / 131 vs 1 / 174) | 198 で reject 6 本を当てはめる |
| ~~201~~ | fitProngs v0.1(箱・縛り・A 固定・共通モード・誤差模型・mm 表示・初期線分の切り詰め) | 済 → [archive/201](archive/201_fitprongs_hardening.md)(テスト 171 PASS、壁 α χ²/ndf 中央 24.6 → 3.78(目標 3 は未達 → 203)、内部 86 本 107 → 5.5、**A_ref = 81,406 ADC/MeV/面**) | → 198 |
| ~~198~~ | 2 本の仮説 H1 {α} vs H2 {α + 頂点から出る一様な腕}(`fitProngs.C` v0.2)| 済 → [archive/198](archive/198_two_prong_hypothesis.md)(v2 = 向きの罰則・A の箱 ± 15 %・沿う成分を種から外す・h1Broken / h2Worse の印、テスト 346 PASS、T = 5,004.6(208 の片端組 200 本)、壁 α の超過 1 / 20、**entry 27 = 腕 12.6 mm −z**、片端組 209 − 208 = 0.010 ± 0.024 で分けられない、208 の偽陽性 9 / 10 は α の真後ろへの腕)| 次の票は Fable キュー |
| [202](202_flaky_tests_under_load.md) | 負荷のかかった全体実行でだけ落ちるテスト 3 件(receiver_stale_link の T4 = 184 の DONTWAIT と接続の競合、root_sink_monitor_pub の 10 s、graw_replay_tool の TOCTOU) | TODO(11/9 の後) | — |
| [203](203_strip_even_odd_response.md) | 偶数 / 奇数 strip で応答が 2–6 % 違う(201 ⑤、pad の形か利得か配線か) | TODO(Fable キュー → 2026-11-04 スポット Fable に相談中) | — |
| ~~204~~ | 停止が root_sink の積み残しの最中に `error:eos-timeout`(run 214)= 181 の実機受け入れ不合格 | 済 → [archive/204](archive/204_stop_eos_timeout_when_root_sink_backlogged.md)(controller が root_sink の status を購読して close まで待つ + ロスレスの守り、SPEC v1.54、main `343f571`、**実機 run 217: 停止 698.6 s で ok:true / normal、graw 23,309 = 受信、落とし 6,309 は数えて記録、events_built 17,000 = 23,309 − 6,309、lost_receiver_to_decoder 0、root_sink 正常 close**)| — |
| [206](206_p3_f2_stale_exit6_expectation.md) | `p3_error_paths` の F2 が 186 以来落ちている(root_sink の exit 6 を期待する古いテスト、env ゲートでだけ走る)| TODO(Sonnet、204 / 188 の後)| — |
| ~~207~~ | analyzeUVW の strip 電荷の切りが 232 ns で高すぎて偽の contained(run 216 で 3.1 /h)| 済 → [archive/207](archive/207_strip_q_min_per_peaking.md)(macros: `stripQMin` 引数・`par_analyzeUVW.sh -q`、**232 ns = 1,100**(谷 950–1,150)、run 216 で contained 0・wall-alpha 4.7 /h、215 は既定でビット一致)| 232 ns の run は `-q 1100` |
| [210](210_online_reco_strip_q_min.md) | オンラインの再構成 `src/reco` の `STRIP_Q_MIN = 2000` を peaking ごとに(表示だけ)| TODO(Fable キュー: SPEC §3.1・§5.5)| 11/9 の後 |
| [208](208_controller_root_sink_status_followups.md) | 204 の残り 2 件(run 番号の再利用で前回の close を取り違える、hist_snapshot の全復号)| TODO(Sonnet、11/9 の後)| — |
| [209](209_graw_writer_status_during_disk_stall.md) | ディスクが詰まった graw-writer は GetStatus にも答えず(blocking の lock)、204 の守りが Off → 停止で graw の送出待ちを捨てる(ユーザー裁定 B-4 が実際には効かない)| TODO(**ユーザー: 後で** — 2026-11-04)| 11/9 の後 |
| ~~188~~ | controller の run 中監視(2 s)+ 保存系 Error の自動異常停止 | 済 → [archive/188](archive/188_controller_watchdog_auto_abort.md)(main `efef4b6`、**実機 run 218: graw_writer SIGSTOP → 溢れてすぐ `abort:overload:receiver0`(operator watchdog)、SIGCONT で eos_closed:true、overflow 400 は数えて恒等式どおり**。対照 run 219 は停止 4.4 s で ok / normal)| — |
| [187](187_ui_v153_overload_and_draining.md) | UI: STOPPING — draining・ROOT DROPPED・ログブックの root_complete・OVERLOAD | **main にマージ `654f2fd`**(413 PASS)、**ELITPC 反映済み**(2026-11-04)、ユーザーの目視(STOPPING — draining)待ち | ③ と ① は 11/9 前 |
| [205](205_n1068ge_settings_at_run_start.md) | トリガ(N1068GE)の設定を run 開始時に読んでログブックの `run_start` に残す(読むだけ、ポーリングなし、`CH:16` の MUX は送らない)| TODO(Fable キュー: 経路) | 11/9 の後 |
| ~~184~~ | receiver の decoder リンク(数えて落とす)| 済 → [archive/184](archive/184_receiver_decoder_link_counted_drop.md)(2026-11-03 コミット `e553bb2`) | 実機の受け入れ(ELITPC 反映後) |
| ~~185~~ | controller の停止(下流が進む限り待つ・リンクの数・root_complete)| 済 → [archive/185](archive/185_controller_stop_drain_and_link_counts.md)(2026-11-03 マージ `d10ab33`) | 実機の受け入れ(ELITPC 反映後) |
| ~~186~~ | root_sink: 次の run が来たら未完の run を閉じる | 済 → [archive/186](archive/186_root_sink_close_incomplete_on_new_run.md)(2026-11-03 マージ `6720087`) | 実機の受け入れ(ELITPC 反映後) |
| ~~183~~ | root_sink: PEventTPC(TPCReco、Warsaw 向け)をオプションに(`[root_sink] pevent`、off なら `--no-root`、monitor.root は raw 側)、SPEC v1.52 | 済 → [archive/183](archive/183_pevent_optional.md)(1 回目の反映で Rust の config が `pevent` を拒否し DAQ が ≈ 10 分停止 → 復旧・修正 `2911209`、**ELITPC は pevent = false で稼働**) | — |
| ~~182~~ | root_sink の持続レートを実機で(run 198–204) | 済 → [archive/182](archive/182_root_sink_throughput_bench.md)(PEventTPC あり workers 1 = 16.6 Hz / 4 = ≈ 66 Hz、**PEventTPC なし = 169 Hz に完全追従、上限 ≈ 220 Hz**、raw ツリー 1 本 ≈ 5 ms / 事象) | — |
| [181](181_root_sink_rate_limit_run197.md) | 高レートで graw まで落ちる件(run 197)の設計 — 保存系 = graw-writer のみ、ROOT 側は数えて落とす(ユーザー裁定)、Fable レビュー済み、**SPEC v1.53 / CLAUDE.md 改訂済み(`a5d37e9`)** | 実装中(184–186 発注、188 監視・187 UI は次) | 184–188 の後に実機で run 197 の再現(§12-14) |
| ~~180~~ | ELITPC: UI のテンプレートを物理の本番 `configure-elinp.xcfg` 1 本だけに(ユーザー指示) | 済 → [archive/180](archive/180_templates_physics_only.md)(他 25 本 + prepare 16 本を `GetSoftware_config/retired_templates_2026-10-24/` へ移動、current = `elinp__6cf41a72` = run 194–195 と同じ xcfg) | 同日 pulser 用 `configure-elinp-pulser.xcfg` だけ戻した(一覧 = 物理 + pulser の 2 本) |
| ~~179~~ | params: 周期トリガの周期 `triggerPeriod_ms`(`Module.triggerPeriod`、[1, 21474] ms)をノブに + `triggerPeriod_10ns` のあるテンプレートは render がエラー(SPEC v1.51)。契機 = run 196 のペデスタルが template の 4000 ms で 0.25 Hz | 済 → [archive/179](archive/179_params_trigger_period_knob.md)(`1876f8c`、cargo 1108 / UI 402。**ELITPC 反映 2026-10-24 17:10**: current.json に 4000 を移行、生成 xcfg は run 196 とバイト一致) | 新ノブで run を取る確認(ユーザー) |
| ~~178~~ | macros: サンプリング周波数とドリフト電圧を引数に、run の定数を `_uvw.root` / `_tracks.root` に記録(hadd でも保つ merge 'f')、stoppedAlphas の z 窓を記録値から | 済 → [archive/178](archive/178_macros_run_constants_as_args.md)(macros 40 / 0、改修前と 3,000 entries 一致、macros オラクル 7 passed。呼び出し側 oracle / event_mix も新署名に) | ELITPC の `~/test/ana` と `~/test/scan` の写しは使うときに入れ替え |
| ~~177~~ | tpcsim への受け渡し `~/test/tpcsim_handoff/`(条件カード、応答の核、較正表、空の窓、データのパス、macros の写し 95e52b5) | 済 → [archive/177](archive/177_handoff_to_tpcsim.md)(12.5 MHz ペデスタル = run 198、4,665 事象) | tpcsim 側の TODO/14 のチェックはユーザー |
| [176](176_ml_event_selection_plan.md) | 機械学習による事象の選別の方針(失敗の型を潰す順番: デジタイザ + 実データの雑音のオーバーレイ → α で答え合わせ → 小さな分類器 → 「わからない」の出口 → 人間の目は最終試験 → 歪みの測定) | DRAFT(**Fable キュー**: 設計の確定) | 確定後にデジタイザの発注書 |
| ~~175~~ | UI の EVENTS(status の events_built)を run ごとに(SPEC v1.50、スポット Fable 確定) | 済 → archive(ELITPC 反映 11:51、run 191 / 192 で開始 0 から・閉じた後 = close 行 470 / 463 と完全一致) | — |
| [174](174_common_mode_burst_scan.md) | 共通モードのオシレーション(バースト)が信号と独立か: `tools/scan/burst_scan.C`(FPN の中央値で検出、位置の一様性 KS、周期、FPN / 信号比、時刻に対する率)+ 合成テスト | IN PROGRESS(道具は完成・照合済み: 合成 49 ok / 0 失敗、run 175 の先頭 10 万事象でバースト 0.0 % = 当時は ±150 ADC 級の振動なし) | run 180(今晩ビームのオン → オフ)に適用、オフの時刻はユーザーが翌朝 |
| [173](173_neutron_u_target_estimate.md) | 隣のビームライン(U ターゲット)の中性子を 100 mbar で見る見積もり: C/O 反跳 1–3 cm、¹⁶O(n,α)¹³C の 2 本の飛跡で E_n 再構成の可能性、E_n < 3 MeV の反跳はトリガ下限の近く | DRAFT | 測定条件(GEM・しきい値・線源)をユーザーが決めたら解析を起票 |
| ~~172~~ | UI: status の ZS 復元カウンタ 3 キー(152 で monitor に追加)を受け取り、ステータスバーに表示(「unknown status keys」の解消) | 済 → archive(ng test 401 / 10 skipped、build・適合 OK)。**ELITPC 反映済み(2026-10-22 14:27、ユーザー了解、ガス圧待ちの Idle 中)**: Mac で ng build → `/opt/tpcdaq/ui/` に上書き(main-OWBLVQ7U.js、8080 で 200)、`~/tpcdaq-rs` を bundle(04c9ad1..d73740d)で ff。04c9ad1 以降 src / root_sink の変更なし = バイナリ再ビルド・再起動不要 | ブラウザはリロードで新 UI |
| [171](171_slow_view_age_delivery_lag.md) | Slow control 画面の age が巨大になる(長時間ほかのページの後、リロードで直る)。時計・ドライバ・slowctl・monitor キューは否定、残る候補 = 配送の遅れ | DRAFT(優先度低) | 直すなら age = 配信時刻 − 取得時刻 + 受信遅れの別表示(Sonnet)|
| ~~170~~ | macros/README.md の更新(SAMPLING_MHZ / DRIFT_HV_V の定数、頭尻の記述、stoppedAlphas の手順) | 済 → archive | — |
| ~~169~~ | macros: analyzeUVW の並列化(`par_analyzeUVW.sh -p 91`)+ 圧力・線源位置の引数を必須に | 済 → archive(run 175: 90 s vs 直列 ≈ 14 分 = 9.4 倍、直列 == 並列をビット一致で確認、15 / 15 PASS) | — |
| [168](168_minitpc_working_report.md) | mini TPC 動作実証レポート(α 3 本・分岐比・直線再構成・飛程 → E)。読み手 = mini TPC メンバー | IN PROGRESS(§2 同定済み: ²¹²Bi / ²²⁰Rn / ²¹⁶Po) | ユーザーの独立解析 → 照合 → 161 / S19 / 160・163 / 162 の名前の訂正 → 本文 |
| [166](166_cf252_fission_fragments.md) | ²⁵²Cf 破片の長さ(飛程)分布で軽 / 重の 2 山を見る(250 mbar、低利得の別運用点、内向き 1 本) | DRAFT | 線源は密閉型(Si で使用実績)。過去の Si スペクトル(破片 2 山の位置)と放射能の確認(ユーザー)|
| [165](165_environment_monitoring.md) | 環境モニタ: 露点(Easidew、ドライバ新規)・室温湿度気圧(arduino_env)・チャンバー温度(センサー選定)・チャンバー圧(bronkhorst)を slowctl → InfluxDB → run の `slow` へ | READY | ガス PC の LabVIEW / FlowPlot に 10-16〜21 の室温・圧力ログがあるか確認(ユーザー)|
| [162](162_pressure_scan_plan.md) | 圧力スキャン(≤ 300 mbar、線源端置き、z 中間、x 位置可変)の計画 + `alpha_lines.C` 発注 | READY | 線源の設置後。alpha_lines は Sonnet へ |
| ~~161~~ | ガス流量 100 → 50 ml/min(run 174 / 175)→ SCANS S19 | 済 → archive(2026-10-19: 電荷 −3.5 %、6 h で平衡、以後平坦) | — |
| ~~161~~ 追記 | run 178(200 ml/min、17.2 h): 差は流量でなく**温度 +1.3 K(密度 −0.45 %)**— 止まる α 線 3 本が 1.5–1.9 mm 奥へ、利得 ≈ +2 %。利得の温度感度 ≈ 1.5 %/K。SCANS S19・報告書(日英)に反映 | 済(2026-10-21) | 露点の記録で水の変化を確認(ユーザー)|
| ~~164~~ | 付録 A が指す日本語文書の英訳(`docs/en/`: SCANS / SPEC / RUNBOOK / チケット 135–161) | 済 → archive(2026-10-20、7 担当並列、照合全 OK) | — |
| [163](163_scan_report_en.md) | スキャン報告書の英語版(S1–S19、mini TPC + Warsaw 向け、非公式) | IN PROGRESS | — |
| [160](160_scan_report_2026-10.md) | スキャン報告書(S1–S18、日本語 PDF、`docs/reports/2026-10_scan/`) | v1 済、ユーザーレビュー待ち | 直し → 英語版は要望次第 |
| [131](131_onsite_and_hardware.md) | 現地・機器・ELITPC 実機でやること(069 / 073 / 079 / 084 / 101 / 107 の残り) | READY | やれるときに 1 項目ずつ |
| 130(未起票) | 複数飛跡の再構成(strip ごとの複数パルス、view ごとの直線探索、view 間の対応付け、N 本のフィット — 3α まで) | 前段の 126–129 は完了。**2026-10-13 ユーザー: 後回し** | 設計は Fable。データ = 128 の重ね合わせ + gas-pressure-test の反応生成(真値つき) |

## 解析バックログ(起票は着手するとき — 「詳細チケットは 1 フェーズ先まで」)

元は 080(プログラム票、archive)と 069 Phase 2.5。

- **run 条件 → v_drift、窓の深さ、窓端クリップ率**(080 B-4)— 25 MHz / delay 1500 でも**トリガ前後の配分とクリップ率は未測定**。急ぎではないが、次の線源 run の前に:
  - 圧力と HV から `v_drift = 0.7233 × E/p` を出し、run_start に書く。
  - f_sample と triggerDelay から、窓の深さ [mm] とトリガ前後の配分を出す。
  - monitor に窓端クリップ率のカウンタを出す。
- **較正解析**(069 Phase 2.5 C-3〜C-5 / 080 D-4):
  - α run のデータから σ_T・σ_L・strip PRF・イオンテール平均波形・ドリフト端の v_drift・ADC/keV を出す。
  - 066 ハーネスの mini .dat 切替(069 0-5)を含む。
  - ドリフト端は 100 MHz の窓では見えなかった。25 MHz(2026-10-11〜)のデータなら使える。
- **較正表の適用**(080 D-2): onMultiplicity 閾値の個別化を 078 の例外リストへ流す。ゲイン正規化は ROOT プロダクト側(生 graw は不変)。
- **イオンテール + Unfolding**(080 D-5): 較正解析の後。
- **macros 配布準備**(080 D-7): LICENSE、channel_map.csv の扱い、ビルド産物の .gitignore。macros はユーザーのもの — 判断はユーザー。
- **QA ヒスト id 20 番台**(074 の残り): 130 の後、130 で扱う。

## Fable 待ち(キュー)

Opus 主対話中に出た設計判断・SPEC 疑義・レビュー依頼をここに積み、Fable で**まとめて**確定する(運用ルールの正は CLAUDE.md「モデル使い分け」)。

- **198 の後の次の票(2026-11-04 積み)**: 分類器として使えるか(片端組で 209 − 208 = 0.010 ± 0.024)、cos ≈ −1 の縮退(208 の偽陽性 9 / 10 = α の真後ろへ伸びる腕、2 体運動学の許す領域なので罰則で止められない)、A の箱の幅(端に 21 %)、{α, α}・H0・N = 3、電荷の勾配、203、232 ns の較正 — どれを次に切るか(archive/198 の結果 v2)。
- ~~198 A–D + 203 の順序~~ **裁定済み(2026-11-04 スポット Fable)**: 腕が α に重なるのは模型の縮退(H2 ⊃ H1 + 振幅の自由度)→ 向きの罰則 + A を ± 15 % の箱で自由に + 種の除外 + 印(h1Broken / h2Worse)、T の対照は run 208 の片端組 200 本、③・④ は測定に、203 は 198 の後、232 ns は別票。v2 を implementer(Opus)に発注。元の記述: 198 の実装は green(268 PASS)、受け入れは ② entry 27 合格・③ 0 / 10・④ 健全 62 本の 37 が hyp 1 — 腕が α に重なって H1 の系統的な残差(203)を吸う。分岐 A–D は 198 の結果節。
- **210 = 207 のオンライン側(2026-11-04 積み)**: `src/reco` の `STRIP_Q_MIN = 2000` を peaking ごとに — `[monitor] reco_strip_q_min` を TOML に足す(SPEC §3.1・§5.5)か、params の peakingTime から引くか。表示だけの影響。
- **205(2026-11-04 積み)**: N1068GE の設定の記録の経路 — slowctl に「run 開始の合図で 1 回読む」口を作るか(Opus 推し)、controller が直接読むか、params 側で照合するか。11/9 の後でよい。
- ~~204~~ **裁定済み(2026-11-04 スポット Fable + ユーザー裁定)**: (a) を厳密化 = controller が root_sink の status を購読して close まで待つ、graw-writer が開いている間は打ち切らない、歩き戻しの第二段も同じ関数に(= 185 の申し送りの裁定も「統一」)。188・187 を起票。SPEC v1.54 案は 204 §5(マージ時に反映)。
- **203(2026-11-03 積み)**: 偶数 / 奇数 strip の応答差(U 0.058・W 0.042・V 0.022)の原因と直し方、198 の前か後か。
- **194 → 選別 v2(2026-11-02 積み)**: Fable 裁定の「頂点が内側 + 電荷の並び」は、片端だけ内側の組で背景 48.5 /h(208)を落とせなかった。原因は端の 5 mm の窓がにじみを見ていたこと(分布の最大は端から 12–20 mm)だが、形を 3 分割で見ても、上流 / 下流、q/mm でも、208 と 209 の内訳は同じ(archive/194 の結果)。→ 片端だけ内側の組の選別をどうするか(197–198 の χ² を待つ / 端の幅 = 拡散で z を測る / ML に回す)。目の偽陽性率の試験(208・209 を混ぜて伏せる)をやるか(ユーザーに確認中)。

- **181(2026-10-24 積み)**: 高レートで DAQ が壊れる件の設計 — ① receiver の送信スレッドが graw / decoder 共用で、派生物(ROOT)の遅さが正本の graw を落とす(run 197 で 19,855 喪失)→ graw のリンクを切り離す、ROOT は数えて graw から作り直せる側に(SPEC の「root-sink も保存系」を改める案、ユーザーの判断も要る)② Stop / Reset で途中のフレームが数えられずに消える(≈ 1,097)③ EOS を失ったとき root_sink の run を閉じる正規の手段 ④ Stop 後の drain 中の UI 表示 ⑤ 周期トリガの目安・下限。材料 = 182(PEventTPC off で 100 Hz は 2 倍の余裕、ユーザーは 2 年以内に 100 Hz の実験)。
- ~~134~~ 完了 → archive(2026-10-14、SPEC v1.47)。
- ~~135~~ / ~~136~~ **完了 → archive(2026-10-14)**: パラメータスキャンの道具(`tools/scan/`)。試運転 = triggerDelay 5 点(run 78–82): 帯の位置は `first = 508 − delay/4` で厳密に線形、トリガ遅れ L ≈ 0.16 µs、**推奨 delay 1750–1850**(現用 1500 はトリガ前 5.4 µs を捨てている)、2000 は 15 % が先頭で切れる。~~次 = peakingTime~~ **[archive/137](archive/137_peaking_time_scan.md) 済(run 83–86)**: 電荷は 232 → 502 → 1014 ns で 1 → 2.3 → 5.2 倍(現用 232 は積分不足)、70 ns は ok 66 % で大損、1014 は 1/3 のイベントが飽和。**gain スキャン(502 ns × 120 fC / 240 fC / 1 pC、1014 × 1 pC)を 13:30 に開始(run 87〜)**。**ユーザー裁定(同日): 現用を 1750 に**(`elinp__e090134b`。25 MHz では 1500 でもお尻は切れないが、他のクロックにも使えるルールとして)。**ルール**: 帯の先頭をトリガの ≈ 2.8 µs 前に置く → `delay[10 ns] = (窓 [ns] − 2,800 − 160) / 10`(25 MHz: 1752 → 1750、12.5 MHz: 3800、100 MHz は窓 5.12 µs なので 1 µs 前 = 396 が限度)。**peakingTime スキャン(70 / 232 / 502 / 1014 ns、各 1,500 events)を 1750 で開始(12:58 EEST、run 83〜)**。
- ~~SPEC v1.46~~ **確定(2026-10-13、スポット Fable、`61d88d4`)**:
  - 対象: 110(取得の再開)、123(`/api/last-event`)、092(EOS 後の Heartbeat なし)、132(run/start の `params_id` / `notes`)、069 の出口(§13-2/3/6/8 を実測済みに)。
  - Fable が文案から直した点: HMP2020 は HO720 = FTDI VCP(ttyUSB0)、§6.3 の判定は実装の 4 条件、§10.5 の 404 の条件 など。
- ~~133~~ **完了 → archive(2026-10-15、SPEC v1.48)**: 取得の再開の判定(4 AsAd)。**2026-10-14 Fable レビューで格上げ**: 送出が AsAd 毎のリング順なら、再開検出後に来る他 AsAd の古いフレーム(idx 2057 など)が新 epoch の pending に入り、タイムアウト emit で高水位が古い値になって新しい取得が late で落ちる = 原事故の再発。idx でなく時刻で世代を分ける設計が要る。一次資料で送出順の確認から。

## 保留・確認事項

- **実 ECC の例外取りこぼし 2 箇所**(043 発見 — 上流仕様なので改変しない。運用留意):
  - `GetEccImpl::breakup` は、失敗時に Ice **UnknownException** を返す(SM::Exception を catch しない)。
  - `onUnPrepare`(reset の Prepared→Described)は、失敗時に dhsm が **halt** する。我々の map では `Off` に見える。
  - 復旧は ECC 再起動(069 #14 で演習、RUNBOOK §5)。
- **Warsaw 確認**: TPCReco 再配布許諾(020 — third_party/tpcreco 昇格の条件)/ PROPOSAL v0.5 反映判断。
- **物理屋向け資料・デモは UI + ファイルデータソース完成まで待つ**(ユーザー決定)。
- 小粒: poisoned 時 metrics の `PoisonError::into_inner`(023 申し送り — 032 実施済み分以外の箇所が残っていれば、次に該当ファイルを触るユニットで確認)。
- **delila-rs への申し送り**:
  - pop_for 競合 → issue 化済み https://github.com/ELI-NP/delila-rs/issues/26
  - ZMQ fair-queue 飢餓(013)も要点検。
- 決着(2026-10-13): 有界キューの単位は**個数建てのまま据え置き**。ELITPC はメモリ 123 GiB で、182 Hz × 30 s の RSS 最大 3.1 GB(069 #20、109)。

## 運用メモ(常時適用はメモリ・CLAUDE.md 側が正)

- C++ の make は必ず `-j`。
- Mac の ROOT は 6.40.04。cling / rootcling を使うときは `SDKROOT=…/MacOSX27.sdk`。Garfield++ も 6.40 で再ビルド済み(メモリ mac-root-cling-sdkroot)。
- **IDE**: `.vscode/c_cpp_properties.json`(gitignore 内・ローカル)に ROOT include + C++17/libc++ を設定済み。worktree レーンを多数出すときは VS Code の列挙負荷に注意(`.vscode/settings.json` で除外済み)。
- **実 .graw + 実 mini ジオメトリの正しいペア**はメモリ参照(合成 fixture と混同しない)。
- ELITPC の操作: run 操作・サービス再起動・負荷試験はユーザーの了解を取ってから。HV の値は絶対に変えない(on/off は 124 の経路だけ)。
- implementer の報告は `git show --stat` と発注書のテスト名で照合してから受け入れる(CLAUDE.md)。
- どんな小修正でも連番チケット + 結果節 + archive(ユーザー方針)。

## 完了ユニット台帳

000〜129 のうち、上の「開いているチケット」以外はすべて [archive/](archive/) に格納。単位の詳細・テスト実測値・逸脱の裁定は、各 md の「結果」節(処置で閉じたものは「処置」節)が正。
日付ごとの流れは archive の CURRENT_* を参照。
