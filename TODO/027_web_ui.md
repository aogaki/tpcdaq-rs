# 027 — Web UI(Angular): モニタ + 波形 + ログブック + Run 制御レイアウト

**Status: OPEN(発注可)**
**仕様**: SPEC **v1.10** §11(スタック・描画・デザイン規律 — 本ユニットの核)/
§10.2/10.3(WS ワイヤ — TS 側デコーダ)/ §10.4(クロス言語適合 — **TS 側はここで実装**)/
§3.2(WS 9000 / REST 8080)/ §8.1(logbook REST)
**依存**: 026(monitor + WS — 実装済みワイヤの正。**申し送りは
[archive/026_monitor_ws.md](archive/026_monitor_ws.md) の結果節を必読**)
**発注先想定**: implementer/**Opus**(UI 構成の裁量が残る)

## 確定済みのユーザー決定(2026-08-13 — 変更不可)

- **Run 制御ボタン類は完成形レイアウトを置き、全部 disabled**(P4 の REST 配線は後日 —
  エンドポイントは §8.1 で確定済みなので、有効化はフラグ 1 つで済む作りにしてよい)。
- **モック関数・仮バックエンドを作らない**。開発・デモで動くのはリプレイ経路
  (graw_replay → receiver → decoder → root-sink → monitor → WS)のみ。

## やること

1. **`ui/` に Angular workspace**(最新安定。Angular Material。ECharts(ngx-echarts 可)。
   **JSROOT は monitor ページで遅延ロード** — 初期バンドルを太らせない(§11)。
   node_modules は .gitignore、lockfile はコミット)。`ng build` の出力を controller の
   `ui_dir`(016 実装済み)で配信できる構成 + README に手順 1 段落。開発時は
   `ng serve` + WS/REST の proxy 設定。
2. **WS クライアント + 本番 TS デコーダ**: 13 B ヘッダ + 0x02/0x03/0x10/0x11(§10.2 の
   バイトレイアウトそのまま。**2D は iy 外側 row-major** — monitor 側で転置済み)+
   JSON 4 種(§10.3。casing は SPEC 文言どおり: status 本体 = snake_case、
   monitorGaps/clients/wsDropped = camelCase)。接続先は **`/ws` に固定**(026 申し送り)。
   自動再接続(バックオフ)+ **staleness 表示**(status が 3 秒途絶 = root-sink または
   monitor 停止の可視化 — 026 申し送り: monitor は独自タイマを持たない)。
3. **ビュー**(§11):
   - **モニタ**: 9 ヒストを JSROOT `createHistogram` で(TH1/TH2 相当を WS スナップ
     ショットから構築、colz・log 切替・軸ズーム・stats box)。**1 Hz 更新でズーム状態を
     保持**(painter `updateObject` + redraw — §11 確認項目①。できない場合は理由を報告)。
     イベント表示は interval・**freeze(表示のみ — run Stop と視覚的に混同させない)**・
     **イベント ID(run / event_idx)常時表示**(R9)。
   - **波形ビュー**(R13): ECharts。面 / (cobo,asad) / AGET 単位の選択、重ね描き/グリッド、
     クライアント側間引き。表示中のみ `subscribe` で waveforms ON(帯域 — §10.3)。
   - **ログブック**(R11): `GET /api/logbook?since_seq=N` のタイムライン + コメント追記
     (author 入力欄つき、token 不要 — §8.1 v1.10)。
   - **Run 制御**: §8.1 の全操作の完成形レイアウト(token 取得 UI 込み)、**全 disabled**。
   - **Power**: タブだけ置く(P6 プレースホルダ)。
   - **status バー**(全ページ共通): state / run / events_built / saturation %(§5.2)/
     monitorGaps / wsDropped / clients / staleness。
4. **§10.4 適合(TS 側)**: `cargo run --bin ws_proto_sample`(026 実装済み)の出力を
   **本番 TS デコーダ**に読ませ既知値照合(float ε=1e-5、vitest)。
   `ui/run_ws_conformance.sh` で生成 → 検証を 1 本化(フィクスチャ非コミット =
   run_conformance.sh 方式)。TS 側独立レイアウトテスト(§10.4-4)も併置。
5. **デザイン規律**(§11): Atlassian Design 準拠、モニタは Grafana 風ダーク。
   JSROOT ダークモードの馴染みを確認(§11 確認項目② — 所見を報告)。
   JSROOT ライセンス最終確認(③ — MIT のはず。確認結果を報告に含める)。

## テスト・受け入れ

- TS: デコーダ単体(バイトオフセット・既知値)+ §10.4 適合スクリプト green。
  `ng build` 成功。`ng test`(karma/jest いずれか標準構成)green。
- Rust 側は**無変更・無影響**(`cargo test` 不変。Rust 側に変更が必要になったら
  実装せず報告して戻る)。
- **実表示確認**: リプレイ経路(mini 実 graw があれば実データ、なければ合成)を流して
  9 ヒスト + 波形 + status バーが動くこと。**スクリーンショットを報告に添付**
  (UI の見た目の自動 E2E は P3 E2E ユニットに送る)。
- ファイル所有権: `ui/` 全部(新規)+ ルート .gitignore(node_modules 等の行追加のみ)。
  他は触らない。発注書に無い設計分岐(レイアウトの大枠変更・依存の大物追加)に
  出会ったら実装せず報告して戻る。
