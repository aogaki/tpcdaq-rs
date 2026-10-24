# 180 — ELITPC: UI のテンプレートを物理の本番 1 本だけに

**Status: COMPLETED**(2026-10-24、主対話 Opus。ユーザー「ちょっとテンプレートがおかしなことになってるように見えます。この際なんで、物理実験用の本番一つだけにしてください」「他のテンプレート群はどこかに移動させてください」)

## 0. 問題
- `GET /api/params/templates` が 26 本(150 の周期の写し per1–per1000、141 の hit 系、138 の pulser、VM の pulser、HIgS、multiplicity、2025 年の pedestals など)を返していた。
  テンプレートを替えると UI はそのファイルの古い値でフォームを埋め直す(111)ので、選び間違えると条件が丸ごと変わる。
- 周期は 179 でノブになり、per* の写しは要らなくなった。

## 1. 処置(コード変更なし、ELITPC のファイル操作と params の PUT だけ)
1. Idle を確認、`current.json` を `current.json.bak-2026-10-24-retire` に退避。
2. **current params を物理に**(当時の current は template `configure-pedestals.xcfg` = 退避対象): API で token を取り(operator `claude`、横取りなし)、
   run 194–195 の `elinp__0a776a9d.json` + `triggerPeriod_ms: 1000`(テンプレートの値)を PUT → id **`elinp__6cf41a72`** → release。audit に残る。
3. `configure-*.xcfg` のうち生成物(`__<hash>`)と `configure-elinp.xcfg` 以外の 25 本と、その `prepare-` の対 16 本(計 41 本)を
   **`/home/daq/ELITPC/GetSoftware_config/retired_templates_2026-10-24/`** へ `mv -n`(消していない)。生成物 `*__<hash>.xcfg`(過去の run の config id)・describe・hardwareDescription・CoboFormats・workspace は残置。
   一覧はディレクトリの直下だけを見る(`params_api.rs` の `read_dir`)ので、サブディレクトリは出ない。

## 結果
- `GET /api/params/templates` = `{"templates":["configure-elinp.xcfg"]}`。`GET /api/params` 200。
- 新しい current(`elinp__6cf41a72`)の生成 xcfg は run 194–195 の `configure-elinp__0a776a9d.xcfg` と **cmp で一致**(条件は物理の本番と同じ、id だけ新ノブの分変わる)。
- 影響(退避したテンプレートを参照するもの): ELITPC `~/test/scan/` の `pulser_kernel3.sh`、`mk_pulser_template.sh`、`mk_hit_template.sh`、`hit_seq.sh`、`base_hit.json`、`mr_*_per*.json` ほか(過去のスキャンの写し)。
  **パルサー run は `configure-elinp-pulser.xcfg` が要る**(テストモードの 2 要素がノブでない — メモリ pulser-runs-need-test-mode)→ そのときだけ戻す。
- 戻し方: `mv /home/daq/ELITPC/GetSoftware_config/retired_templates_2026-10-24/<name> /home/daq/ELITPC/GetSoftware_config/`。

## 追記(2026-10-24、ユーザー「では pulser 用だけ戻してください」)
- `configure-elinp-pulser.xcfg` を退避先から戻した(prepare の対は元から無い)。一覧 = `["configure-elinp-pulser.xcfg", "configure-elinp.xcfg"]`、抽出は unset / unsupported なし。`configure-elinp-hit-pulser.xcfg`(hit 版)は退避のまま。
