# 077 — モニタタブに AsAd 電子回路座標 2D(272 ch × 512)を追加

**Status: READY**(2026-09-14 起票、Fable。**着手はユーザー指示待ち**。
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
