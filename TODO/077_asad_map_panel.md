# 077 — モニタタブに AsAd 電子回路座標 2D(272 ch × 512)を追加

**Status: MERGED**(2026-09-17 実装 Sonnet レーン → Fable レビュー → main へマージ `3a973f7`。**残 = demo 目視(受け入れ 2 項目目)**、
076 Phase 0 レーンが 46002 を使い終わってから Fable が実施 → COMPLETED/archive。2026-09-14 起票、Fable。
発端 = ユーザー: 「信号が時間窓に収まっているか」の確認はチップ毎の波形表示だと選択を変えている間に
次のイベントが来る → **UVW Event Display(0x02)がまさにそれで、同一タブに 3 面が並ぶので解決**。
「しいて言えば 272 ch × 512 の 2D が同じタブで同時に見えれば最高」)

## 目的

monitor タブの Event Display(U/V/W の 3 面、`0x02 Uvw` → JSROOT TH2)の**下に 4 枚目として**
AsAd 1 枚分の**電子回路座標の 2D**(y = `aget×68 + ch` の 272 行、x = 512 bucket、値 = 表示専用
ベースライン減算後の生 ADC)を並べる。ジオメトリを通さないので FPN・未マップ ch・dead ch も見える。
現行の波形ビュー(overlay / grid)は無変更。

## 発注内容(UI、Angular + JSROOT。モデル = Sonnet)

1. **データ**: `0x03 Waveforms`(AsAd 毎、`nAget × nCh × nBuckets` の u16 生 ADC)。monitor タブは
   表示中だけ `ws.setWaveforms(true)`(離脱で false)— 波形ビューと同じ帯域制御(§10.3)。
   (cobo,asad) 毎の最新は `waveform-select.ts` の `updateWaveformCache` を再利用(ELITPC = 4 AsAd)。
2. **描画 = 既存パスに乗せる(KISS)**: `root-histo.ts` の `PanelMessage` に `WaveformsMessage` を
   加え、`buildAsad(create, message, spec)` で TH2D を作る。x = time bucket(0..nBuckets)、
   y = channel index `aget×nCh + ch`(0..nAget×nCh)。値 = `adc − baseline(channel)`、baseline は
   **`stripBaseline`(先頭 25 cell 平均、負値保持 — 058 と同じ規則)**を channel 単位で適用。
   draw option・パレット・log 切替は Uvw パネルと同一(`panelDrawOption`)。`uvwPanelSpec` と対に
   `asadPanelSpec(cobo, asad)` を追加(タイトル `AsAd cobo0/asad0 — raw ADC (baseline-subtracted)`、
   y 軸タイトル `AGET×68 + ch`)。
3. **レイアウト**: monitor-view.html の Event Display 行の下に AsAd 行(mini = 1 枚、ELITPC = 4 枚を
   横並び。枚数はメッセージ由来 — 焼き込み禁止)。刻みは同じ `DisplayClock`(freeze / interval 共通)。
4. **イベント整合(silent 禁止)**: AsAd パネルのラベルに `event #N` を出し、UVW 側の表示イベントと
   番号が違うとき(0x03 が drop-oldest で落ちた等)は `≠ UVW` を明示する。
5. **テスト**(vitest、既存 192 に追加): `buildAsad` の純関数テスト —
   (a) 合成メッセージ(nAget=2, nCh=3, nBuckets=8 等の小さい形)で bin 位置と値(baseline 減算、
   負値保持)を手計算オラクルで照合 (b) nAget/nCh/nBuckets がメッセージ由来で決まる
   (c) イベント番号不一致の表示ロジック。
6. **SPEC**: §10.3 の「波形ビューを開いたクライアントだけが 0x03 を受ける」を「波形ビューまたは
   monitor タブ」に改訂(v1.29、Fable が入れる)。§5.4 に AsAd パネルの 1 行。

## 受け入れ

- `cd ui && npm test` 全 green(件数増)+ `npm run build`。
- reference/_spike/demo(`start_demo.sh`、mini)で monitor タブに 4 枚目が出て、UVW と同じ刻みで
  更新・freeze すること。`TPCDAQ_DEMO_PROFILE=elitpc` で 4 枚。パルサー実データ(2025 mini .graw の
  リプレイ)で 272 行すべてにパルスが見え、FPN 行(各 AGET の 11/22/45/56)が平坦に見えること
  (見た目の確認 = スクリーンショットを結果節に添える)。

## 見積もり

TS 150〜200 行 + テスト 100 行。Sonnet 半日。Fable レビューは完了時 1 回。

## 注意

- **canvas の ImageData 直描きはしない**: 既存の JSROOT TH2 経路で 272 × 512 = 139k bin は
  1 Hz 更新に十分軽い(Uvw の 3 面は既に 92/72 × 512 を毎刻み描いている)。描画系を増やさない。
- 帯域: monitor タブを開いたクライアントは 0x02(3 面 ≈ 262 KB/event)に加えて 0x03(278 KB/AsAd/event)
  を受ける。ローカル/LAN 前提で問題ないが、WS 送信キューは drop-oldest(§10.3)なので落ちた分は
  4 の整合表示で見える。
- FPN 行のガイド線は付けない(平坦な行として自然に見える)。必要なら後続。

## 結果(第 1 弾: 実装 + 単体、2026-09-17)

- 実装(Sonnet、worktree ブランチ `worktree-agent-a6bae87d89abaff69`、コミット `9393714`)→ Fable レビュー → main マージ `3a973f7`。
  変更は ui/ の 6 ファイル(+290/−10): `root-histo.ts`(`WaveformsMessage` を `PanelMessage` に、`asadPanelSpec` /
  `buildAsad`(channel-major、y = aget×nCh + ch、`stripBaseline` 再利用、負値保持)/ `asadEventMismatch`)、
  `jsroot-panel.ts`(`shape()` の `case 'waveforms'`)、`monitor-view.{ts,html,scss}`(`ws.setWaveforms(true/false)`、
  `updateWaveformCache` 再利用、`asadPanels` computed、`≠ UVW` バッジ、`.grid-asad` auto-fit)。
- テスト(main、Mac、2026-09-17): `npm test -- --watch=false` → **Test Files 14 passed / 1 skipped、Tests 201 passed /
  5 skipped(206)**(変更前 192 → +9: 発注書 5 の (a) bin 位置と値の手計算オラクル・負値保持、(b) nAget/nCh/nBuckets が
  メッセージ由来、(c) event# 一致 / 不一致 / UVW 未表示)。`npm run build` green(monitor-view lazy chunk 58.4 kB、
  Initial total 501.5 kB = 変更前と同一)。prettier 全 green。
- レビュー所見(Fable): 逸脱なし。y 軸タイトル `AGET×68 + ch` は固定文字列(68 = AGET のハード不変量、bin 計算は
  `message.nCh` 由来)— 許容。パネル順は (cobo, asad) 昇順。ラベル(event# / ≠ UVW)は DisplayClock の刻みで、描画は
  JsrootPanel の同じ刻みで進む(UVW と同じ流儀)。
- SPEC §5.4 / §10.3 / 履歴を **v1.30** で改訂済み(2026-09-17)。
- demo 目視: headless Chrome の自動撮影はデータ到着前に閉じてしまい判定不能(monitor ログには monitor タブからの
  `subscribe … waveforms: true` が出ており購読配線は実証)。**2026-09-17 10:40 から demo スタック(mini、2 Hz ループ、
  1 時間 run)を立てたままにしてユーザーの目視を依頼** → 結果を追記して archive。
