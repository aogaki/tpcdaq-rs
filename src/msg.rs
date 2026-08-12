//! ZMQ メッセージ核(Message/Batch/Fragment、直列化)。SPEC §2。
//!
//! # ワイヤ表現
//!
//! データ面は MessagePack の **compact(positional)** 形式(`rmp_serde::to_vec`)。
//! フィールド名はワイヤに乗らず、構造体は宣言順の配列になる。delila-rs と同一方式で、
//! C++(root-sink)側は delila の MessagePack リーダを流用してデコードする。
//!
//! - [`Message`] は enum なので fixmap(1) `{variant 名: ペイロード}`
//! - [`Batch`] は positional array(5)
//! - [`Fragment`] は positional array(12)
//! - バイト列(生フレーム・items)は `serde_bytes` 経由で msgpack の **bin** 型。
//!   素の `Vec<u8>` は「u8 の配列」に落ちて約 2 倍に膨らむため必ず [`serde_bytes::ByteBuf`] を使う。
//!
//! フィールド順序と型タグの正本は [`schema`] の定数表で、実構造体を直列化して
//! 表と突き合わせるテストが漂流を検出する(SPEC §2.5)。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

// ---------------------------------------------------------------------
// エラー
// ---------------------------------------------------------------------

/// メッセージの符号化・復号・パックで起きうる失敗。
///
/// 範囲外の値を silent に切り詰めないためのエラー型でもある(CLAUDE.md)。
#[derive(Debug, thiserror::Error)]
pub enum MsgError {
    #[error("msgpack encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),

    #[error("msgpack decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),

    /// item の各フィールドがビット幅に収まらない(SPEC §2.4)。
    #[error("item field {field}={value} exceeds max {max}")]
    ItemFieldRange {
        field: &'static str,
        value: u32,
        max: u32,
    },

    /// items バイト列が u32 の整数倍でない。
    #[error("items byte length {0} is not a multiple of 4")]
    ItemsMisaligned(usize),

    /// 長さ前置ストリームが途中で切れている。
    #[error("length-prefixed stream truncated at byte offset {0}")]
    TruncatedStream(usize),

    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------
// エンベロープ(SPEC §2.2)
// ---------------------------------------------------------------------

/// receiver → graw-writer / decoder のペイロード(SPEC §2.3)。
/// 1 要素 = MFM フレーム 1 個のバイト列そのもの(フレーム境界 = 要素境界)。
pub type RawFrames = Vec<ByteBuf>;

/// decoder → root-sink のペイロード(SPEC §2.3)。
pub type Fragments = Vec<Fragment>;

/// バッチのエンベロープ(SPEC §2.2)。positional array(5)。
///
/// `run_number` を全バッチに載せるのは delila-rs からの意図的差分(SPEC §14-3)。
/// コンシューマ側の run 番号 latch を不要にし、stale/消失 EOS 事故の芽を潰す。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch<P> {
    /// 送り手の一意 ID(SPEC §3.2: receiver = cobo_id、decoder = 100、psu = 200)。
    pub source_id: u32,
    /// この run の番号。全バッチに必ず載る。
    pub run_number: u32,
    /// ソース毎の単調増加番号。run 開始で 0 リセット。ロスレスリンクでは受信側が連続性を検証する。
    pub sequence_number: u64,
    /// 送信時の unix ns(レイテンシ計測用)。
    pub created_ns: u64,
    /// リンク別ペイロード(SPEC §2.3)。
    pub payload: P,
}

/// ZMQ 1 メッセージ = この enum 1 個(トピックフレームなし、SPEC §2.1/§2.2)。
///
/// 型パラメタ `P` はペイロードの型付けだけに使われ、ワイヤには現れない。
/// そのため `EndOfStream` / `Heartbeat` はどの `P` で符号化しても同じバイト列になる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message<P> {
    /// データバッチ。**間引いてよいのはこれだけ**([`Message::is_droppable`])。
    Data(Batch<P>),
    /// ソースの終端。in-band バリアであり、**いかなる間引き・破棄規則からも除外**(SPEC §2.2)。
    EndOfStream { source_id: u32, run_number: u32 },
    /// アイドル時 1 Hz の死活信号。
    Heartbeat {
        source_id: u32,
        run_number: u32,
        counter: u64,
    },
}

impl<P> Message<P> {
    /// 過負荷時に落としてよいメッセージか(SPEC §2.2)。
    ///
    /// モニタ系の間引き器は必ずこの分類を通す。`Data` だけが `true` で、
    /// `EndOfStream`(run の終端バリア)と `Heartbeat`(死活)は決して落とさない。
    /// なお落としたときは silent にせずカウントして可視化すること(CLAUDE.md)。
    pub fn is_droppable(&self) -> bool {
        matches!(self, Message::Data(_))
    }
}

impl<P: Serialize> Message<P> {
    /// MessagePack(compact/positional)へ符号化する。
    pub fn to_msgpack(&self) -> Result<Vec<u8>, MsgError> {
        Ok(rmp_serde::to_vec(self)?)
    }
}

impl<P: DeserializeOwned> Message<P> {
    /// MessagePack から復号する。
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self, MsgError> {
        Ok(rmp_serde::from_slice(bytes)?)
    }
}

// ---------------------------------------------------------------------
// Fragment(SPEC §2.4)
// ---------------------------------------------------------------------

/// デコード済みの 1 CoBo フレーム(= 1 AsAd 分)。GDataFrame を再構成できるヘッダを全部運ぶ。
/// positional array(12)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fragment {
    pub event_idx: u32,
    /// 48 bit 有効。
    pub event_time: u64,
    pub cobo: u8,
    pub asad: u8,
    /// 1 = 2018 形式、2 = 2025 compact。
    pub frame_type: u8,
    pub revision: u8,
    pub read_offset: u16,
    pub status: u8,
    pub mult: [u16; 4],
    pub window_out: u32,
    pub last_cell: [u16; 4],
    /// パック済み item の u32 LE 連結([`pack_item`] / [`items_to_bytes`])。
    pub items: ByteBuf,
}

// ---------------------------------------------------------------------
// item の 32bit パック(SPEC §2.4)
// ---------------------------------------------------------------------

/// aget の最大値(bits[31:30])。
pub const ITEM_AGET_MAX: u8 = 3;
/// chan の最大値(bits[29:23]、7 bit)。raw 0–67 の運用値はこの幅に収まる。
pub const ITEM_CHAN_MAX: u8 = 127;
/// bucket の最大値(bits[22:14]、9 bit)。
pub const ITEM_BUCKET_MAX: u16 = 511;
/// ADC の最大値(bits[11:0]、12 bit)。
pub const ITEM_ADC_MAX: u16 = 4095;
/// 予約ビット bits[13:12]。パック時は常に 0。
pub const ITEM_RESERVED_MASK: u32 = 0x0000_3000;

const ITEM_AGET_SHIFT: u32 = 30;
const ITEM_CHAN_SHIFT: u32 = 23;
const ITEM_BUCKET_SHIFT: u32 = 14;

/// アンパックした 1 サンプル。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub aget: u8,
    /// GRAW の raw チャンネル 0–67(FPN 込み。ジオメトリ変換はここではしない)。
    pub chan: u8,
    pub bucket: u16,
    /// 生 ADC(減算なし)。
    pub adc: u16,
}

fn check_field(field: &'static str, value: u32, max: u32) -> Result<u32, MsgError> {
    if value > max {
        return Err(MsgError::ItemFieldRange { field, value, max });
    }
    Ok(value)
}

/// 1 サンプルを u32 へパックする(SPEC §2.4)。
///
/// ビット割り: `[31:30]=aget` `[29:23]=chan(raw)` `[22:14]=bucket` `[13:12]=予約 0` `[11:0]=ADC`。
/// 範囲外は切り詰めずに [`MsgError::ItemFieldRange`](CLAUDE.md「silent failure を作らない」)。
pub fn pack_item(aget: u8, chan: u8, bucket: u16, adc: u16) -> Result<u32, MsgError> {
    let aget = check_field("aget", u32::from(aget), u32::from(ITEM_AGET_MAX))?;
    let chan = check_field("chan", u32::from(chan), u32::from(ITEM_CHAN_MAX))?;
    let bucket = check_field("bucket", u32::from(bucket), u32::from(ITEM_BUCKET_MAX))?;
    let adc = check_field("adc", u32::from(adc), u32::from(ITEM_ADC_MAX))?;
    Ok((aget << ITEM_AGET_SHIFT) | (chan << ITEM_CHAN_SHIFT) | (bucket << ITEM_BUCKET_SHIFT) | adc)
}

/// パック済み u32 を展開する。予約ビットは無視する(前方互換)。
pub fn unpack_item(word: u32) -> Item {
    Item {
        aget: ((word >> ITEM_AGET_SHIFT) & u32::from(ITEM_AGET_MAX)) as u8,
        chan: ((word >> ITEM_CHAN_SHIFT) & u32::from(ITEM_CHAN_MAX)) as u8,
        bucket: ((word >> ITEM_BUCKET_SHIFT) & u32::from(ITEM_BUCKET_MAX)) as u16,
        adc: (word & u32::from(ITEM_ADC_MAX)) as u16,
    }
}

/// パック済み item 列を [`Fragment::items`] のバイト列(u32 LE 連結)にする。
pub fn items_to_bytes(items: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(items.len() * 4);
    for item in items {
        out.extend_from_slice(&item.to_le_bytes());
    }
    out
}

/// [`Fragment::items`] のバイト列を u32 列へ戻す。長さが 4 の倍数でなければエラー
/// (境界ずれを silent に通さない)。
pub fn items_from_bytes(bytes: &[u8]) -> Result<Vec<u32>, MsgError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(MsgError::ItemsMisaligned(bytes.len()));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

// ---------------------------------------------------------------------
// スキーマ漂流ガード(SPEC §2.5)
// ---------------------------------------------------------------------

/// ワイヤ表現(フィールド順序と型タグ)の正本表。
///
/// delila-rs `src/common/delila_schema.rs` の方式。ここに宣言した順序・型と、実際に
/// 直列化されたバイト列が一致することをテストが検証するので、構造体だけを直したり
/// 表だけを直したりするとテストが落ちる。C++(root-sink)側デコーダはこの表を
/// 人間可読な契約として参照する。
///
/// # 型タグの文法
///
/// - `u8` / `u16` / `u32` / `u64` — MessagePack のスカラ(符号なし整数)
/// - `bin` — MessagePack の bin(バイト列。配列ではない)
/// - `[T]` — 可変長配列、`[T;N]` — 固定長配列
/// - `P` — [`Batch`] のペイロード。実型はリンク別(SPEC §2.3)で `[bin]` か `[Fragment]`
/// - それ以外は [`type_fields`] で解決できるレコード型名(= positional array)
pub mod schema {
    /// ワイヤ表現の版。どれかの型のレイアウトを変えたら上げる。
    pub const SCHEMA_VERSION: u32 = 1;

    /// [`Batch`](super::Batch) のペイロードを指す型タグ。
    pub const PAYLOAD_TAG: &str = "P";

    /// レコード 1 フィールドの定義(表示名 + 型タグ)。
    pub struct FieldDef {
        pub name: &'static str,
        pub tag: &'static str,
    }

    /// enum [`Message`](super::Message) の 1 バリアント(fixmap(1) の鍵と値の型)。
    pub struct VariantDef {
        pub name: &'static str,
        pub payload: &'static str,
    }

    const fn f(name: &'static str, tag: &'static str) -> FieldDef {
        FieldDef { name, tag }
    }

    const fn v(name: &'static str, payload: &'static str) -> VariantDef {
        VariantDef { name, payload }
    }

    /// `Message` のバリアント(宣言順 = ワイヤの鍵名)。
    pub const MESSAGE_VARIANTS: &[VariantDef] = &[
        v("Data", "Batch"),
        v("EndOfStream", "EndOfStream"),
        v("Heartbeat", "Heartbeat"),
    ];

    /// `Batch` のワイヤレイアウト(SPEC §2.2)。
    pub const BATCH: &[FieldDef] = &[
        f("source_id", "u32"),
        f("run_number", "u32"),
        f("sequence_number", "u64"),
        f("created_ns", "u64"),
        f("payload", PAYLOAD_TAG),
    ];

    /// `Message::EndOfStream` のワイヤレイアウト。
    pub const END_OF_STREAM: &[FieldDef] = &[f("source_id", "u32"), f("run_number", "u32")];

    /// `Message::Heartbeat` のワイヤレイアウト。
    pub const HEARTBEAT: &[FieldDef] = &[
        f("source_id", "u32"),
        f("run_number", "u32"),
        f("counter", "u64"),
    ];

    /// `Fragment` のワイヤレイアウト(SPEC §2.4)。
    pub const FRAGMENT: &[FieldDef] = &[
        f("event_idx", "u32"),
        f("event_time", "u64"),
        f("cobo", "u8"),
        f("asad", "u8"),
        f("frame_type", "u8"),
        f("revision", "u8"),
        f("read_offset", "u16"),
        f("status", "u8"),
        f("mult", "[u16;4]"),
        f("window_out", "u32"),
        f("last_cell", "[u16;4]"),
        f("items", "bin"),
    ];

    /// 型名からフィールド表を引く。
    pub fn type_fields(name: &str) -> Option<&'static [FieldDef]> {
        match name {
            "Batch" => Some(BATCH),
            "Fragment" => Some(FRAGMENT),
            "EndOfStream" => Some(END_OF_STREAM),
            "Heartbeat" => Some(HEARTBEAT),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------
// golden fixture(SPEC §10.4 方式)
// ---------------------------------------------------------------------

/// golden fixture に入れる既知値メッセージ(符号化済み)。
///
/// 内容は 4 通: RawFrames バッチ / Fragments バッチ / Heartbeat / EndOfStream。
/// 値は非対称に選んである(取り違えが目に見えるように)。
pub fn golden_messages() -> Result<Vec<Vec<u8>>, MsgError> {
    let raw: Message<RawFrames> = Message::Data(Batch {
        source_id: 0, // receiver = cobo_id 0(SPEC §3.2)
        run_number: 7,
        sequence_number: 0,
        created_ns: 1_755_000_000_123_456_789,
        payload: vec![
            ByteBuf::from(vec![0x08, 0x05, 0x00, 0x01, 0x02, 0x03]),
            ByteBuf::from(vec![0xff, 0x7f]),
        ],
    });

    let items = items_to_bytes(&[
        pack_item(0, 0, 0, 0)?,
        pack_item(2, 45, 300, 1234)?,
        pack_item(ITEM_AGET_MAX, ITEM_CHAN_MAX, ITEM_BUCKET_MAX, ITEM_ADC_MAX)?,
    ]);
    let fragments: Message<Fragments> = Message::Data(Batch {
        source_id: 100, // decoder(SPEC §3.2)
        run_number: 7,
        sequence_number: 1,
        created_ns: 1_755_000_000_223_456_789,
        payload: vec![Fragment {
            event_idx: 42,
            event_time: 0x0000_1234_5678_9abc,
            cobo: 0,
            asad: 1,
            frame_type: 2,
            revision: 5,
            read_offset: 136,
            status: 0,
            mult: [68, 0, 17, 3],
            window_out: 512,
            last_cell: [7, 9, 11, 13],
            items: ByteBuf::from(items),
        }],
    });

    let heartbeat: Message<RawFrames> = Message::Heartbeat {
        source_id: 0,
        run_number: 7,
        counter: 3,
    };
    let eos: Message<RawFrames> = Message::EndOfStream {
        source_id: 0,
        run_number: 7,
    };

    Ok(vec![
        raw.to_msgpack()?,
        fragments.to_msgpack()?,
        heartbeat.to_msgpack()?,
        eos.to_msgpack()?,
    ])
}

/// [`golden_messages`] を `u32 LE 長さ + ペイロード` の連結ストリームとして書き出す。
///
/// SPEC §10.4 の生成器と同じ枠組み。将来 C++(root-sink)側デコーダの照合に使う。
/// **生成物はコミットしない**(毎回再生成 — 陳腐化が構造的に起きない)。
pub fn write_golden_stream(path: &Path) -> Result<(), MsgError> {
    let io_err = |source| MsgError::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = std::fs::File::create(path).map_err(io_err)?;
    let mut out = std::io::BufWriter::new(file);
    for frame in golden_messages()? {
        let len = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        out.write_all(&len.to_le_bytes()).map_err(io_err)?;
        out.write_all(&frame).map_err(io_err)?;
    }
    out.flush().map_err(io_err)?;
    Ok(())
}

/// `u32 LE 長さ + ペイロード` の連結ストリームをフレームへ分解する(検証器側)。
pub fn split_length_prefixed(bytes: &[u8]) -> Result<Vec<&[u8]>, MsgError> {
    let mut frames = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes.len() - pos < 4 {
            return Err(MsgError::TruncatedStream(pos));
        }
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&bytes[pos..pos + 4]);
        let len = u32::from_le_bytes(len_bytes) as usize;
        pos += 4;
        if bytes.len() - pos < len {
            return Err(MsgError::TruncatedStream(pos));
        }
        frames.push(&bytes[pos..pos + len]);
        pos += len;
    }
    Ok(frames)
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::schema::{self, FieldDef};
    use super::*;
    use serde_bytes::ByteBuf;

    // -----------------------------------------------------------------
    // 最小 MessagePack スキマー(テスト専用)。
    //
    // serde_json::Value は bin 型を表現できない(バイト列を受けると失敗する)ため、
    // ワイヤ表現の検証は生マーカーを直接読む。これにより「Vec<u8> が bin ではなく
    // 配列にフォールバックした」という漂流も検出できる。
    // -----------------------------------------------------------------

    #[derive(Debug, PartialEq)]
    enum Mp<'a> {
        Nil,
        Uint(u64),
        Str(&'a str),
        Bin(&'a [u8]),
        Array(Vec<Mp<'a>>),
        Map(Vec<(Mp<'a>, Mp<'a>)>),
    }

    fn take<'a>(b: &'a [u8], p: &mut usize, n: usize) -> &'a [u8] {
        let s = &b[*p..*p + n];
        *p += n;
        s
    }

    fn be_uint(b: &[u8]) -> u64 {
        b.iter().fold(0u64, |acc, &x| (acc << 8) | u64::from(x))
    }

    fn read_array<'a>(b: &'a [u8], p: &mut usize, n: usize) -> Mp<'a> {
        Mp::Array((0..n).map(|_| read(b, p)).collect())
    }

    fn read_map<'a>(b: &'a [u8], p: &mut usize, n: usize) -> Mp<'a> {
        Mp::Map((0..n).map(|_| (read(b, p), read(b, p))).collect())
    }

    fn read<'a>(b: &'a [u8], p: &mut usize) -> Mp<'a> {
        let marker = b[*p];
        *p += 1;
        match marker {
            0x00..=0x7f => Mp::Uint(u64::from(marker)),
            0x80..=0x8f => read_map(b, p, usize::from(marker & 0x0f)),
            0x90..=0x9f => read_array(b, p, usize::from(marker & 0x0f)),
            0xa0..=0xbf => {
                let n = usize::from(marker & 0x1f);
                Mp::Str(std::str::from_utf8(take(b, p, n)).unwrap())
            }
            0xc0 => Mp::Nil,
            0xc4..=0xc6 => {
                let width = 1usize << (marker - 0xc4); // bin8/16/32
                let n = be_uint(take(b, p, width)) as usize;
                Mp::Bin(take(b, p, n))
            }
            0xcc..=0xcf => {
                let width = 1usize << (marker - 0xcc); // uint8/16/32/64
                Mp::Uint(be_uint(take(b, p, width)))
            }
            0xd9..=0xdb => {
                let width = 1usize << (marker - 0xd9); // str8/16/32
                let n = be_uint(take(b, p, width)) as usize;
                Mp::Str(std::str::from_utf8(take(b, p, n)).unwrap())
            }
            0xdc => {
                let n = be_uint(take(b, p, 2)) as usize;
                read_array(b, p, n)
            }
            0xdd => {
                let n = be_uint(take(b, p, 4)) as usize;
                read_array(b, p, n)
            }
            0xde => {
                let n = be_uint(take(b, p, 2)) as usize;
                read_map(b, p, n)
            }
            0xdf => {
                let n = be_uint(take(b, p, 4)) as usize;
                read_map(b, p, n)
            }
            other => panic!("unsupported msgpack marker 0x{other:02x}"),
        }
    }

    fn parse(bytes: &[u8]) -> Mp<'_> {
        let mut p = 0usize;
        let v = read(bytes, &mut p);
        assert_eq!(p, bytes.len(), "trailing bytes after top-level value");
        v
    }

    // -----------------------------------------------------------------
    // スキーマ表 ↔ ワイヤ表現の突き合わせ(漂流ガード、SPEC §2.5)
    // -----------------------------------------------------------------

    /// `tag` が示す型で `v` が符号化されているか検査する。`payload_tag` は
    /// `Batch<P>` の P に入る実型のタグ(リンク別 — SPEC §2.3)。
    fn check(tag: &str, payload_tag: &str, v: &Mp) -> Result<(), String> {
        let tag = if tag == schema::PAYLOAD_TAG {
            payload_tag
        } else {
            tag
        };

        // "[T;N]" 固定長配列 / "[T]" 可変長配列
        if let Some(inner) = tag.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            let (inner, fixed_len) = match inner.split_once(';') {
                Some((t, n)) => (t, Some(n.parse::<usize>().map_err(|e| e.to_string())?)),
                None => (inner, None),
            };
            let Mp::Array(items) = v else {
                return Err(format!("{tag}: expected msgpack array, got {v:?}"));
            };
            if let Some(n) = fixed_len {
                if items.len() != n {
                    return Err(format!("{tag}: expected {n} elements, got {}", items.len()));
                }
            }
            for item in items {
                check(inner, payload_tag, item)?;
            }
            return Ok(());
        }

        // スカラ
        let scalar_max = match tag {
            "u8" => Some(u64::from(u8::MAX)),
            "u16" => Some(u64::from(u16::MAX)),
            "u32" => Some(u64::from(u32::MAX)),
            "u64" => Some(u64::MAX),
            _ => None,
        };
        if let Some(max) = scalar_max {
            return match v {
                Mp::Uint(n) if *n <= max => Ok(()),
                _ => Err(format!("{tag}: expected unsigned int <= {max}, got {v:?}")),
            };
        }
        if tag == "bin" {
            return match v {
                Mp::Bin(_) => Ok(()),
                _ => Err(format!("bin: expected msgpack bin, got {v:?}")),
            };
        }

        // 名前付きレコード(positional array)
        let fields = schema::type_fields(tag).ok_or_else(|| format!("unknown type tag {tag}"))?;
        check_record(fields, payload_tag, v)
    }

    fn check_record(fields: &[FieldDef], payload_tag: &str, v: &Mp) -> Result<(), String> {
        let Mp::Array(items) = v else {
            return Err(format!("record: expected positional array, got {v:?}"));
        };
        if items.len() != fields.len() {
            return Err(format!(
                "record: schema has {} fields, wire has {}",
                fields.len(),
                items.len()
            ));
        }
        for (field, item) in fields.iter().zip(items) {
            check(field.tag, payload_tag, item)
                .map_err(|e| format!("field `{}`: {e}", field.name))?;
        }
        Ok(())
    }

    /// enum のワイヤ表現 fixmap(1) を剥がして (variant 名, ペイロード) を返す。
    fn unwrap_variant<'a>(v: &'a Mp<'a>) -> (&'a str, &'a Mp<'a>) {
        let Mp::Map(entries) = v else {
            panic!("Message must be a msgpack map, got {v:?}");
        };
        assert_eq!(entries.len(), 1, "Message must be fixmap(1)");
        let (key, payload) = &entries[0];
        let Mp::Str(name) = key else {
            panic!("variant key must be a string, got {key:?}");
        };
        (name, payload)
    }

    // -----------------------------------------------------------------
    // 既知値(テストの出典 — 実装側と独立に手で書く)
    // -----------------------------------------------------------------

    fn sample_fragment() -> Fragment {
        Fragment {
            event_idx: 42,
            event_time: 0x0000_1234_5678_9abc, // 48 bit 有効
            cobo: 1,
            asad: 3,
            frame_type: 2,
            revision: 5,
            read_offset: 136,
            status: 0,
            mult: [68, 0, 17, 3],
            window_out: 512,
            last_cell: [7, 9, 11, 13],
            items: ByteBuf::from(items_to_bytes(&[0x0000_0000, 0x96cb_04d2])),
        }
    }

    fn sample_raw_batch() -> Batch<RawFrames> {
        Batch {
            source_id: 1,
            run_number: 7,
            sequence_number: 300, // > 255 → uint16 マーカーになる値をあえて選ぶ
            created_ns: 1_755_000_000_123_456_789,
            payload: vec![
                ByteBuf::from(vec![0x08, 0x05, 0x00, 0x01, 0x02, 0x03]),
                ByteBuf::from(vec![0xff, 0x7f]), // 長さ非対称
            ],
        }
    }

    // -----------------------------------------------------------------
    // 直列化ラウンドトリップ
    // -----------------------------------------------------------------

    #[test]
    fn data_batch_of_raw_frames_roundtrips() {
        let msg = Message::Data(sample_raw_batch());
        let bytes = msg.to_msgpack().unwrap();
        let back: Message<RawFrames> = Message::from_msgpack(&bytes).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn data_batch_of_fragments_roundtrips() {
        let msg = Message::Data(Batch {
            source_id: 100, // decoder(SPEC §3.2)
            run_number: 7,
            sequence_number: 1,
            created_ns: 1_755_000_000_223_456_789,
            payload: vec![sample_fragment()],
        });
        let bytes = msg.to_msgpack().unwrap();
        let back: Message<Fragments> = Message::from_msgpack(&bytes).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn end_of_stream_and_heartbeat_roundtrip() {
        let eos: Message<RawFrames> = Message::EndOfStream {
            source_id: 1,
            run_number: 7,
        };
        let hb: Message<RawFrames> = Message::Heartbeat {
            source_id: 1,
            run_number: 7,
            counter: 3,
        };
        for msg in [eos, hb] {
            let bytes = msg.to_msgpack().unwrap();
            let back: Message<RawFrames> = Message::from_msgpack(&bytes).unwrap();
            assert_eq!(back, msg);
        }
    }

    /// EOS は payload 型に依存しない。decoder リンク(Fragments)で符号化したものを
    /// graw-writer リンク(RawFrames)側でそのまま読める = 型パラメタはワイヤに出ない。
    #[test]
    fn end_of_stream_is_payload_type_agnostic() {
        let eos: Message<Fragments> = Message::EndOfStream {
            source_id: 0,
            run_number: 12,
        };
        let bytes = eos.to_msgpack().unwrap();
        let back: Message<RawFrames> = Message::from_msgpack(&bytes).unwrap();
        assert_eq!(
            back,
            Message::EndOfStream {
                source_id: 0,
                run_number: 12
            }
        );
    }

    // -----------------------------------------------------------------
    // ワイヤ表現(バイト固定 — C++ 側デコーダの照合材料)
    // -----------------------------------------------------------------

    /// 手計算の出典:
    ///   0x81                     fixmap(1)
    ///   0xab "EndOfStream"       fixstr(11) = 0xa0|11
    ///   0x92                     fixarray(2)  ← struct variant も positional
    ///   0x01 0x07                source_id=1, run_number=7(fixint)
    #[test]
    fn end_of_stream_wire_bytes_are_exact() {
        let eos: Message<RawFrames> = Message::EndOfStream {
            source_id: 1,
            run_number: 7,
        };
        let bytes = eos.to_msgpack().unwrap();
        assert_eq!(
            bytes,
            vec![
                0x81, 0xab, b'E', b'n', b'd', b'O', b'f', b'S', b't', b'r', b'e', b'a', b'm', 0x92,
                0x01, 0x07,
            ]
        );
    }

    #[test]
    fn message_is_fixmap_of_one_keyed_by_variant_name() {
        let data = Message::Data(sample_raw_batch()).to_msgpack().unwrap();
        let (name, payload) = {
            let parsed = parse(&data);
            assert!(matches!(parsed, Mp::Map(ref e) if e.len() == 1));
            // 所有権の都合で個別に取り直す
            let Mp::Map(entries) = parsed else {
                unreachable!()
            };
            let (k, v) = entries.into_iter().next().unwrap();
            let Mp::Str(name) = k else { panic!("key") };
            (name.to_string(), v)
        };
        assert_eq!(name, "Data");
        // Data のペイロードは Batch = positional array(5)
        assert!(matches!(payload, Mp::Array(ref a) if a.len() == 5));
    }

    /// 保存系のバイト列が msgpack の **bin** で載ること(配列フォールバックの検出)。
    /// bin へ落ちないと 1 バイト = 1 要素で 2 倍近く膨らみ、C++ 側デコーダとも噛み合わない。
    #[test]
    fn raw_frames_are_msgpack_bin_not_arrays() {
        let bytes = Message::Data(sample_raw_batch()).to_msgpack().unwrap();
        let parsed = parse(&bytes);
        let (_, batch) = unwrap_variant(&parsed);
        let Mp::Array(fields) = batch else {
            panic!("Batch must be positional array")
        };
        let Mp::Array(frames) = &fields[4] else {
            panic!("payload must be an array of frames")
        };
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Mp::Bin(&[0x08, 0x05, 0x00, 0x01, 0x02, 0x03]));
        assert_eq!(frames[1], Mp::Bin(&[0xff, 0x7f]));
        // bin8 マーカー 0xc4 + 長さ 6 が生バイト列にも現れる
        assert!(bytes.windows(2).any(|w| w == [0xc4, 0x06]));
    }

    #[test]
    fn fragment_items_are_msgpack_bin() {
        let msg = Message::Data(Batch {
            source_id: 100,
            run_number: 7,
            sequence_number: 1,
            created_ns: 5,
            payload: vec![sample_fragment()],
        });
        let bytes = msg.to_msgpack().unwrap();
        let parsed = parse(&bytes);
        let (_, batch) = unwrap_variant(&parsed);
        let Mp::Array(fields) = batch else {
            panic!("Batch")
        };
        let Mp::Array(frags) = &fields[4] else {
            panic!("Fragments")
        };
        let Mp::Array(frag) = &frags[0] else {
            panic!("Fragment must be positional array")
        };
        // items は Fragment の末尾フィールド
        assert_eq!(
            frag[schema::FRAGMENT.len() - 1],
            Mp::Bin(&items_to_bytes(&[0x0000_0000, 0x96cb_04d2]))
        );
    }

    // -----------------------------------------------------------------
    // スキーマ漂流ガード(SPEC §2.5)
    // -----------------------------------------------------------------

    #[test]
    fn schema_table_lists_every_message_variant_in_wire_order() {
        let msgs: Vec<Vec<u8>> = vec![
            Message::Data(sample_raw_batch()).to_msgpack().unwrap(),
            Message::<RawFrames>::EndOfStream {
                source_id: 1,
                run_number: 7,
            }
            .to_msgpack()
            .unwrap(),
            Message::<RawFrames>::Heartbeat {
                source_id: 1,
                run_number: 7,
                counter: 3,
            }
            .to_msgpack()
            .unwrap(),
        ];
        assert_eq!(msgs.len(), schema::MESSAGE_VARIANTS.len());
        for (bytes, variant) in msgs.iter().zip(schema::MESSAGE_VARIANTS) {
            let parsed = parse(bytes);
            let (name, payload) = unwrap_variant(&parsed);
            assert_eq!(name, variant.name);
            check(variant.payload, "[bin]", payload)
                .unwrap_or_else(|e| panic!("variant {name}: {e}"));
        }
    }

    #[test]
    fn schema_table_matches_batch_wire_layout() {
        let bytes = Message::Data(sample_raw_batch()).to_msgpack().unwrap();
        let parsed = parse(&bytes);
        let (_, batch) = unwrap_variant(&parsed);
        check_record(schema::BATCH, "[bin]", batch).unwrap();

        // 値も表の順序どおりに並んでいること(順序の入替を検出)
        let Mp::Array(fields) = batch else {
            panic!("Batch")
        };
        assert_eq!(schema::BATCH[0].name, "source_id");
        assert_eq!(fields[0], Mp::Uint(1));
        assert_eq!(schema::BATCH[1].name, "run_number");
        assert_eq!(fields[1], Mp::Uint(7));
        assert_eq!(schema::BATCH[2].name, "sequence_number");
        assert_eq!(fields[2], Mp::Uint(300));
        assert_eq!(schema::BATCH[3].name, "created_ns");
        assert_eq!(fields[3], Mp::Uint(1_755_000_000_123_456_789));
        assert_eq!(schema::BATCH[4].name, "payload");
    }

    #[test]
    fn schema_table_matches_fragment_wire_layout() {
        let msg = Message::Data(Batch {
            source_id: 100,
            run_number: 7,
            sequence_number: 1,
            created_ns: 5,
            payload: vec![sample_fragment()],
        });
        let bytes = msg.to_msgpack().unwrap();
        let parsed = parse(&bytes);
        let (_, batch) = unwrap_variant(&parsed);
        check_record(schema::BATCH, "[Fragment]", batch).unwrap();

        let Mp::Array(fields) = batch else {
            panic!("Batch")
        };
        let Mp::Array(frags) = &fields[4] else {
            panic!("Fragments")
        };
        let Mp::Array(frag) = &frags[0] else {
            panic!("Fragment")
        };
        assert_eq!(frag.len(), schema::FRAGMENT.len());

        // 全フィールドを表の順序と値で突き合わせる。値は互いに相異なるので、
        // 同じ型どうしの並べ替え(例: cobo と asad の入替)もここで落ちる。
        let expected: [(&str, Mp); 12] = [
            ("event_idx", Mp::Uint(42)),
            ("event_time", Mp::Uint(0x0000_1234_5678_9abc)),
            ("cobo", Mp::Uint(1)),
            ("asad", Mp::Uint(3)),
            ("frame_type", Mp::Uint(2)),
            ("revision", Mp::Uint(5)),
            ("read_offset", Mp::Uint(136)),
            ("status", Mp::Uint(0)),
            (
                "mult",
                Mp::Array(vec![Mp::Uint(68), Mp::Uint(0), Mp::Uint(17), Mp::Uint(3)]),
            ),
            ("window_out", Mp::Uint(512)),
            (
                "last_cell",
                Mp::Array(vec![Mp::Uint(7), Mp::Uint(9), Mp::Uint(11), Mp::Uint(13)]),
            ),
            (
                "items",
                Mp::Bin(&[0x00, 0x00, 0x00, 0x00, 0xd2, 0x04, 0xcb, 0x96]),
            ),
        ];
        for (i, (name, value)) in expected.iter().enumerate() {
            assert_eq!(schema::FRAGMENT[i].name, *name, "field {i} name");
            assert_eq!(frag[i], *value, "field {i} ({name}) value");
        }
    }

    /// 表が実型より短い/長い場合に確実に落ちること(ガード自身の健全性)。
    #[test]
    fn drift_guard_rejects_a_wrong_table() {
        let bytes = Message::Data(sample_raw_batch()).to_msgpack().unwrap();
        let parsed = parse(&bytes);
        let (_, batch) = unwrap_variant(&parsed);
        let short = &schema::BATCH[..4];
        assert!(check_record(short, "[bin]", batch).is_err());
        // 型タグ違い(payload が bin だと主張する)も検出する
        let wrong = [
            FieldDef {
                name: "source_id",
                tag: "u32",
            },
            FieldDef {
                name: "run_number",
                tag: "u32",
            },
            FieldDef {
                name: "sequence_number",
                tag: "u64",
            },
            FieldDef {
                name: "created_ns",
                tag: "u64",
            },
            FieldDef {
                name: "payload",
                tag: "bin",
            },
        ];
        assert!(check_record(&wrong, "[bin]", batch).is_err());
    }

    #[test]
    fn schema_type_fields_resolves_every_named_type() {
        for name in ["Batch", "Fragment", "EndOfStream", "Heartbeat"] {
            assert!(schema::type_fields(name).is_some(), "{name} missing");
        }
        assert!(schema::type_fields("Nonexistent").is_none());
    }

    // -----------------------------------------------------------------
    // item パック/アンパック(SPEC §2.4)
    // -----------------------------------------------------------------

    /// 手計算の出典: aget=2, chan=45, bucket=300, adc=1234
    ///   2 << 30    = 0x8000_0000
    ///   45 << 23   = 45 * 0x80_0000 = 0x1680_0000
    ///   300 << 14  = 0x12c * 0x4000 = 0x004b_0000
    ///   1234       = 0x0000_04d2
    ///   OR         = 0x96cb_04d2
    #[test]
    fn item_packs_to_the_hand_computed_word() {
        assert_eq!(pack_item(2, 45, 300, 1234).unwrap(), 0x96cb_04d2);
        assert_eq!(
            unpack_item(0x96cb_04d2),
            Item {
                aget: 2,
                chan: 45,
                bucket: 300,
                adc: 1234
            }
        );
    }

    /// 全フィールド最大値。
    ///   3 << 30    = 0xc000_0000
    ///   127 << 23  = 0x3f80_0000
    ///   511 << 14  = 0x007f_c000
    ///   4095       = 0x0000_0fff
    ///   OR         = 0xffff_cfff  ← 予約ビット [13:12] は 0 のまま
    #[test]
    fn item_max_values_keep_reserved_bits_zero() {
        let word = pack_item(ITEM_AGET_MAX, ITEM_CHAN_MAX, ITEM_BUCKET_MAX, ITEM_ADC_MAX).unwrap();
        assert_eq!(word, 0xffff_cfff);
        assert_eq!(word & ITEM_RESERVED_MASK, 0);
    }

    #[test]
    fn item_roundtrips_over_the_full_field_ranges() {
        for aget in 0..=ITEM_AGET_MAX {
            for chan in [0, 11, 45, 67, ITEM_CHAN_MAX] {
                for bucket in [0, 1, 255, 256, ITEM_BUCKET_MAX] {
                    for adc in [0, 1, 2047, ITEM_ADC_MAX] {
                        let word = pack_item(aget, chan, bucket, adc).unwrap();
                        assert_eq!(word & ITEM_RESERVED_MASK, 0);
                        assert_eq!(
                            unpack_item(word),
                            Item {
                                aget,
                                chan,
                                bucket,
                                adc
                            }
                        );
                    }
                }
            }
        }
    }

    /// 範囲外は silent に切り捨てず Err(CLAUDE.md「silent failure を作らない」)。
    #[test]
    fn item_out_of_range_fields_are_errors_not_silent_truncation() {
        assert!(pack_item(ITEM_AGET_MAX + 1, 0, 0, 0).is_err());
        assert!(pack_item(0, ITEM_CHAN_MAX + 1, 0, 0).is_err());
        assert!(pack_item(0, 0, ITEM_BUCKET_MAX + 1, 0).is_err());
        assert!(pack_item(0, 0, 0, ITEM_ADC_MAX + 1).is_err());
    }

    /// 予約ビットが立っていても unpack は他フィールドを正しく取り出す(前方互換)。
    #[test]
    fn unpack_ignores_reserved_bits() {
        let word = pack_item(1, 3, 5, 7).unwrap() | ITEM_RESERVED_MASK;
        assert_eq!(
            unpack_item(word),
            Item {
                aget: 1,
                chan: 3,
                bucket: 5,
                adc: 7
            }
        );
    }

    #[test]
    fn items_bytes_are_little_endian_u32() {
        let bytes = items_to_bytes(&[0x96cb_04d2, 0x0000_0001]);
        assert_eq!(bytes, vec![0xd2, 0x04, 0xcb, 0x96, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(
            items_from_bytes(&bytes).unwrap(),
            vec![0x96cb_04d2, 0x0000_0001]
        );
    }

    #[test]
    fn items_from_bytes_rejects_misaligned_length() {
        assert!(items_from_bytes(&[0x01, 0x02, 0x03]).is_err());
        assert!(items_from_bytes(&[]).unwrap().is_empty());
    }

    // -----------------------------------------------------------------
    // 間引き分類(SPEC §2.2「EndOfStream はいかなる間引きからも除外」)
    // -----------------------------------------------------------------

    #[test]
    fn only_data_may_be_dropped() {
        assert!(Message::Data(sample_raw_batch()).is_droppable());
        assert!(!Message::<RawFrames>::EndOfStream {
            source_id: 1,
            run_number: 7
        }
        .is_droppable());
        assert!(!Message::<RawFrames>::Heartbeat {
            source_id: 1,
            run_number: 7,
            counter: 0
        }
        .is_droppable());
    }

    /// 間引き経路の担保: 「落としてよいのは Data だけ」を通す間引き器を書くと、
    /// EOS は必ず生き残る(モニタ系の実装がこの分類 API を使う限り経路レベルで保証される)。
    #[test]
    fn a_thinning_filter_built_on_is_droppable_never_drops_eos() {
        let stream: Vec<Message<RawFrames>> = vec![
            Message::Data(sample_raw_batch()),
            Message::Heartbeat {
                source_id: 1,
                run_number: 7,
                counter: 1,
            },
            Message::Data(sample_raw_batch()),
            Message::EndOfStream {
                source_id: 1,
                run_number: 7,
            },
        ];
        // 「Data は全部落とす」最も攻撃的な間引き器
        let survivors: Vec<_> = stream.iter().filter(|m| !m.is_droppable()).collect();
        assert_eq!(survivors.len(), 2);
        assert!(survivors.iter().any(|m| matches!(
            m,
            Message::EndOfStream {
                source_id: 1,
                run_number: 7
            }
        )));
    }

    // -----------------------------------------------------------------
    // 長さ前置ストリーム(golden fixture の枠組み、SPEC §10.4)
    // -----------------------------------------------------------------

    #[test]
    fn length_prefixed_stream_splits_back_into_frames() {
        let frames = golden_messages().unwrap();
        let mut stream = Vec::new();
        for f in &frames {
            stream.extend_from_slice(&(f.len() as u32).to_le_bytes());
            stream.extend_from_slice(f);
        }
        let split = split_length_prefixed(&stream).unwrap();
        assert_eq!(split.len(), frames.len());
        for (a, b) in split.iter().zip(&frames) {
            assert_eq!(*a, b.as_slice());
        }
    }

    #[test]
    fn split_length_prefixed_rejects_truncated_input() {
        let frames = golden_messages().unwrap();
        let mut stream = Vec::new();
        stream.extend_from_slice(&(frames[0].len() as u32).to_le_bytes());
        stream.extend_from_slice(&frames[0][..frames[0].len() - 1]); // 1 バイト欠け
        assert!(split_length_prefixed(&stream).is_err());
        assert!(split_length_prefixed(&[0x01, 0x00]).is_err()); // 長さヘッダすら欠ける
    }
}
