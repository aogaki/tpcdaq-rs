# 041 — 統合デモ: UI/controller → 実 ECC → vcobo-daq → receiver → 保存二系統 + モニタ

**Status: COMPLETED**(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS。
証拠一式 = `reference/_spike/demo/out/`、起動レシピ = `reference/_spike/demo/`)

## 結果

- **A〜D 全て成立。run 3 本 + 非既定ポート 1 本 + 異常系 2 種。リポのコード変更ゼロ**
  (git status はセッション開始時と同一、reference/ 正本 0 ファイル変更、残留プロセスゼロ)。
  データ源 = **実 .graw**(109 フレーム / 30,108,684 B)+ 実 mini ジオメトリ。
- **B(run 一周)**: run/start → 実 ECC describe→…→start 全通過 → vcobo-daq → receiver →
  保存二系統 + モニタ。`run0001.root` = **TPCData 108 entries(P1 オラクル一致)**、
  monitor.root 9 ヒスト(ChargeU/V/W integral 7776/9936/9936 = 手計算一致)、
  **WS 実測でも同値**(依存ゼロの ws_probe.py で復号照合。0x10×144 / 0x11×72 / 0x02×147)。
  **graw はソースと `ctrl ++ AsAd0` 連結で sha256 一致**(12 B の frameType 7 制御フレームは
  SPEC v1.2 どおり ctrl/ に保全 — 実データで再確認)。
  **停止の実機正規経路が初観測**: EOS 不達 5 s → 強制 EOS → `forced_eos:true /
  eos_closed:true / ok:true / reason:"normal"`(ただし記録先は audit + REST 応答 —
  下記不具合①)。
- **A(listen-before-start)**: 非既定ポート 46105 で機械実証 — receiver の実 bind が
  links → 実 ECC → vcobo-daq の `connect("TCP","127.0.0.1:46105")` に到達(Arm の 7 s 後)。
- **C(連続 run)**: 3 本連続成功、実 ECC 13 遷移すべて `result: 0`。audit の
  `ecc_walk_back` = run2/3 で `["status→Ready","breakup→Prepared","reset→Described",
  "reset→Idle"]` — **036 の歩き戻しの実機級初検証**。2 本目 describe で vcobo-daq の
  destroy→create(魔法値再焼き込み)+ daqStop 冪等を実ログで確認。
- **D-1(vcobo 不在で run/start)**: HTTP 500 + 明瞭なエラー、0.04 s で健全失敗、
  `next_run` 巻き戻り(034-C どおり)、全コンポーネント Idle 復帰。ECC 着地は
  **`WHEN_DESCRIBE`**(039 予想の WHEN_CONFIGURE は getHwServer あり構成の話 —
  v1.1 構成改訂の当然の帰結)。
- **D-2(run 中 SIGKILL)**: OS の正常 FIN → 自然 EOF として run クローズ(62 entries
  finalize)、出力はソースの**厳密なバイト前置**。その後の run/stop は
  `ok:true / reason:"normal" / forced_eos:false` — **異常の痕跡が logbook にほぼ残らない**
  (下記③ = 最大の発見)。
- **所要実測**: run/start **7.0〜7.5 s**(ほぼ 100% 実 ECC 内部 sleep。我々のチェーンは
  < 5 ms)/ run/stop 5.7 s(eos_timeout 5 s が支配 — **033-E の受信静止検出で秒未満になる
  見込み**)。
- **発見(裁定は下記のとおり処理)**:
  ① run_stop に `forced_eos`/`eos_closed` が無い = **033-A 未実装の確認**(audit には有る)
  ② ECC のエラーフラグ(`WHEN_DESCRIBE` 等)が `/api/status` に出ない → **043 起票**
  ③ **実機経路では `forced_eos:false` が「stop 前にリンクが死んだ」印になる**
  (D-2 と B の対照で実証)→ **SPEC v1.14 §9.2 に注記、033 に織り込み指示**
  ④ docs R7 の記述を実測に更新 ⑤ vcobo-daq の SIGINT graceful 化 = 小粒フォローアップ。
- 実行環境・日付: macOS Darwin 25.5.0(arm64)、2026-08-15。

---

(以下、起票時の発注書)

**Status(起票時): READY**(040 完了(2026-08-15)により着手条件成立 — 即発注可)
**仕様の正**: [docs/VIRTUAL_ZCOBO_ja.md](../docs/VIRTUAL_ZCOBO_ja.md) v1.1 §5 /
SPEC v1.13 §1.3(run シーケンス・歩き戻し)/ §8(run 制御)/ §9(logbook)
**発注先想定**: implementer/**Opus**(実走オペレーション + 観測。コード変更は原則なし)

---

## 目的

仮想 zCoBo スタックで **run 一周を我々の本番経路で回す**:
controller(REST)→ ecc-bridge → **実 ECC** → **vcobo-daq** → receiver → decoder →
{graw-writer, root-sink, monitor}。「UI から見える経路は全て本物」の実証 = デモ改良の土台完成。

## やること

### A. デモ起動レシピ

- `reference/_spike/demo/` に起動スクリプト一式(実 ECC、vcobo-daq、我々のコンポーネント群、
  停止スクリプト)。028 付録 A のリプレイ経路レシピを土台に、graw_replay を vcobo-daq に
  差し替える形。controller 設定は `ecc_proxy = "Ecc:tcp -h 127.0.0.1 -p 46002"`、
  `config_id = "mini"`(3 相同値 — 042 の文字列形)。
- **listen-before-start の実確認**: receiver の実 bind ポートが DataLinkSet(links)として
  ecc-bridge → 実 ECC → vcobo-daq へ流れ、vcobo-daq が configure でそこへ接続すること。

### B. run 一周(単発)

- `POST /api/run/start` → **実 ECC の describe→prepare→configure→start が全部通り**、
  vcobo-daq のデータが receiver に流れ、graw-writer / root-sink が書くこと。
- `POST /api/run/stop` → §1.3 v1.12 の停止シーケンス(受信静止検出 → 強制 EOS)で閉じ、
  **logbook の `run_stop` が `ok:true` / `forced_eos:true` / `eos_closed:true`** であること
  (実機 TCP flow の正規経路 — 033 の意味論が仮想スタックでそのまま出るかの初観測)。
- 出力検証: graw が**ソース .graw とバイト一致**(実機命名)/ run.root が開ける /
  monitor WS にヒストが出る(ブラウザ確認はスクリーンショット等の記録で可)。

### C. 連続 run(2 本)

- run/start → stop → **run/start(2 本目)**が通ること = **実 ECC 相手の歩き戻し
  (breakup → reset → reset)の初の実機級検証**(036 はテストダブル相手だった)。
  2 本目の describe で vcobo-daq の create が再度走る(魔法値の再焼き込み含む)ことを確認。

### D. 異常系スポット(観測のみ、修正はスコープ外)

- vcobo-daq 不在で run/start → controller がどう見えるか(ECC は `WHEN_CONFIGURE` 健全着地の
  はず — 039 実測)。audit ログと UI 表示を記録。
- run 中に vcobo-daq を kill → receiver の silent stall 可視化(032)がどう出るか記録。

## 受け入れ

- 結果節に: ①run 一周の logbook 実物(run_start / run_stop 行)②graw バイト一致の実測
  ③連続 run 2 本の ECC 状態遷移ログ ④異常系 2 種の観測記録 ⑤起動レシピの場所と手順
  ⑥所要時間(run 開始レイテンシ ≈ 7 s + α の実測)。
- リポのコード変更なし(見つかった不具合は**修正せず記録** — 裁定・修正チケットは Fable)。
- `git status` クリーン(md とスクリプト以外)。reference/ 正本無変更。

## 非スコープ

- Run 制御 UI の disabled 解除(P4 チケット群 — 本ユニットの完了が前提条件)/
  不具合の修正(記録のみ)/ 24 h soak(031)。
