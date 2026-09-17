//! Ice エンコーディング **1.1** の基本型の読み書き。TODO/076 §2.2、
//! 一次資料 = archive.zeroc.com/ice/3.7 「Basic Data Encoding」。
//!
//! 規則(全部ここに閉じ込める):
//! - 数値はリトルエンディアン。`bool`/`byte` は 1 B、`int` 4 B、`long` 8 B。
//! - **size** = 要素数 255 未満なら 1 B、255 以上は `0xFF` + `int`(4 B)。
//! - `string` = size + UTF-8 の中身(NUL 終端なし)。size は**バイト数**であって文字数ではない。
//! - `seq` / `dict` = size + 要素(dict の 1 対はキー・値をこの順に並べた struct)。
//! - enum(1.1)= size 形式。struct = メンバを宣言順に並べるだけ。
//! - **encapsulation** = `int size` + `byte major` + `byte minor` + 中身。
//!   `size` は size/major/minor の 3 フィールド自身を含む(= 空なら 6)。
//!
//! 壊れた入力は全部 `Err`([`IceError::protocol`])。黙って 0 や空文字を返さない。

use std::collections::BTreeMap;

use super::error::IceError;

/// 我々が使うエンコーディング(1.1 固定 — SPEC §8.2、C++ 版 ecc_bridge と同じ)。
pub const ENCODING_MAJOR: u8 = 1;
/// 我々が使うエンコーディング(1.1 固定)。
pub const ENCODING_MINOR: u8 = 1;
/// encapsulation の固定部(size 4 B + major 1 B + minor 1 B)。
pub const ENCAPSULATION_HEADER_SIZE: usize = 6;

// ---------------------------------------------------------------------
// 書き出し
// ---------------------------------------------------------------------

/// [`OutputStream::start_encapsulation`] が返す栞。
/// [`OutputStream::end_encapsulation`] に渡すと size を埋め戻す。
#[derive(Debug, Clone, Copy)]
pub struct EncapsulationMarker {
    size_pos: usize,
}

/// エンコーディング 1.1 の書き出しバッファ。
#[derive(Debug, Default)]
pub struct OutputStream {
    buf: Vec<u8>,
}

impl OutputStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn write_byte(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn write_bytes(&mut self, value: &[u8]) {
        self.buf.extend_from_slice(value);
    }

    pub fn write_bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    pub fn write_int(&mut self, value: i32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_long(&mut self, value: i64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// size 形式(255 未満 = 1 B、以上 = `0xFF` + int)。
    pub fn write_size(&mut self, value: usize) -> Result<(), IceError> {
        if value < 255 {
            self.buf.push(value as u8);
            return Ok(());
        }
        let long = i32::try_from(value)
            .map_err(|_| IceError::protocol(format!("size {value} does not fit in an Ice int")))?;
        self.buf.push(0xFF);
        self.write_int(long);
        Ok(())
    }

    /// enum(1.1)は size 形式で並ぶ(enumerator 数 255 未満なら 1 B)。
    pub fn write_enum(&mut self, value: u32) -> Result<(), IceError> {
        self.write_size(value as usize)
    }

    pub fn write_string(&mut self, value: &str) -> Result<(), IceError> {
        self.write_size(value.len())?;
        self.buf.extend_from_slice(value.as_bytes());
        Ok(())
    }

    pub fn write_string_seq(&mut self, values: &[&str]) -> Result<(), IceError> {
        self.write_size(values.len())?;
        for value in values {
            self.write_string(value)?;
        }
        Ok(())
    }

    /// `dict<string, string>`(Ice の `Context` そのもの)。
    pub fn write_dict(&mut self, entries: &BTreeMap<String, String>) -> Result<(), IceError> {
        self.write_size(entries.len())?;
        for (key, value) in entries {
            self.write_string(key)?;
            self.write_string(value)?;
        }
        Ok(())
    }

    /// encapsulation を開く(size は仮置き、`end_encapsulation` で埋め戻す)。
    pub fn start_encapsulation(&mut self) -> EncapsulationMarker {
        let size_pos = self.buf.len();
        self.write_int(0);
        self.write_byte(ENCODING_MAJOR);
        self.write_byte(ENCODING_MINOR);
        EncapsulationMarker { size_pos }
    }

    /// encapsulation を閉じて size を埋め戻す。
    pub fn end_encapsulation(&mut self, marker: EncapsulationMarker) -> Result<(), IceError> {
        let size = self.buf.len().saturating_sub(marker.size_pos);
        let size = i32::try_from(size).map_err(|_| {
            IceError::protocol(format!(
                "encapsulation of {size} bytes does not fit in an Ice int"
            ))
        })?;
        let bytes = size.to_le_bytes();
        let slot = self
            .buf
            .get_mut(marker.size_pos..marker.size_pos + 4)
            .ok_or_else(|| IceError::protocol("encapsulation marker points outside the buffer"))?;
        slot.copy_from_slice(&bytes);
        Ok(())
    }

    /// 中身だけを持つバイト列を encapsulation(6 B ヘッダ + 中身)として書く。
    pub fn write_encapsulation(&mut self, payload: &[u8]) -> Result<(), IceError> {
        let marker = self.start_encapsulation();
        self.write_bytes(payload);
        self.end_encapsulation(marker)
    }
}

// ---------------------------------------------------------------------
// 読み込み
// ---------------------------------------------------------------------

/// エンコーディング 1.1 の読み出しカーソル。
#[derive(Debug)]
pub struct InputStream<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> InputStream<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn read_bytes(&mut self, count: usize) -> Result<&'a [u8], IceError> {
        let end = self.pos.checked_add(count).ok_or_else(|| {
            IceError::protocol(format!("read of {count} bytes overflows the cursor"))
        })?;
        let slice = self.buf.get(self.pos..end).ok_or_else(|| {
            IceError::protocol(format!(
                "truncated message: need {count} bytes at offset {}, only {} left",
                self.pos,
                self.remaining()
            ))
        })?;
        self.pos = end;
        Ok(slice)
    }

    /// 残り全部を取り出す(例外のメンバ部など、先まで解釈しないところ用)。
    pub fn read_remaining(&mut self) -> &'a [u8] {
        let rest = &self.buf[self.pos..];
        self.pos = self.buf.len();
        rest
    }

    pub fn read_byte(&mut self) -> Result<u8, IceError> {
        Ok(self.read_bytes(1)?[0])
    }

    pub fn read_bool(&mut self) -> Result<bool, IceError> {
        match self.read_byte()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(IceError::protocol(format!(
                "bool must be 0 or 1 on the wire, got {other}"
            ))),
        }
    }

    pub fn read_int(&mut self) -> Result<i32, IceError> {
        let bytes = self.read_bytes(4)?;
        let mut fixed = [0u8; 4];
        fixed.copy_from_slice(bytes);
        Ok(i32::from_le_bytes(fixed))
    }

    pub fn read_long(&mut self) -> Result<i64, IceError> {
        let bytes = self.read_bytes(8)?;
        let mut fixed = [0u8; 8];
        fixed.copy_from_slice(bytes);
        Ok(i64::from_le_bytes(fixed))
    }

    pub fn read_size(&mut self) -> Result<usize, IceError> {
        let first = self.read_byte()?;
        if first != 0xFF {
            return Ok(first as usize);
        }
        let long = self.read_int()?;
        usize::try_from(long)
            .map_err(|_| IceError::protocol(format!("negative size {long} on the wire")))
    }

    pub fn read_enum(&mut self) -> Result<u32, IceError> {
        let value = self.read_size()?;
        u32::try_from(value)
            .map_err(|_| IceError::protocol(format!("enumerator {value} is out of range")))
    }

    pub fn read_string(&mut self) -> Result<String, IceError> {
        let size = self.read_size()?;
        let bytes = self.read_bytes(size)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|err| IceError::protocol(format!("string is not valid UTF-8: {err}")))
    }

    pub fn read_string_seq(&mut self) -> Result<Vec<String>, IceError> {
        let size = self.read_size()?;
        let mut items = Vec::with_capacity(size.min(64));
        for _ in 0..size {
            items.push(self.read_string()?);
        }
        Ok(items)
    }

    pub fn read_dict(&mut self) -> Result<BTreeMap<String, String>, IceError> {
        let size = self.read_size()?;
        let mut entries = BTreeMap::new();
        for _ in 0..size {
            let key = self.read_string()?;
            let value = self.read_string()?;
            entries.insert(key, value);
        }
        Ok(entries)
    }

    /// encapsulation を 1 つ読み、**中身だけ**(6 B ヘッダを除く)を返す。
    /// エンコーディングは 1.1 のみ受理する(SPEC §8.2 で 1.1 固定 — 違う版が
    /// 返ってきたら黙って読み進めず `Err`)。
    pub fn read_encapsulation(&mut self) -> Result<&'a [u8], IceError> {
        let size = self.read_int()?;
        let size = usize::try_from(size)
            .map_err(|_| IceError::protocol(format!("negative encapsulation size {size}")))?;
        if size < ENCAPSULATION_HEADER_SIZE {
            return Err(IceError::protocol(format!(
                "encapsulation size {size} is smaller than its own header"
            )));
        }
        let major = self.read_byte()?;
        let minor = self.read_byte()?;
        if (major, minor) != (ENCODING_MAJOR, ENCODING_MINOR) {
            return Err(IceError::protocol(format!(
                "encapsulation uses encoding {major}.{minor}, only 1.1 is supported"
            )));
        }
        self.read_bytes(size - ENCAPSULATION_HEADER_SIZE)
    }
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 期待バイトは Ice 3.7 Basic Data Encoding から手で起こす)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn out() -> OutputStream {
        OutputStream::new()
    }

    #[test]
    fn scalars_are_little_endian_and_fixed_width() {
        let mut o = out();
        o.write_byte(0x7F);
        o.write_bool(true);
        o.write_bool(false);
        o.write_int(0x0102_0304);
        o.write_int(-2);
        o.write_long(0x0102_0304_0506_0708);
        o.write_long(-2);
        // int  0x01020304 -> 04 03 02 01、-2 は 2 の補数 FE FF FF FF
        // long 0x0102030405060708 -> 08 07 06 05 04 03 02 01、-2 は FE FF FF FF FF FF FF FF
        assert_eq!(
            o.as_bytes(),
            &[
                0x7F, 0x01, 0x00, //
                0x04, 0x03, 0x02, 0x01, //
                0xFE, 0xFF, 0xFF, 0xFF, //
                0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, //
                0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            ]
        );

        let bytes = o.into_bytes();
        let mut i = InputStream::new(&bytes);
        assert_eq!(i.read_byte().unwrap(), 0x7F);
        assert!(i.read_bool().unwrap());
        assert!(!i.read_bool().unwrap());
        assert_eq!(i.read_int().unwrap(), 0x0102_0304);
        assert_eq!(i.read_int().unwrap(), -2);
        assert_eq!(i.read_long().unwrap(), 0x0102_0304_0506_0708);
        assert_eq!(i.read_long().unwrap(), -2);
        assert!(i.is_empty());
    }

    #[test]
    fn a_size_below_255_is_a_single_byte() {
        let mut o = out();
        o.write_size(0).unwrap();
        o.write_size(1).unwrap();
        o.write_size(254).unwrap();
        assert_eq!(o.as_bytes(), &[0x00, 0x01, 0xFE]);
    }

    #[test]
    fn a_size_of_255_or_more_escapes_to_an_int() {
        let mut o = out();
        o.write_size(255).unwrap();
        o.write_size(300).unwrap(); // 300 = 0x0000_012C
        assert_eq!(
            o.as_bytes(),
            &[0xFF, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x2C, 0x01, 0x00, 0x00]
        );

        let bytes = o.into_bytes();
        let mut i = InputStream::new(&bytes);
        assert_eq!(i.read_size().unwrap(), 255);
        assert_eq!(i.read_size().unwrap(), 300);
    }

    #[test]
    fn a_negative_escaped_size_is_rejected() {
        // 0xFF + int32(-1)
        let bytes = [0xFFu8, 0xFF, 0xFF, 0xFF, 0xFF];
        let err = InputStream::new(&bytes).read_size().unwrap_err();
        assert!(format!("{err}").contains("negative size"), "{err}");
    }

    #[test]
    fn a_string_is_a_size_in_bytes_plus_utf8() {
        let mut o = out();
        o.write_string("Ecc").unwrap(); // 'E'=0x45 'c'=0x63 'c'=0x63
        o.write_string("").unwrap();
        o.write_string("aé").unwrap(); // 'a'=0x61、'é' は UTF-8 で C3 A9 = 2 B → size 3
        assert_eq!(
            o.as_bytes(),
            &[0x03, 0x45, 0x63, 0x63, 0x00, 0x03, 0x61, 0xC3, 0xA9]
        );
    }

    #[test]
    fn the_string_size_switches_form_at_255_bytes() {
        let short = "a".repeat(254);
        let long = "a".repeat(255);

        let mut o = out();
        o.write_string(&short).unwrap();
        assert_eq!(o.as_bytes()[0], 0xFE);
        assert_eq!(o.len(), 1 + 254);

        let mut o = out();
        o.write_string(&long).unwrap();
        assert_eq!(&o.as_bytes()[..5], &[0xFF, 0xFF, 0x00, 0x00, 0x00]);
        assert_eq!(o.len(), 5 + 255);

        // 往復(configure の DataLinkSet XML は必ず 255 B を超える — TODO/076 §5-4)。
        let bytes = o.into_bytes();
        assert_eq!(InputStream::new(&bytes).read_string().unwrap(), long);
    }

    #[test]
    fn a_string_that_is_not_utf8_is_rejected() {
        let bytes = [0x01u8, 0xFF];
        let err = InputStream::new(&bytes).read_string().unwrap_err();
        assert!(format!("{err}").contains("UTF-8"), "{err}");
    }

    #[test]
    fn a_truncated_string_is_rejected() {
        let bytes = [0x04u8, b'a', b'b'];
        let err = InputStream::new(&bytes).read_string().unwrap_err();
        assert!(format!("{err}").contains("truncated"), "{err}");
    }

    #[test]
    fn a_string_seq_is_a_size_then_the_strings() {
        let mut o = out();
        o.write_string_seq(&[]).unwrap();
        o.write_string_seq(&["f1", "f22"]).unwrap();
        assert_eq!(
            o.as_bytes(),
            &[
                0x00, // 空 seq(= Request の facet)
                0x02, 0x02, b'f', b'1', 0x03, b'f', b'2', b'2',
            ]
        );

        let bytes = o.into_bytes();
        let mut i = InputStream::new(&bytes);
        assert_eq!(i.read_string_seq().unwrap(), Vec::<String>::new());
        assert_eq!(i.read_string_seq().unwrap(), vec!["f1", "f22"]);
    }

    #[test]
    fn a_dict_is_a_size_then_key_value_pairs() {
        let mut entries = BTreeMap::new();
        entries.insert("a".to_string(), "1".to_string());
        entries.insert("bb".to_string(), "22".to_string());

        let mut o = out();
        o.write_dict(&BTreeMap::new()).unwrap();
        o.write_dict(&entries).unwrap();
        assert_eq!(
            o.as_bytes(),
            &[
                0x00, // 空 dict(= Request の context)
                0x02, //
                0x01, b'a', 0x01, b'1', //
                0x02, b'b', b'b', 0x02, b'2', b'2',
            ]
        );

        let bytes = o.into_bytes();
        let mut i = InputStream::new(&bytes);
        assert!(i.read_dict().unwrap().is_empty());
        assert_eq!(i.read_dict().unwrap(), entries);
    }

    #[test]
    fn an_enum_is_written_in_size_form() {
        let mut o = out();
        o.write_enum(0).unwrap();
        o.write_enum(6).unwrap(); // GetEcc.ice の State は 7 値 → 1 B に収まる
        assert_eq!(o.as_bytes(), &[0x00, 0x06]);

        let bytes = o.into_bytes();
        let mut i = InputStream::new(&bytes);
        assert_eq!(i.read_enum().unwrap(), 0);
        assert_eq!(i.read_enum().unwrap(), 6);
    }

    #[test]
    fn an_empty_encapsulation_is_six_bytes() {
        let mut o = out();
        let marker = o.start_encapsulation();
        o.end_encapsulation(marker).unwrap();
        // size=6(自分自身を含む)、major=1、minor=1
        assert_eq!(o.as_bytes(), &[0x06, 0x00, 0x00, 0x00, 0x01, 0x01]);
    }

    #[test]
    fn the_encapsulation_size_is_patched_after_the_body() {
        let mut o = out();
        o.write_byte(0xAA); // encapsulation の前にも何か書いてある(= Request 本体)
        let marker = o.start_encapsulation();
        o.write_string("Ecc").unwrap();
        o.end_encapsulation(marker).unwrap();
        // 6 + 4("Ecc" の size 1 B + 3 B) = 10 = 0x0A
        assert_eq!(
            o.as_bytes(),
            &[0xAA, 0x0A, 0x00, 0x00, 0x00, 0x01, 0x01, 0x03, 0x45, 0x63, 0x63]
        );

        let bytes = o.into_bytes();
        let mut i = InputStream::new(&bytes);
        assert_eq!(i.read_byte().unwrap(), 0xAA);
        let payload = i.read_encapsulation().unwrap();
        assert_eq!(payload, &[0x03, 0x45, 0x63, 0x63]);
        assert!(i.is_empty());
    }

    #[test]
    fn write_encapsulation_wraps_a_ready_made_payload() {
        let mut o = out();
        o.write_encapsulation(&[0x03, 0x45, 0x63, 0x63]).unwrap();
        assert_eq!(
            o.as_bytes(),
            &[0x0A, 0x00, 0x00, 0x00, 0x01, 0x01, 0x03, 0x45, 0x63, 0x63]
        );
    }

    #[test]
    fn an_encapsulation_with_another_encoding_is_rejected() {
        // size=6、major=1、minor=0(= 1.0 の空 encapsulation)
        let bytes = [0x06u8, 0x00, 0x00, 0x00, 0x01, 0x00];
        let err = InputStream::new(&bytes).read_encapsulation().unwrap_err();
        assert!(format!("{err}").contains("encoding 1.0"), "{err}");
    }

    #[test]
    fn an_encapsulation_smaller_than_its_header_is_rejected() {
        let bytes = [0x05u8, 0x00, 0x00, 0x00, 0x01, 0x01];
        let err = InputStream::new(&bytes).read_encapsulation().unwrap_err();
        assert!(
            format!("{err}").contains("smaller than its own header"),
            "{err}"
        );
    }

    #[test]
    fn an_encapsulation_longer_than_the_buffer_is_rejected() {
        let bytes = [0x20u8, 0x00, 0x00, 0x00, 0x01, 0x01, 0xAA];
        let err = InputStream::new(&bytes).read_encapsulation().unwrap_err();
        assert!(format!("{err}").contains("truncated"), "{err}");
    }

    #[test]
    fn reading_past_the_end_is_an_error_not_a_default_value() {
        let bytes = [0x01u8];
        let mut i = InputStream::new(&bytes);
        assert_eq!(i.read_byte().unwrap(), 1);
        assert!(i.read_byte().is_err());
        assert!(i.read_int().is_err());
        assert!(i.read_long().is_err());
    }

    #[test]
    fn a_bool_other_than_zero_or_one_is_rejected() {
        let bytes = [0x02u8];
        let err = InputStream::new(&bytes).read_bool().unwrap_err();
        assert!(format!("{err}").contains("bool must be 0 or 1"), "{err}");
    }
}
