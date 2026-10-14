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
- **実装の正本 = docs/SPEC_ja.md v1.46**。ゲート(main `22e9284`)は cargo 1078 passed / 0 failed / 1 ignored、UI 393 passed / 10 skipped、root_sink の C++ テスト全 passed(make test / test-root)。適合 402(127 時点)。ELITPC は main `22e9284` で稼働。
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
- **取得条件**: mini の現用は **25 MHz / triggerDelay 1500**(params `elinp__dcdaf9ca`、2026-10-11 の run 57 から — ユーザーが Parameters で切り替え。それ以前の α run は 100 MHz で、時間窓 ≈ 24 mm)。

## 開いているチケット

| # | 中身 | 状態 | 次の一手 |
|---|---|---|---|
| [134](134_restart_time_gap_threshold.md) | 再開判定に時刻後退幅の閾値(偽陽性で新イベントを捨てる穴) | 実装中(Sonnet、2026-10-14) | マージ → SPEC §2.3 / §6.3 に 1 行 |
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
