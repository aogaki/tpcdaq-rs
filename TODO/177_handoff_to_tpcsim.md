# 177 — tpcsim(旧 gas-pressure-test)へ渡すもの一式

**Status: IN PROGRESS**(2026-10-24 起票 Opus。ユーザー: tpcsim の `TODO/14_user_tasks.md`(Fable 起票)の宿題のうち tpcdaq-rs 側で引き取れるものを「進めてください」)

## 0. 背景
- tpcsim(`~/test/gas-pressure-test`、リポ名は `tpcsim` に決定 2026-10-24)の TODO/13・15〜17 が、ユーザー経由で tpcdaq-rs の材料を要求(176 §4b の「渡すもの」)。
- 作業は tpcdaq-rs 側でまとめ、**受け渡し場所 `~/test/tpcsim_handoff/`**(どちらのリポにも入れない = 実データ・内部情報を含む)に置く。tpcsim 側の取り込みは向こうの Fable。

## 1. 中身(14_user_tasks.md の節番号)
1. **§2 条件カードの下書き** `runs.csv`(run 1〜195): ログブックの run_start / run_stop と params(`/srv/tpcdaq/params/<id>.json`)と run_start.slow から機械的に。
   列ごとに値と根拠(記録 / 推定 / 不明)。圧力は 2026-10-24 の復元表(91 / 100 mbar、移行中、根拠つき)。死 ch は「なし(068: ペデスタルで要注意 7 ch はパルサーで正常)」。
2. **§3 応答の核**: ELITPC `~/test/scan/kernel2/run01{01,03–10}_kernel.{json,png}`(138、25 MHz、peaking 70 / 232 / 502 / 1014 × −300 / −100 mV、`mean_raw` / `mean_norm` = 平均波形)。運用点 B = run 107 / 108。
   あわせて `~/test/scan/calib/`(pedestal_232ns / 502ns、pulser の相対利得)。
3. **§3 空の窓**: ペデスタル run 99(232 ns)/ 100(502 ns)(各 150 事象、25 MHz、周期トリガ、full)の raw、最大レート試験 run 148–152(周期トリガ、502 ns、2,333–7,674 事象)のパス、
   12.5 MHz のヒット無し事象(run 180 / 186 / 190 のエントリ番号)。**12.5 MHz のペデスタル run は未取得**(取るならユーザーの合図で 1–2 分)。
4. **§3 データのパス**: run 175(²²⁸Th、106 万事象、91 mbar)の raw、パルサー run 179・187–192、**線源を外した後の run 180〜195**(= 線源抜きラン)。
5. **§3 最新の macros**: tpcdaq-rs の `macros/`(169 / 170 以降)の写し + どのコミットか。geometry .dat は内部資料なのでパスだけ。
6. **§3 tpcdaq-rs への依頼**(SAMPLING_MHZ / DRIFT_HV_V を引数に)→ **178** で実装。

## 2. 受け入れ
- `~/test/tpcsim_handoff/README.md` に索引(各ファイル / パスの意味・出どころ・注意)。runs.csv の列の定義と根拠の規則。
- tpcsim の `TODO/14_user_tasks.md` は向こうのファイルなので書き換えない(ユーザーに完了項目を報告)。

## 結果(2026-10-24、主対話 Opus、ELITPC は読み出しとファイル複製のみ)
- 受け渡し場所 `~/test/tpcsim_handoff/`(README.md に索引)。
- **runs.csv**(195 run、38 列): ログブック + params + slow から機械的に(生成スクリプト scratchpad `runcard.py` を ELITPC で標準入力から実行)。
  圧力の根拠: 記録 144 / 推定 47 / 不明 2(run 1–2)/ 移行中 2(179・180)。電子回路の設定: params 162、不明 33(初期、サンプリングだけ run ≤ 56 = 100 MHz と推定)。HV: slow 127、不明 68(10-12 より前)。
  kind: physics 123 / pulser 31 / periodic 13 / 不明 28。死 ch: なし(068)。線源: ≤ 178 = ²²⁸Th、≥ 179 = なし。
- **kernels/**: ELITPC `~/test/scan/kernel2/run01{01,03–10}_kernel.{json,png}`(138、`all.mean_raw` / `mean_norm` = 平均波形)。運用点 B = 107 / 108。12.5 MHz の核は未解析。
- **calib/**: pedestal_232ns / 502ns、pulser の相対利得(081 / 091)。
- **empty_windows/**: ペデスタル run 99 / 100(各 150 事象、17 MB)、12.5 MHz のヒット無し事象 23(run 180: 11、186: 7、190: 5 — 多くは振動でトリガ = 174、`burst174/pick_nohit.C` で同じ raw 形式に抜き出し)。
  **12.5 MHz のペデスタル run は未取得 → README で 1 本取ることを推奨**。周期トリガの窓 run 148–152 は ELITPC のパスのみ。
- **データのパス**(実在を確認): run 175 raw 122 GB(+ Mac の写し)、パルサー 179・187–192、線源抜き 180・185–194(181–184・195 は raw 無し / 取得中)、周期 148–152。
- 未: macros の写し(178 の完了後)。tpcsim の TODO/14 はユーザーに完了項目を報告(向こうのファイルは書き換えない)。
