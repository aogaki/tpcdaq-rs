# 071 — 実験ログブック 5 年分(OCR 407 ページ)の全数調査

**Status: COMPLETED**(起票・実施 Fable 2026-08-18。素材 = `reference/logbook/page_001..407.md`、
ユーザーが 2026-08-18 に OCR 結果を配置。1.6 MB / 手書きログの OCR につき誤読あり前提)

## 目的

過去 5 年(2019〜2026 とみられる)の ELITPC/mini 運用ログから、プロジェクトが
必要としている以下を**ページ引用付きで**構造化抽出する:

1. **ガス条件の時系列**(ガス種・混合比・圧力・流量・温度)— **069 C-6 の入力**
   (D_L 理論値 → 066 の v_drift 縮退破り)
2. **HV 条件の時系列**(E_drift / U_drift、GEM 各段電圧、記載された v_drift 値)—
   docs/DRIFT_DIFFUSION_PRIMER_ja.md の実地例。初見サンプル(page_200)に
   E_drift 100→200 V/cm・U_drift 1960→3920 V・「Higher Edrift is preferred due to
   diffusion dependency」の記録あり
3. **GET/DAQ 設定の変遷**(サンプリング周波数・peaking・trigDelay・ZS の有無)—
   065 の config 系譜と突き合わせ
4. **較正・測定の記録**(pulser、線源、v_drift 実測、拡散実測、エネルギー較正)—
   069 Phase 2.5 の前例探し
5. **事故・トラブル**(放電/spark、ノイズ、CoBo/DAQ 再起動、HV trip)— 069 異常系の実地根拠
6. **run インデックス**(日付 ↔ run 名 ↔ 条件)— exp_data 2022/2026 との突き合わせ

## 方法

- Sonnet 調査エージェント 4 本(page 001–100 / 101–200 / 201–300 / 301–407)で全数読み。
  転記は**原文の数値を正確に**(OCR の疑わしい箇所は [OCR?] を付ける)。
- Fable が統合し、`reference/logbook/_survey/SURVEY.md`(統合レポート)を作成。
  C-6 に効く数値はメモリと 069 へ反映。

## 検収

- 全 407 ページがいずれかのレーンで読まれていること(ページ範囲の抜け無し)。
- 上記 6 分類の抽出結果(ページ引用付き)+ OCR 品質所見。

---

## 結果(2026-08-18)

- **実施**: Sonnet 調査エージェント 4 本(p.001–100 / 101–200 / 201–300 / 301–407)で
  全 407 ページを読み、6 分類 + OCR 所見をページ引用付きで抽出(抜け無し。
  各レーンの一次整理 = `reference/logbook/_survey/lane1..4_*.md`)。Fable が統合して
  **`reference/logbook/_survey/SURVEY.md` v1.0** を作成。テストは対象外(調査のみ、
  コード無変更 — リポゲートは 809eb29 時点の cargo 468 passed から無変更)。
  実行環境: macOS ローカル、2026-08-18。
- **最重要成果**: ①**HIGS 2026 の運転条件が確定**(CO₂ 130 mbar / working point #3 =
  255/310/1372V [3067V] / E_drift 70 V/cm / **v_drift 0.390 cm/µs** / 25 MHz / delay 1848
  → 現地 1748)②この v_drift により **066 の厳密縮退が一次資料で解消**:
  **σ_L = 6.0 [5.0,7.0] cells = 0.94 [0.78,1.09] mm**、066 実測の成長率 0.0461 cell²/cell →
  **D_L 換算 268 µm/√cm = 熱限界(269 µm/√cm @70 V/cm)と 1% 一致**(CO₂ の
  冷たいガス挙動の定量確認 — docs/DRIFT_DIFFUSION_PRIMER_ja.md の実地検証)。
  ③運転点マスタ表(config #1〜#5、Eγ↔圧力↔U_DRIFT↔v_drift)を回収。
  ④DAQ 設定の系譜(GET release / delay の年表)が 065 の config 調査と完全接続。
  ⑤069 向け運用知見(zCoBo 再起動手順 p.230、DAQ start/stop 順序 p.334、
  run 誤上書き事故 p.204、zCoBo v.4 の存在と SSH 不通)。
- **持ち帰り(要検証)**: GET_RELEASE「20190315_patched-v2」(p.030、draw2 frame hack /
  data Router hack)と我々の reference の同一性確認 / p.254·257–259 運転点表の原本目視
  (§1 導出の前提)/ OCR 数値を SPEC 昇格させる際の原本再確認ルール。
- 069 C-6 は「ガス条件の収集」から「**ログブック値の確認 + 当日条件の転記**」に軽量化
  (069 に反映済み)。メモリ = `logbook-survey-essentials`。
