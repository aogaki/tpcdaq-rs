# 111 — Parameters UI の初期値をテンプレート xcfg から読む(mini で 25 MHz に化ける穴を塞ぐ)

**Status: READY**(2026-10-09 起票 Opus。**着手はユーザー指示待ち** — ユーザー「着手はせずにチケットだけ」)

## 0. 問題(1 分で)

- 078 の Parameters UI(A/B/C 完了、SPEC v1.34 §11)は、「Load defaults from template」で**テンプレート名だけ**入れ、ノブの初期値は
  `ui/src/app/params/params-view.ts` の `DEFAULT_KNOBS`(= HIgS の configure-physics: **25 MHz / triggerDelay 1748** / 120 fC / 232 ns …)を直書きで使う
  (078 §11「テンプレートは読まない」)。
- mini の現用テンプレート `configure-elinp.xcfg`(= VM の `delay_test.xcfg`、**100 MHz / triggerDelay 320** — 101 追記 2、107)に対して
  そのまま保存すると、render がノブ値でテンプレートを上書きする → **100 MHz → 25 MHz、320 → 1748 に化けた xcfg ができる**。
- 現状の回避: 運用マニュアル(Claude Doc)で「Parameters 画面は使わない」と明記。設定の切替は daq.toml の `config_id` を手で書き換えて再起動。

## 1. やること

1. **抽出(render の逆)**: `src/params/` に「テンプレート xcfg → `Params`(knobs + exceptions)」を足す。各ノブの xcfg 上の位置は既存の
   `knob_path` / `KnobSpec` を使う。CompoundConfig の継承(`id="*"` が既定、個別 id が上書き、**粗い例外は一様値より優先されない** — SPEC v1.32 §3.3)
   に従い、一様値 + 例外に分ける。テンプレートに無いノブは「未設定」を明示(黙って既定値を入れない — CLAUDE.md)。
2. **REST**: `GET /api/params/defaults?template=<name>`(`ecc_config_dir` 配下のファイルだけ、パス外は 400)→ 抽出結果の JSON。
3. **UI**: 「Load defaults from template」で 2. を呼び、フォームを埋める。`DEFAULT_KNOBS` の直書きはやめる(テンプレート無しの空フォームは「未設定」表示)。
4. **(任意)ノブ追加**: `triggerPeriod`(periodic トリガの周期、ms。107 / 109 の periodic 試験で使用)。
5. **実機確認(078 §5-3 を兼ねる)**: ELITPC で elinp テンプレートから抽出 → 無変更で保存 → 生成された `configure-<id>.xcfg` が
   `configure-elinp.xcfg` と**バイト一致** → その id で短い run(periodic か線源)を 1 本。zCoBo は ELITPC に直結済みなので遠隔で可。

6. **運用マニュアル(Claude Doc、日英)の更新**: ユーザー方針(2026-10-09)「マニュアルはパラメーターができてから配布」。
   完了時に「Parameters 画面は使わない」(取得設定・やってはいけないこと の 2 か所、日英)を使い方の説明に置き換え、PDF を作り直す。

## 2. テスト(TDD、オラクル)

- 実テンプレート 3 種で**往復バイト一致**: 抽出 → render → 元の xcfg と同一。
  対象 = mini `delay_test.xcfg`(md5 0845d6c4、ELITPC `~/ELITPC/vm_configs_2026-10-08/`)、HIgS `configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg`
  (reference/config、md5 1d7b8854)、VM のパルサー `configure-pulser.xcfg`(md5 86a8ab10)。実ファイルはリポに入れない → 環境変数パスの任意回帰 +
  合成フィクスチャ(`tests/fixtures/params/`)で例外ありのケース。
- 078 の CompoundConfig オラクル(`tools/ccfg_resolve`)で、抽出した値が ECC の解決結果と全インスタンス一致。

## 2.1 手元の実テンプレート(2026-10-09 配置、gitignore)
`reference/_vm_configs_2026-10-08/` = VM の `TPCConfigFiles/` 17 本 + `delay_test.xcfg`(md5 0845d6c4)+ VM / GDED のパルサー
(`share_configure-pulser.xcfg` = 86a8ab10、`gded_configure-pulser.xcfg` = e1043496)。HIgS の physics は `reference/config/`。
ELITPC 上の同じもの = `~/ELITPC/vm_configs_2026-10-08/` と `~/ELITPC/GetSoftware_config/configure-elinp.xcfg`。

## 3. 見積もり(2026-10-09 にユーザーへ提示)

| 作業 | 担当 | 目安 |
|---|---|---|
| 1. 抽出 + 2. の往復テスト | Opus | 1〜1.5 日 |
| 2. REST + 3. UI | Sonnet | 0.5〜1 日 |
| 4. triggerPeriod ノブ(任意) | Sonnet | 0.5 日 |
| 5. 実機確認 | Opus(主対話) | 1 時間 |
| SPEC §3.3 / §11 反映 + 完了時レビュー | Fable | Fable キューでまとめて |

応急処置(DEFAULT_KNOBS を mini の値に差し替える、15 分)は ELITPC(HIgS)側で逆の事故を起こすので採らない。

## 4. 参考: UI に出しているのは「必要そうなものだけ」

- ノブは 11 個(triggerDelay / writingClockFrequency_MHz / triggerMode / globalThreshold / peakingTime_ns / readout / lsbThreshold / gain /
  triggerInhibition / zeroSuppression / pulser)。xcfg の残り(delay_test で値を持つ要素 ≈ 284、インスタンス展開で 4,423)は**テンプレートから無変更で引き継ぐ**。
- 特定の AsAd / AGET / ch だけ変える値は「Exceptions (N)」(折りたたみ、件数バッジ + ログブックで常に見える)— 078 §0。
- 構成・識別子系(AsAd `isActive`、readoutDepth 等)はノブにせず、テンプレートの選択が運ぶ(078 §2)。
