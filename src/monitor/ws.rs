//! WS バイナリメッセージのエンコーダ(SPEC §10.1/§10.2)。
//!
//! バイトオフセットは SPEC の表そのもの。**ここを動かすと UI(027)の
//! デコーダが黙って壊れる**ので、レイアウトはオフセット assert 付きの単体テスト
//! (SPEC §10.4-4)で固定してある。

/// 13 バイトヘッダ(SPEC §10.1)。
pub const HEADER_LEN: usize = 13;
/// マジック `'T' 'P'`(off 0–1)。
pub const MAGIC: [u8; 2] = *b"TP";
/// off 3。型再定義のため 2(SPEC §10.1)。
pub const VERSION: u8 = 2;
/// off 4 の bit0 = incomplete event。
pub const FLAG_INCOMPLETE: u8 = 0x01;

/// off 2 の msgType(SPEC §10.2)。
pub const TYPE_UVW: u8 = 0x02;
pub const TYPE_WAVEFORMS: u8 = 0x03;
pub const TYPE_HISTO1D: u8 = 0x10;
pub const TYPE_HISTO2D: u8 = 0x11;

/// 13 バイトヘッダの中身。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub msg_type: u8,
    /// bit0(イベント系のみ。ヒストは常に false)。
    pub incomplete: bool,
    pub run_number: u32,
    /// ヒスト・status 系は 0(SPEC §10.1)。
    pub event_number: u32,
}

impl Header {
    /// イベント系(0x02/0x03)のヘッダ。
    pub fn event(msg_type: u8, run_number: u32, event_number: u32, incomplete: bool) -> Header {
        Header {
            msg_type,
            incomplete,
            run_number,
            event_number,
        }
    }

    /// ヒスト系(0x10/0x11)のヘッダ。`eventNumber` は 0、flags は 0(SPEC §10.1)。
    pub fn hist(msg_type: u8, run_number: u32) -> Header {
        Header {
            msg_type,
            incomplete: false,
            run_number,
            event_number: 0,
        }
    }

    /// ヘッダを書き出す(全フィールド LE)。
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        out.push(self.msg_type);
        out.push(VERSION);
        out.push(if self.incomplete { FLAG_INCOMPLETE } else { 0 });
        out.extend_from_slice(&self.run_number.to_le_bytes());
        out.extend_from_slice(&self.event_number.to_le_bytes());
    }
}

/// ヒストの軸レンジ(f32 で運ぶ — 表示専用)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    pub min: f32,
    pub max: f32,
}

/// `0x02 Uvw` — `u8 plane`, `u16 nStrips`, `u16 nBuckets`, `u16 ADC × nStrips×nBuckets`
/// (strip-major: `idx=(strip-1)*nBuckets+bucket`)。
pub fn uvw(header: Header, plane: u8, n_strips: u16, n_buckets: u16, grid: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + 5 + grid.len() * 2);
    header.write(&mut out);
    out.push(plane);
    out.extend_from_slice(&n_strips.to_le_bytes());
    out.extend_from_slice(&n_buckets.to_le_bytes());
    for v in grid {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// `0x03 Waveforms` — `u8 cobo`, `u8 asad`, `u8 nAget`, `u8 nCh`, `u16 nBuckets`,
/// `u16 ADC × nAget×nCh×nBuckets`(aget-major、raw ch 順、FPN 込み・減算なし)。
pub fn waveforms(
    header: Header,
    cobo: u8,
    asad: u8,
    n_aget: u8,
    n_ch: u8,
    n_buckets: u16,
    grid: &[u16],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + 6 + grid.len() * 2);
    header.write(&mut out);
    out.push(cobo);
    out.push(asad);
    out.push(n_aget);
    out.push(n_ch);
    out.extend_from_slice(&n_buckets.to_le_bytes());
    for v in grid {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// `0x10 Histo1d` — `u16 id`, `u32 nbins`, `f32 xmin`, `f32 xmax`, `f32 × nbins`。
///
/// `bins_le` は SPEC §5.3 の生バイト列(f64 LE)。中間 Vec を作らずに f32 へ落とす。
pub fn histo1d(header: Header, id: u16, x: Axis, bins_le: &[u8]) -> Vec<u8> {
    let nbins = bins_le.len() / 8;
    let mut out = Vec::with_capacity(HEADER_LEN + 14 + nbins * 4);
    header.write(&mut out);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&(nbins as u32).to_le_bytes());
    out.extend_from_slice(&x.min.to_le_bytes());
    out.extend_from_slice(&x.max.to_le_bytes());
    for chunk in bins_le.chunks_exact(8) {
        out.extend_from_slice(&(super::f64_le(chunk) as f32).to_le_bytes());
    }
    out
}

/// `0x11 Histo2d` — `u16 id`, `u16 nx`, `u16 ny`, `f32 xmin,xmax,ymin,ymax`,
/// `f32 × nx×ny`(**iy 外側 row-major**)。
///
/// 入力 `bins_le` は SPEC §5.3 の PUB 順(`(strip-1)*512 + bucket` = **ix が外側**)なので、
/// ここで**転置**する。この 1 行が §5.3 と §10.2 の並び順の違いを吸収している。
pub fn histo2d(
    header: Header,
    id: u16,
    nx: u16,
    ny: u16,
    x: Axis,
    y: Axis,
    bins_le: &[u8],
) -> Vec<u8> {
    let (nxu, nyu) = (nx as usize, ny as usize);
    let mut out = Vec::with_capacity(HEADER_LEN + 22 + nxu * nyu * 4);
    header.write(&mut out);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&nx.to_le_bytes());
    out.extend_from_slice(&ny.to_le_bytes());
    for v in [x.min, x.max, y.min, y.max] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for iy in 0..nyu {
        for ix in 0..nxu {
            let at = (ix * nyu + iy) * 8;
            let value = match bins_le.get(at..at + 8) {
                Some(chunk) => super::f64_le(chunk) as f32,
                None => 0.0,
            };
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}
