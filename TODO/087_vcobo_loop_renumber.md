# 087 — vcobo `loop = true` で eventIdx / eventTime を付け替える(2 周目以降がビルダに捨てられる)

**Status: READY**(2026-09-22 起票 Fable。Sonnet 1 日、C++ tools/vcobo)

## 0. 事実(077 目視 第 2 弾で判明)
- `tools/vcobo/vcobo_link.hpp` のループは `cursor_ = 0` に戻して**同じバイト列**を再送する。eventIdx が 1 周目と同じなので root-sink の
  イベントビルダは 2 周目以降を **late fragments** として捨てる(demo 実測: events_built 108 / late_fragments 8866、frames 8974)。
  → monitor の 0x02/0x03 は 1 周目の 108 イベントで止まり、後から繋いだクライアントには何も届かない。demo の「ずっと流れる」用途が成立していない。

## 1. 発注
- ループで先頭に戻るたびに **eventIdx オフセット**(= セット内の最大 eventIdx + 1)× 周回数を、送出する各データフレームの共通 MFM ヘッダ
  eventIdx(offset 22..26、エンディアンはフレームの `little` フラグに従う — `vcobo_core.hpp::peek_event_idx` と同じ読み方)に加算して送る。
  eventTime(offset 16..22、6 バイト)にもセットの時間幅(最大 − 最小 eventTime + 1 フレーム分)× 周回数を加算して単調増加を保つ。
  制御フレーム(eventIdx を持たない)は無改変。
- 実装は `vcobo_core.hpp` に純関数 `patch_frame_header(uint8_t* frame, size_t len, uint32_t idx_offset, uint64_t time_offset)`(戻り値 bool、
  制御フレームは false)を置き、`vcobo_link.hpp` のホットパスは pending_ にコピーした**後に**そのコピーへ適用する(元バッファは不変、heap 確保なし)。
  `loop = false` のときは呼ばない(1 周目はバイト一致のまま = 既存テストが証拠)。
- テスト(`test_vcobo_core.cpp`): ①合成フレーム(little / big 両方)で eventIdx / eventTime の加算値と他バイトの不変を検査 ②制御フレームは無改変
  ③2 周分の送出列で eventIdx が単調増加し重複が無いこと(既存のループテストがあれば拡張)。`make -j` で green、件数を結果節に。
- 受け入れ(demo): `TPCDAQ_DEMO_LOOP=true` で run を 3 分回し、`/api/status` の events_built が周回に応じて増え、late_fragments が 0 のまま。
  monitor に後から繋いだ WS クライアントに 0x02/0x03 が届く(`reference/_spike/demo/ws_probe.py --seconds 20` で 0x02 が数えられる)。
- 注意: graw_writer が書く .graw も付け替え後のヘッダになる(demo 用途なので可。docs/VIRTUAL_ZCOBO_ja.md に 1 行追記)。

## 2. 参照
077 結果 第 2 弾、docs/VIRTUAL_ZCOBO_ja.md、SPEC §6.3(ビルダは順序に依存しない / late fragment)、tools/vcobo/vcobo_core.hpp(peek_event_idx)。
