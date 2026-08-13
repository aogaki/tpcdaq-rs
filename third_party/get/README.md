# third_party/get — GET 由来コード(CeCILL)の隔離置き場

## 何が置いてあるか

GET software の ROOT 永続化クラス一式。root-sink(`tools/root_sink/`)が書き出す
イベント TTree(SPEC §6.4)は、このクラス群を `Branch("GDataFrame", ...)` に載せることで
**graw2root 互換**になる —— オフライン解析(TPCReco / CoBoFrameViewer)が
無改造で読める形が目的。

| ファイル | 役割 |
|---|---|
| `GDataFrame.{h,cpp}` | 1 CoBo フレーム = TTree の 1 エントリ。ヘッダ + チャンネル/サンプルの TClonesArray |
| `GFrameHeader.{h,cpp}` | フレームヘッダ(eventIdx / eventTime / coboIdx / asadIdx / mult / lastCell 等) |
| `GDataChannel.{h,cpp}` | (aget, chan) 1 本分。サンプルへの TRefArray |
| `GDataSample.{h,cpp}` | (bucket, ADC 値) 1 点 |
| `LinkDefGET.h` | ROOT 辞書生成(`rootcling`)用の pragma |
| `LICENSE` | CeCILL 本文(コピー元 `CoBoFrameViewer/COPYING`) |

## 出自

- **元**: GET software `20190315_patched` の `CoBoFrameViewer/src/root/`
  (`reference/20190315_patched/` — **実験で使用中の版と同一**)。
- **コピー日**: 2026-08-13。
- **改変**: **無し**(バイト一致。ヘッダのライセンス表記も原文のまま)。
  改変が必要になった場合は、まず改変しないで済む方法を探すこと。どうしても必要なら
  差分を本 README に明記する(CeCILL の改変表示義務)。

コピー元との一致は md5 で確認済み(2026-08-13):

```
GDataFrame.h    b95b8b5cf3c369767486915921d7eb6e
GDataFrame.cpp  415b4681b07bb90a3f9b10a1ce47234c
GFrameHeader.h  441d15affe7a98b616d8c4b52a492950
GFrameHeader.cpp 25f8a51b8ce962fed93ffc4b740087c7
GDataChannel.h  c4b0cd795bf1c3b24546479a19a6a0d2
GDataChannel.cpp 08e0b00989d0727e0aaef9a642c383e6
GDataSample.h   76c5e4016e8c7a9dfffccc0768aa0927
GDataSample.cpp 42e34d108ad49e5f0e29bbdcbeb5d68b
LinkDefGET.h    4f6b3035705bb494d1a27c4ed9e7f7e9
```

## ライセンス

**CeCILL**(© Commissariat à l'Énergie Atomique et aux Énergies Alternatives (CEA)、
Contributors: Patrick Sizun)。本文は `LICENSE`。

tpcdaq-rs 本体は別ライセンス(リポジトリ直下 `LICENSE`)。**混ぜないための隔離が
このディレクトリの存在理由**(CLAUDE.md「GET 由来コード(CeCILL)を持ち込む場合は
`third_party/` に隔離しライセンス表示」/ SPEC §6.6)。

## `reference/` との区別(重要)

- `reference/` は **.gitignore 済み・コミット絶対禁止**(調査用のローカル参照)。
- `third_party/get/` は **コミット対象**。ビルドに必要なので、リポジトリに入っていないと
  root-sink が組めない。「reference は入れない、third_party は入れる」——
  この 2 つを取り違えないこと。

## 使われ方

- ビルドは `tools/root_sink/Makefile` の中で完結する(`rootcling` で `LinkDefGET.h` から
  辞書を生成 → `root_sink` 本体と `test_recorder` にリンク)。
- **Rust 側には一切リンクしない**(SPEC §6.6: 境界は ZMQ のみ)。
- 名前空間は ROOT 6 では `GET::`(ROOT 5 では `get::`)—— `RVersion.h` の
  `ROOT_VERSION_CODE > 393216` で切り替わる。ROOT 6.36 を使う本プロジェクトは常に `GET::`。
