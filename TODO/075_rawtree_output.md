# 075 — root-sink raw ツリー出力(macros 解析チェーン直結)

**Status: READY**(裁定 = SPEC v1.26 §6.7(2026-09-03 Fable)。**着手はユーザー指示待ち**)

**正 = SPEC §6.7。契約の現物 = [macros/graw2root.C](../macros/graw2root.C)**(読んで一致させる。
下流 = macros/analyzeUVW.C が `SetBranchAddress` する branch 名・型が凍結対象)。

## 目的

mini eTPC のスタンドアロン解析チェーン(macros/)を**下流無変更**で DAQ に直結する。
root-sink が run 毎に `run{run:04}_raw.root` を書けば、graw2root.C(変換ステップ)は不要になる。

## 発注内容

1. **設定**: root_sink に `--raw-tree` フラグ(既定 off)。TOML 側の対応キーも(§3 の流儀)。
2. **書き出し**(tools/root_sink、C++17):
   - TTree `raw`、branch = `eventId/i`(builder event_idx)、`eventTime/l`(先頭フラグメントの
     48-bit タイムスタンプ)、`adc[...]/S`(リーフリストは観測 AsAd 数から実行時生成 —
     `adc[4×nAsad][68][512]/S`。mini = 1 AsAd で graw2root.C と完全一致)。
   - 生 ADC・FPN 込み・減算なし。incomplete は届いた分 fill、欠損 0。
   - エントリ順 = emit 順(event_idx 昇順、§6.3)。
   - **単一ライタ**(worker 分割 _w{k} に載せない — 下流が単一ファイル前提)。
     PEventTPC recorder スレッドとロック共有しないこと(§5.1 の条件と同じ流儀)。
   - 命名・ライフサイクル: `run{run:04}_raw.root`、inprogress→rename はイベント TTree と同一
     (§6.5 表)。0 イベント run はファイルを作らない(§6.5)。
   - 書き込み失敗 = Error + カウント(silent 禁止)。
3. **テスト**(受け入れ = SPEC §6.7):
   - (a) 合成フィクスチャ: 既存 recorder テストの流儀で内容一致ユニットテスト
     (eventId / eventTime / adc、incomplete 含む)。
   - (b) **実データオラクル**: graw_replay で実 mini .graw(ローカル、環境変数パス)を流し、
     root-sink の raw.root と `root -l -b -q 'graw2root.C(...)'` の出力を**全エントリ比較**
     (比較スクリプト同梱 — ROOT マクロ可)。エントリ数・eventId 列・adc 全要素一致。
   - (c) §12-8 変形: raw on/off で PEventTPC 書き出しスループット不変(±5%)。
4. **ゲート**: 既存 root_sink 全スイート green 維持 + cargo 側無影響。

## 注意(監査で判明した契約の細部)

- graw2root.C は frameType 1(full readout、itemSize 4)専用。本ユニットは **built event
  由来なので frameType 非依存**(decoder が吸収)— compact run でも raw tree は書ける
  (これは graw2root.C に対する改善点。テスト (b) のオラクル照合は frameType 1 run で行う)。
- eventTime はイベント先頭フラグメント基準。mini(1 AsAd)では graw2root.C と自明に一致。
- macros/ 側は**一切変更しない**(graw2root.C はオラクルとして凍結)。

## モデル配分

implementer **Opus**(recorder 統合・スレッド分離の工学判断が残る)。
比較スクリプト(3b)のみ Sonnet 切り出し可。
