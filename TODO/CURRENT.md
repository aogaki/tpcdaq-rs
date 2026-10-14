# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-10-13(棚卸し — 止まっていた 069 / 073 / 074 / 078 / 079 / 080 / 084 / 101 / 107 を処置して archive、~~092~~ を修正して archive([archive/092](archive/092_receiver_abandoned_after_eof.md) — EOS の後も Heartbeat を出し続けていた)、131 を起票。続けて ~~124~~(全 ch の ON/OFF をユーザーが実機確認)・~~132~~(run/start 応答に params_id / notes)・~~110~~(未停止で落とした後の古いフレーム = 取得の再開 epoch、ELITPC で再現・確認)を完了 → archive)。**
2026-08-18〜10-13 の作業ログ(Wave 1〜3、ELITPC 移行、125–129)は [archive/CURRENT_2026-10-13_wave_log.md](archive/CURRENT_2026-10-13_wave_log.md)、
その前は [archive/CURRENT_2026-08-14_p2_p3wave.md](archive/CURRENT_2026-08-14_p2_p3wave.md)。

## いま(1 分で読める要約)

- **本番**: ELITPC(旧 puppex、Ubuntu 26.04)で mini eTPC を運用中(2026-10-08〜、[archive/107](archive/107_elitpc_host_setup.md))。
  - 機器: 実 zCoBo + 実 ECC(getEccServer ネイティブ)+ Rust ecc_bridge。
  - 保存先: データは ZFS `data/tpcdaq`。
  - 記録: ELOG(TPC 用)、InfluxDB(HV / LV)。
  - マニュアル: 日英 PDF が `reference/_manual/`。
- **実装の正本 = docs/SPEC_ja.md v1.47**。ゲート(main `22e9284`)は cargo 1078 passed / 0 failed / 1 ignored、UI 393 passed / 10 skipped、root_sink の C++ テスト全 passed(make test / test-root)。適合 402(127 時点)。ELITPC は main `d960c44`(134 込み)で稼働(2026-10-14 反映、RUNBOOK §7.1 の手順で。run 77 を停止して入れ替え、新しい run は未開始)。
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
| 1 | 〜13:52 | gain スキャン完了 → 4 点の表(揺れの列つき)を 137 の形で記録(新チケット or 137 追記) | 待ち |
| 2 | 〜14:05 | 138 パルサー核 8 run(自動) | 待機中 |
| 3 | 14:05 | **D 較正表**: pedestal(`triggerMode: periodically`、2 min)と pulser(1 min)を 232 / 502 ns で取り、ELITPC の `~/tpcdaq-rs/target/release/calib pedestal|pulser <graw> --out` で per-ch 表 | 未 |
| 4 | 14:15 | **B 自己トリガ**: `triggerMode: onMultiplicity`、閾値 4 点(`globalThreshold` 1/2/4/8 — 単位は docs/PARAMS_KNOBS_ja.md で確認してから)、各 ≤ 4 min(`stop.max_s 240`)→ レート・電荷・ok 率を run 84(外部)と比較。scan.py で可 | 未 |
| 5 | 14:50 | まとめて報告。**E の設定を決める規則**: 飽和 ch を含むイベント < 2 % の中で最長 peaking とその gain(1 pC で 1014 が収まればそれ、無理なら 502) | 未 |
| 6 | 15:00 | **A 12.5 MHz ドリフト端 run** 開始: knobs `writingClockFrequency_MHz: 12.5`、`triggerDelay: 3800`(ルール: (窓 40,960 − 2,800 − 160)/10)、ほか現用。**帰宅後まで走らせたまま**(2–3 h)。16:20 以降も走っているのはこれだけ | 未 |
| 7 | 帰宅後 | A を stop → ドリフト端(t_max → v_drift = L / t_max、σ_L(z))の解析(小マクロ) → **E 一晩 α run**(25 MHz、5 の設定)を開始(開始前にユーザーに一言) | 未 |
| 8 | 明日 | C(部分読み出し閾値 / ZS を実飛跡で)、139、E の解析(エネルギー較正 C-5、130 の素材) | 未 |

守ること: HV / LV / ガスには触らない(web からは不可、方針)。run の開始・停止はこの表の範囲で了承済み(ユーザー「進めてください」)。各 run の設定は台帳の `run_start.params` に自動記録。反映は RUNBOOK §7.1。

## 開いているチケット

| # | 中身 | 状態 | 次の一手 |
|---|---|---|---|
| [138](138_pulser_response_kernels.md) | 整形器の応答核(パルサー、peaking 4 × 振幅 2)と「揺れ」の出どころ → 139 デコンボリューション試作(ユーザー「デコンボリューションが可能かも」) | 取得を gain スキャンの後に自動開始(14:00 頃) | 解析マクロ → 139 発注書 |
| [133](133_restart_detection_multi_asad.md) | 4 AsAd では再開検出後に来る他 AsAd の古いフレームで原事故が再発しうる(送出順が AsAd 毎なら標準経路)、多 CoBo のカウンタ | **Warsaw 前の必須**(mini では起きない) | 一次資料で AsAd のリングを流す順を確かめ → Fable 設計(時刻で世代を分ける) |
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

- ~~134~~ 完了 → archive(2026-10-14、SPEC v1.47)。
- ~~135~~ / ~~136~~ **完了 → archive(2026-10-14)**: パラメータスキャンの道具(`tools/scan/`)。試運転 = triggerDelay 5 点(run 78–82): 帯の位置は `first = 508 − delay/4` で厳密に線形、トリガ遅れ L ≈ 0.16 µs、**推奨 delay 1750–1850**(現用 1500 はトリガ前 5.4 µs を捨てている)、2000 は 15 % が先頭で切れる。~~次 = peakingTime~~ **[archive/137](archive/137_peaking_time_scan.md) 済(run 83–86)**: 電荷は 232 → 502 → 1014 ns で 1 → 2.3 → 5.2 倍(現用 232 は積分不足)、70 ns は ok 66 % で大損、1014 は 1/3 のイベントが飽和。**gain スキャン(502 ns × 120 fC / 240 fC / 1 pC、1014 × 1 pC)を 13:30 に開始(run 87〜)**。**ユーザー裁定(同日): 現用を 1750 に**(`elinp__e090134b`。25 MHz では 1500 でもお尻は切れないが、他のクロックにも使えるルールとして)。**ルール**: 帯の先頭をトリガの ≈ 2.8 µs 前に置く → `delay[10 ns] = (窓 [ns] − 2,800 − 160) / 10`(25 MHz: 1752 → 1750、12.5 MHz: 3800、100 MHz は窓 5.12 µs なので 1 µs 前 = 396 が限度)。**peakingTime スキャン(70 / 232 / 502 / 1014 ns、各 1,500 events)を 1750 で開始(12:58 EEST、run 83〜)**。
- ~~SPEC v1.46~~ **確定(2026-10-13、スポット Fable、`61d88d4`)**:
  - 対象: 110(取得の再開)、123(`/api/last-event`)、092(EOS 後の Heartbeat なし)、132(run/start の `params_id` / `notes`)、069 の出口(§13-2/3/6/8 を実測済みに)。
  - Fable が文案から直した点: HMP2020 は HO720 = FTDI VCP(ttyUSB0)、§6.3 の判定は実装の 4 条件、§10.5 の 404 の条件 など。
- [133](133_restart_detection_multi_asad.md): 取得の再開の判定(4 AsAd)。**2026-10-14 Fable レビューで格上げ**: 送出が AsAd 毎のリング順なら、再開検出後に来る他 AsAd の古いフレーム(idx 2057 など)が新 epoch の pending に入り、タイムアウト emit で高水位が古い値になって新しい取得が late で落ちる = 原事故の再発。idx でなく時刻で世代を分ける設計が要る。一次資料で送出順の確認から。

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
