# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-10-13(棚卸し — 止まっていた 069 / 073 / 074 / 078 / 079 / 080 / 084 / 101 / 107 を処置して archive、~~092~~ を修正して archive([archive/092](archive/092_receiver_abandoned_after_eof.md) — EOS の後も Heartbeat を出し続けていた)、131・132 を起票)。**
2026-08-18〜10-13 の作業ログ(Wave 1〜3、ELITPC 移行、125–129)は [archive/CURRENT_2026-10-13_wave_log.md](archive/CURRENT_2026-10-13_wave_log.md)、
その前は [archive/CURRENT_2026-08-14_p2_p3wave.md](archive/CURRENT_2026-08-14_p2_p3wave.md)。

## いま(1 分で読める要約)

- **本番**: ELITPC(旧 puppex、Ubuntu 26.04)で mini eTPC を運用中(2026-10-08〜、[archive/107](archive/107_elitpc_host_setup.md))。
  - 機器: 実 zCoBo + 実 ECC(getEccServer ネイティブ)+ Rust ecc_bridge。
  - 保存先: データは ZFS `data/tpcdaq`。
  - 記録: ELOG(TPC 用)、InfluxDB(HV / LV)。
  - マニュアル: 日英 PDF が `reference/_manual/`。
- **実装の正本 = docs/SPEC_ja.md v1.45**。ゲート(main、092 の後)は cargo 1070 passed / 0 failed / 1 ignored。UI 392、適合 402(127 時点)。
- **できること**:
  - 保存二系統: graw はロスレス、root-sink は PEventTPC と raw ツリー。
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
- **注意**: mini の現用の取得は **100 MHz**(時間窓 ≈ 24 mm、|θ| > 10° の飛跡は欠ける)。25 MHz に戻すかは 131 C-1。

## 開いているチケット

| # | 中身 | 状態 | 次の一手 |
|---|---|---|---|
| [110](110_stale_frames_after_unclean_stop.md) | 取得を止めずにスタックを落とすと、次の run の頭に古いフレームが混ざり root-sink が late で捨てる | **Fable 裁定済み**(2026-10-13): 混入は防げない(新しく起動した ECC から zCoBo へ stop が届かない)→ root-sink の「取得の再開 = epoch」+ decoder のカウンタ、graw は残す | ユーザーの答え(eventId 重複の扱い・実機再現)→ レーン A(C++)/ B(Rust)を Sonnet で |
| [124](124_hv_lv_power_onoff_from_web.md) | HV / LV の ch ごとの ON/OFF を web から(SPEC v1.42) | ELITPC に反映済み。HV OFF・LV ON/OFF は実機で確認済み | **ユーザーが UI で HV ch7 を ON**(ランプアップ約 5 分)→ 結果節 → archive |
| [131](131_onsite_and_hardware.md) | 現地・機器・ELITPC 実機でやること(069 / 073 / 079 / 084 / 101 / 107 の残り) | READY | やれるときに 1 項目ずつ |
| [132](132_params_override_notice.md) | params の current.json が TOML の config_id を上書きしたことを run/start の応答と log に出す | READY(Sonnet) | 着手指示で |
| 130(未起票) | 複数飛跡の再構成(strip ごとの複数パルス、view ごとの直線探索、view 間の対応付け、N 本のフィット — 3α まで) | 前段の 126–129 は完了。**2026-10-13 ユーザー: 後回し** | 設計は Fable。データ = 128 の重ね合わせ + gas-pressure-test の反応生成(真値つき) |

## 解析バックログ(起票は着手するとき — 「詳細チケットは 1 フェーズ先まで」)

元は 080(プログラム票、archive)と 069 Phase 2.5。

- **run 条件 → v_drift、窓の深さ、窓端クリップ率**(080 B-4):
  - 圧力と HV から `v_drift = 0.7233 × E/p` を出し、run_start に書く。
  - f_sample と triggerDelay から、窓の深さ [mm] とトリガ前後の配分を出す。
  - monitor に窓端クリップ率のカウンタを出す。
  - 131 C-1(サンプリングを戻すか)と同時に決めるのがよい。
- **較正解析**(069 Phase 2.5 C-3〜C-5 / 080 D-4):
  - α run のデータから σ_T・σ_L・strip PRF・イオンテール平均波形・ドリフト端の v_drift・ADC/keV を出す。
  - 066 ハーネスの mini .dat 切替(069 0-5)を含む。
  - ドリフト端は 100 MHz の窓では見えない。
- **較正表の適用**(080 D-2): onMultiplicity 閾値の個別化を 078 の例外リストへ流す。ゲイン正規化は ROOT プロダクト側(生 graw は不変)。
- **イオンテール + Unfolding**(080 D-5): 較正解析の後。
- **macros 配布準備**(080 D-7): LICENSE、channel_map.csv の扱い、ビルド産物の .gitignore。macros はユーザーのもの — 判断はユーザー。
- **QA ヒスト id 20 番台**(074 の残り): 130 の後、130 で扱う。

## Fable 待ち(キュー)

Opus 主対話中に出た設計判断・SPEC 疑義・レビュー依頼をここに積み、Fable で**まとめて**確定する(運用ルールの正は CLAUDE.md「モデル使い分け」)。

1. ~~110 の設計~~ 裁定済み(2026-10-13、スポット Fable — 110 の md)。SPEC の文案(§1.3 / §5.3 / §6.3 / §7 / §9.2)は 110 の md にあり、2 と同じ回で確定する。
2. **SPEC 文案の確定(まとめて 1 回 = v1.46)**:
   - (a) **§10.5 `GET /api/last-event`(TODO/123、実装・実機済み)** — §10 の末尾に「### 10.5 解析向け HTTP(v1.42)」:
     monitor(`ws_listen` のポート)は WS に加えて **`GET /api/last-event`** を持つ。応答は、root-sink から最後に受けた built event(`event_publish_hz` で間引き済み)を JSON にしたもの:
     `{run, event_idx, complete, received_unix_ms, age_ms, fragments: [{cobo, asad, event_idx, event_time, frame_type, revision, read_offset, status, mult, window_out, last_cell, n_aget, n_ch, n_buckets, items_out_of_range, adc}]}`。
     - `adc[aget][ch][bucket]` は生 ADC(FPN 込み・減算なし、0x03 と同じ並び)、読まれていない位置は `-1`。範囲外の item は捨てて数える。
     - 受信前は 404 `{"error": "no event received yet"}`。`Access-Control-Allow-Origin: *`。GET のみ。JSON への変換は要求時だけ(モニタ系、取得系に影響しない)。
     - 実測は ≈ 545 kB / 40 ms(123 結果節)。
   - (b) **§13 を実測済みに(069 の出口)**:
     - 3 に「**実測済み(ELITPC 2026-10-09、TODO/107・109)**: periodic トリガでイベント間隔が『周期 + 2.931 ms』で一定 = zCoBo の 1 イベントの読み出し 2.931 ms(frameType 1、557,312 B、全 ch)、上限 ≈ 340 Hz。外部トリガで見えた約 1 s の不感時間は CAEN の NIM→TTL 変換器によるもので、GET ではない(104 / 106 の訂正節)。1 GbE では 182.5 Hz(101.7 MB/s)で TCP の背圧が先に効く」。
     - 6 に「**実測済み(ELITPC、TODO/109)**: ZFS に 182.5 Hz × 557,312 B = 101.7 MB/s を 60 s、全数・連番で書いた。素の書き込みは ZFS 226–328 MB/s / NVMe 809 MB/s。root_sink の持続処理は CPU 律速(workers 8 で ≈ 122 events/s)」。
     - 8 に「**実機済み(TODO/118–124)**: HiVolta は USB シリアル(by-id)で運用し、ポーリングのたびに開閉(119)。ON/OFF は REMOTE + 前面スイッチが最終許可(124)。HMP2020 は HO720(USB CDC)」。
     - 2 は「mini は 1GbE で 182 Hz まで全数」を追記。ELITPC×100 Hz の NIC 構成は Warsaw のまま。
   - (c) **§1.3 / §2.2 に 1 行(TODO/092)**: 「EOS を送った後の receiver はそのリンクに Heartbeat を出さない(下流は EOS で run を閉じ、graw-writer は PULL を放す — 117)」。
   - (d) **§8.1 の run/start 応答に `params_id` と `notes`(TODO/132、実装後)**。

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
