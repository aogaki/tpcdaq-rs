# 160 — スキャン報告書(2026-10-14〜16、日本語 PDF)

**Status: IN PROGRESS**(2026-10-16 起票 Fable。v1 作成済み、ユーザーレビュー待ち)

## 目的
S1–S18(run 78–173)の結果を 1 本の報告書にまとめる。読み手 = 自分 + mini TPC メンバー(日本語)。
来月の Warsaw 訪問の資料にも流用(正式文書ではない)。形式 = PDF(LuaLaTeX、ltjsarticle)。

## 成果物
- `docs/reports/2026-10_scan/scan_report_2026-10.tex`(正)→ `scan_report_2026-10.pdf`(11 ページ)。
- 図: `fig/`(ELITPC `~/test/scan/{kernel2,deconv,resid,closure,drift}/` の PNG を 13 枚コピー、うち 8 枚を使用)+ pgfplots(S1 直線、S10 ゲイン則、データ量の棒)。
- ビルド: `cd docs/reports/2026-10_scan && lualatex scan_report_2026-10.tex`(2 回)。Mac の TeX Live 2026、HaranoAji フォント。

## 構成
要旨(5 点)→ 1 条件と方法(装置、指標の定義、索引)→ 2 電子回路(S1 / S2+S4+S8 / S3)→ 3 HV(S10 / S12 / 運用点 B と一晩 run)
→ 4 精度(S11 / S14)→ 5 読み出し・レート・ZS(S9+S15 / S17 / S5+S13+S18 / ビーム設計)→ 6 未解決 → 付録(所在、B の params)。
数字はすべて `docs/SCANS_ja.md` と archive の結果節から転記(新しい解析はしていない)。

## 結果
- v1(2026-10-16 13:00): 11 ページ、lualatex 2 回でエラー 0・overfull 0。全ページ目視で確認(表の収まり、図の位置)。
- v2(2026-10-16 午後): ユーザーレビュー 6 点を反映 — §1.1 線源 228Th・「1 点 = 1 run」、§1.3 用語と略語(FPN / IQR / robust σ / view / GET 系ほか)を新設、§3.1 抵抗鎖 200 MΩ は Traian の回路図説明どおりで V–I(1,770 V / 200 MΩ = 8.85 µA vs 実測 8.6–8.8)からも言える、§4.1「パルス幅を z 誤差に」は Aogaki の思いつきの検証と明記、§4.2「1 view」の定義、§5.3 ZS が効き得る唯一の処理 = 裾を使う波形処理(デコンボ / Unfolding)。加えて §5.4 をランダム到来(m = n/(1+nτ) の表、不感時間の実測 = 131 B-5)と 10 GbE の事実(RJ45 = PS MAC 1G、SFP+ = PL の 1G コア、FW 作り直しが要る)に書き換え。13 ページ、overfull 0。
- 未: 次のレビュー。英語版(Warsaw 用)は要望があれば別票。

## 実行できなかった操作
なし。
