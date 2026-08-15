//! ジオメトリ抽象(SPEC §4)。
//!
//! TPCReco `.dat` ジオメトリファイル(NEW 10 欄 / LEGACY 7 欄 / AUX 5 欄)を読み、
//! `(cobo, asad, aget, raw_ch 0–67)` から [`ChannelRole`] を引けるようにする。
//! ホットパス(decoder のチャンネル解決)は稠密配列の O(1) 引きのみで済むよう、
//! FPN リオーダ(`REORDER_FROM_GEOMETRY_TO_GRAW`)はパース時に一度だけ適用する(SPEC §4.3)。
//!
//! mm 変換はしない(SPEC §4.4)。ヘッダスカラ(ANGLES 等)は [`HeaderScalars`] に
//! そのまま保持するだけ。

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------
// ハードウェア定数(AGET/AsAd ASIC の物理構造。実験ごとに変わる「ch 数」— mini
// 256 / ELITPC 1024 の総信号 ch 数 — とは別物なのでここに焼き込んでよい)。
// ---------------------------------------------------------------------

/// AsAd 1 枚あたりの AGET チップ数(GET ハードウェア定数)。
pub const AGET_CHIPS_PER_ASAD: u32 = 4;

/// AGET 1 チップあたりの raw チャンネル数(信号 64 + FPN 4)。
pub const RAW_CH_PER_AGET: u32 = 68;

/// AGET 1 チップあたりの信号チャンネル数(`.dat` の `AGET_CH` が使う範囲)。
pub const SIGNAL_CH_PER_AGET: u32 = 64;

/// FPN の raw チャンネル番号(AGET 毎、昇順)。`ChannelRole::Fpn.index` は
/// この配列内の位置 + 1(1..4)。
const FPN_RAW_CHANNELS: [u32; 4] = [11, 22, 45, 56];

/// 信号チャンネル(0–63)→ raw チャンネル(0–67)の並び替え定数表。
///
/// rust_reference(`~/test/get/reuse/rust_reference/geometry.rs` 85–89 行)採用。
/// TPCReco `GeometryTPC::Aget_normal2raw` のループ版と全 64 入力で一致することを
/// `reorder_loop_matches_constant_table_for_all_64_inputs` で確認済み。
pub const REORDER_FROM_GEOMETRY_TO_GRAW: [u32; 64] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 46, 47, 48, 49, 50, 51, 52,
    53, 54, 55, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67,
];

// ---------------------------------------------------------------------
// 役割・平面
// ---------------------------------------------------------------------

/// ストリップ平面(U/V/W)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    U = 0,
    V = 1,
    W = 2,
}

impl Plane {
    fn parse(token: &str) -> Option<Plane> {
        match token {
            "U" => Some(Plane::U),
            "V" => Some(Plane::V),
            "W" => Some(Plane::W),
            _ => None,
        }
    }

    /// TSV ダンプ等で使う表示名。
    pub fn as_str(self) -> &'static str {
        match self {
            Plane::U => "U",
            Plane::V => "V",
            Plane::W => "W",
        }
    }
}

/// `(cobo, asad, aget, raw_ch)` を引いた結果の役割(SPEC §4.2)。
///
/// FPN は「ハッシュミス」ではなく明示的な役割として返す。`Unmapped` は
/// `.dat` に記載のないチャンネル(要可視化 — [`Geometry::unmapped_hit_count`])。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelRole {
    /// 信号ストリップ。
    Strip {
        plane: Plane,
        section: u8,
        strip: u16,
    },
    /// FPN(1..4、raw {11,22,45,56} の並び順)。
    Fpn { index: u8 },
    /// AUX 入力(ヒストからは除外、波形ビューでは表示 — SPEC §4.1)。
    Aux { name: String },
    /// `.dat` に記載なし。
    Unmapped,
}

/// ヘッダスカラ 7 キー(SPEC §4.1)。mm 計算はしない(SPEC §4.4)ので、
/// パースした値をそのまま保持するだけ。値が読めなかったキーは `None`。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HeaderScalars {
    pub angles_deg: Option<[f64; 3]>,
    pub diamond_size_mm: Option<f64>,
    pub reference_point_mm: Option<(f64, f64)>,
    pub drift_velocity_cm_per_us: Option<f64>,
    pub sampling_rate_mhz: Option<f64>,
    pub trigger_delay_us: Option<f64>,
    /// `DRIFT CAGE ACCEPTANCE`(rust_reference が取りこぼしていたキー — SPEC §4.1)。
    pub drift_cage_acceptance_mm: Option<(f64, f64)>,
}

/// 重複 `(cobo,asad,aget,raw_ch)` の警告(先勝ちで無視した側の記録)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateChannel {
    pub cobo: u32,
    pub asad: u32,
    pub aget: u32,
    pub raw_ch: u32,
    pub line_number: usize,
}

/// トークン数が NEW(10)/ LEGACY(7)/ AUX(5)のいずれにも合わない、または
/// 数値パースに失敗した行の記録(silent にしないためのフック)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MalformedLine {
    pub line_number: usize,
}

/// [`load`] の失敗(ファイル I/O のみ。ファイル内容の不整合は警告扱い — 上記参照)。
#[derive(Debug, thiserror::Error)]
pub enum GeometryError {
    #[error("failed to read geometry file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------
// Geometry 本体
// ---------------------------------------------------------------------

/// パース済みジオメトリ。`(cobo, asad, aget, raw_ch)` の稠密配列(累積 AsAd
/// オフセット方式 — TPCReco `Global_raw2normal` と同じ考え方、SPEC §4.2)。
#[derive(Debug)]
pub struct Geometry {
    pub header: HeaderScalars,
    /// 平面ごとの最大ストリップ番号(`[U, V, W]`。`Plane::U as usize` 等で引く)。
    pub max_strip: [u16; 3],
    channels: Vec<ChannelRole>,
    /// cobo ごとの累積 AsAd オフセット(`asad_offset[cobo] + asad` がグローバル AsAd 番号)。
    asad_offset: Vec<u32>,
    /// cobo ごとの AsAd 枚数。
    asad_count: Vec<u32>,
    duplicates: Vec<DuplicateChannel>,
    malformed: Vec<MalformedLine>,
    /// `Unmapped` を引いた回数(ホットパスはアトミック加算のみ — ログ整形はしない)。
    unmapped_hits: AtomicU64,
}

/// 範囲外・未記載を [`Geometry::lookup_ref`] が返すための実体。
static UNMAPPED: ChannelRole = ChannelRole::Unmapped;

impl Geometry {
    /// [`Self::lookup`] の借用版。ホットパス(monitor の per-sample 引き)はこちらを使う
    /// —— `Aux { name: String }` を持つジオメトリでは `clone` が per-sample malloc になるため。
    pub fn lookup_ref(&self, cobo: u32, asad: u32, aget: u32, raw_ch: u32) -> &ChannelRole {
        match self.index_of(cobo, asad, aget, raw_ch) {
            Some(i) => {
                let role = &self.channels[i];
                if *role == ChannelRole::Unmapped {
                    self.unmapped_hits.fetch_add(1, Ordering::Relaxed);
                }
                role
            }
            None => {
                self.unmapped_hits.fetch_add(1, Ordering::Relaxed);
                &UNMAPPED
            }
        }
    }

    /// `(cobo, asad, aget, raw_ch)` から役割を引く。範囲外・未記載はいずれも
    /// `Unmapped` を返し、`unmapped_hits` を加算する(silent にしない可視化フック)。
    pub fn lookup(&self, cobo: u32, asad: u32, aget: u32, raw_ch: u32) -> ChannelRole {
        self.lookup_ref(cobo, asad, aget, raw_ch).clone()
    }

    /// `lookup` が `Unmapped` を返した累計回数。呼び出し側が周期的に読んで
    /// `info!` 以上で可視化する想定(ホットパスではログを直接出さない)。
    pub fn unmapped_hit_count(&self) -> u64 {
        self.unmapped_hits.load(Ordering::Relaxed)
    }

    /// 重複 `(cobo,asad,aget,raw_ch)` の警告一覧(先勝ちで捨てた側)。
    pub fn duplicate_warnings(&self) -> &[DuplicateChannel] {
        &self.duplicates
    }

    /// パースできなかった行の一覧。
    pub fn malformed_lines(&self) -> &[MalformedLine] {
        &self.malformed
    }

    /// 登場した CoBo の数(`cobo` id の最大値 + 1)。
    pub fn cobo_count(&self) -> usize {
        self.asad_offset.len()
    }

    /// `.dat` に現れた `(cobo, asad)` の在庫一覧(`cobo` 昇順 → `asad` 昇順、重複なし)。
    ///
    /// 025(controller の期待フラグメント導出、SPEC §9.2 `expected_fragments`)向け。
    /// `asad_count`(構築時に確定した cobo ごとの AsAd 枚数)を直接なぞるだけで、
    /// `lookup` は一切呼ばない — ホットパス可視化カウンタ [`Self::unmapped_hit_count`] を
    /// 汚さないための配慮(016 が `dump_tsv` パース経由でこれを避けていた理由と同じ)。
    pub fn asad_inventory(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for (cobo, &count) in self.asad_count.iter().enumerate() {
            for asad in 0..count {
                out.push((cobo as u32, asad));
            }
        }
        out
    }

    fn index_of(&self, cobo: u32, asad: u32, aget: u32, raw_ch: u32) -> Option<usize> {
        compute_index(
            &self.asad_offset,
            &self.asad_count,
            cobo,
            asad,
            aget,
            raw_ch,
        )
    }
}

/// `(cobo,asad,aget,raw_ch)` を稠密配列の添字に変換する。`Geometry::index_of`
/// (ルックアップ)と `build_geometry`(構築)の両方から使う共通実装。
fn compute_index(
    asad_offset: &[u32],
    asad_count: &[u32],
    cobo: u32,
    asad: u32,
    aget: u32,
    raw_ch: u32,
) -> Option<usize> {
    let c = cobo as usize;
    if c >= asad_offset.len() || asad >= asad_count[c] {
        return None;
    }
    if aget >= AGET_CHIPS_PER_ASAD || raw_ch >= RAW_CH_PER_AGET {
        return None;
    }
    let global_asad = asad_offset[c] as usize + asad as usize;
    Some(
        (global_asad * AGET_CHIPS_PER_ASAD as usize + aget as usize) * RAW_CH_PER_AGET as usize
            + raw_ch as usize,
    )
}

/// 全チャンネルを `cobo/asad/aget/raw_ch` 昇順で 1 行ずつ、
/// `cobo asad aget raw_ch role plane section strip` をタブ区切りでダンプする
/// (SPEC §4.5、Rust/C++ 一致テストの入力。一致テスト自体は後続ユニット)。
///
/// `role` に付随しない列(FPN の index、AUX の name を含む plane/section/strip)は
/// 空文字。ホットパスの `unmapped_hits` は増やさない(全走査で意味のない値になるため)。
pub fn dump_tsv(geometry: &Geometry) -> String {
    let mut out = String::new();
    for cobo in 0..geometry.asad_offset.len() {
        let asad_count = geometry.asad_count[cobo];
        for asad in 0..asad_count {
            for aget in 0..AGET_CHIPS_PER_ASAD {
                for raw_ch in 0..RAW_CH_PER_AGET {
                    // cobo/asad/aget/raw_ch はすべてこのループの範囲内(構築時に
                    // asad_count 等から導いた境界そのもの)なので必ず Some。
                    let Some(idx) = compute_index(
                        &geometry.asad_offset,
                        &geometry.asad_count,
                        cobo as u32,
                        asad,
                        aget,
                        raw_ch,
                    ) else {
                        continue;
                    };
                    let (role, plane, section, strip) = match &geometry.channels[idx] {
                        ChannelRole::Strip {
                            plane,
                            section,
                            strip,
                        } => (
                            "Strip",
                            plane.as_str().to_string(),
                            section.to_string(),
                            strip.to_string(),
                        ),
                        ChannelRole::Fpn { .. } => {
                            ("Fpn", String::new(), String::new(), String::new())
                        }
                        ChannelRole::Aux { .. } => {
                            ("Aux", String::new(), String::new(), String::new())
                        }
                        ChannelRole::Unmapped => {
                            ("Unmapped", String::new(), String::new(), String::new())
                        }
                    };
                    // String への書き込みは失敗しない(容量拡張以外に失敗要因がない)。
                    let _ = writeln!(
                        out,
                        "{cobo}\t{asad}\t{aget}\t{raw_ch}\t{role}\t{plane}\t{section}\t{strip}"
                    );
                }
            }
        }
    }
    out
}

/// ファイルパスから読み込む([`parse`] のファイル版)。
pub fn load(path: impl AsRef<Path>) -> Result<Geometry, GeometryError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|source| GeometryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(parse(&text))
}

/// `.dat` テキストをパースする。行単位で寛容(TPCReco の sscanf 方式を踏襲):
/// 解釈できない行はエラーにせず [`MalformedLine`] として記録し、後続行の
/// パースは続行する。重複 `(cobo,asad,aget,raw_ch)` も同様に警告 + 先勝ち。
pub fn parse(text: &str) -> Geometry {
    let mut header = HeaderScalars::default();
    let mut pending: Vec<PendingChannel> = Vec::new();
    let mut active_agets: HashSet<(u32, u32, u32)> = HashSet::new();
    let mut malformed: Vec<MalformedLine> = Vec::new();
    let mut max_strip = [0u16; 3];

    for (line_idx, raw_line) in text.lines().enumerate() {
        let line_number = line_idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if try_parse_header_scalar(trimmed, &mut header) {
            continue;
        }

        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        match tokens.len() {
            10 => match parse_new_strip_line(&tokens) {
                Some(parsed) => push_strip(
                    parsed,
                    line_number,
                    &mut pending,
                    &mut active_agets,
                    &mut max_strip,
                    &mut malformed,
                ),
                None => malformed.push(MalformedLine { line_number }),
            },
            7 => match parse_legacy_strip_line(&tokens) {
                Some(parsed) => push_strip(
                    parsed,
                    line_number,
                    &mut pending,
                    &mut active_agets,
                    &mut max_strip,
                    &mut malformed,
                ),
                None => malformed.push(MalformedLine { line_number }),
            },
            5 => match parse_aux_line(&tokens) {
                Some(parsed) => push_aux(
                    parsed,
                    line_number,
                    &mut pending,
                    &mut active_agets,
                    &mut malformed,
                ),
                None => malformed.push(MalformedLine { line_number }),
            },
            _ => malformed.push(MalformedLine { line_number }),
        }
    }

    build_geometry(header, max_strip, pending, active_agets, malformed)
}

// ---------------------------------------------------------------------
// パース内部実装
// ---------------------------------------------------------------------

struct ParsedStrip {
    cobo: u32,
    asad: u32,
    aget: u32,
    plane: Plane,
    section: u8,
    strip: u16,
    signal_ch: u32,
}

struct ParsedAux {
    name: String,
    cobo: u32,
    asad: u32,
    aget: u32,
    signal_ch: u32,
}

struct PendingChannel {
    cobo: u32,
    asad: u32,
    aget: u32,
    raw_ch: u32,
    role: ChannelRole,
    line_number: usize,
}

/// NEW(10 欄): `DIR SECTION STRIP COBO ASAD AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS`。
fn parse_new_strip_line(tokens: &[&str]) -> Option<ParsedStrip> {
    let plane = Plane::parse(tokens[0])?;
    let section: u8 = tokens[1].parse().ok()?;
    let strip: u16 = tokens[2].parse().ok()?;
    let cobo: u32 = tokens[3].parse().ok()?;
    let asad: u32 = tokens[4].parse().ok()?;
    let aget: u32 = tokens[5].parse().ok()?;
    if aget >= AGET_CHIPS_PER_ASAD {
        return None;
    }
    let signal_ch: u32 = tokens[6].parse().ok()?;
    let _off_pad: f64 = tokens[7].parse().ok()?;
    let _off_strip: f64 = tokens[8].parse().ok()?;
    let _len_pads: f64 = tokens[9].parse().ok()?;
    Some(ParsedStrip {
        cobo,
        asad,
        aget,
        plane,
        section,
        strip,
        signal_ch,
    })
}

/// LEGACY(7 欄): `DIR STRIP AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS`(section/cobo/asad = 0)。
fn parse_legacy_strip_line(tokens: &[&str]) -> Option<ParsedStrip> {
    let plane = Plane::parse(tokens[0])?;
    let strip: u16 = tokens[1].parse().ok()?;
    let aget: u32 = tokens[2].parse().ok()?;
    if aget >= AGET_CHIPS_PER_ASAD {
        return None;
    }
    let signal_ch: u32 = tokens[3].parse().ok()?;
    let _off_pad: f64 = tokens[4].parse().ok()?;
    let _off_strip: f64 = tokens[5].parse().ok()?;
    let _len_pads: f64 = tokens[6].parse().ok()?;
    Some(ParsedStrip {
        cobo: 0,
        asad: 0,
        aget,
        plane,
        section: 0,
        strip,
        signal_ch,
    })
}

/// AUX(5 欄): `NAME COBO ASAD AGET AGET_CH`。
///
/// 設計メモ(要レビュー): TPCReco の `LoadAnalog` は実ファイル全版で未実装の
/// スタブ(登録処理なし)であり、AUX の `AGET_CH` が信号番号(0–63)か raw
/// (0–67)かを参照実装の挙動からは確定できない。実データ(ELITPC AUX: 58/60/62)
/// はどちらの解釈でも範囲内で判別不能。本実装は同一列名 `AGET_CH` を STRIP 行と
/// 共有している以上「信号番号 0–63」規約が同じとみなし、STRIP と同じ
/// `REORDER_FROM_GEOMETRY_TO_GRAW` を適用する。
fn parse_aux_line(tokens: &[&str]) -> Option<ParsedAux> {
    let name = tokens[0].to_string();
    let cobo: u32 = tokens[1].parse().ok()?;
    let asad: u32 = tokens[2].parse().ok()?;
    let aget: u32 = tokens[3].parse().ok()?;
    if aget >= AGET_CHIPS_PER_ASAD {
        return None;
    }
    let signal_ch: u32 = tokens[4].parse().ok()?;
    Some(ParsedAux {
        name,
        cobo,
        asad,
        aget,
        signal_ch,
    })
}

fn push_strip(
    parsed: ParsedStrip,
    line_number: usize,
    pending: &mut Vec<PendingChannel>,
    active_agets: &mut HashSet<(u32, u32, u32)>,
    max_strip: &mut [u16; 3],
    malformed: &mut Vec<MalformedLine>,
) {
    let Some(&raw_ch) = REORDER_FROM_GEOMETRY_TO_GRAW.get(parsed.signal_ch as usize) else {
        malformed.push(MalformedLine { line_number });
        return;
    };
    active_agets.insert((parsed.cobo, parsed.asad, parsed.aget));
    let plane_idx = parsed.plane as usize;
    if parsed.strip > max_strip[plane_idx] {
        max_strip[plane_idx] = parsed.strip;
    }
    pending.push(PendingChannel {
        cobo: parsed.cobo,
        asad: parsed.asad,
        aget: parsed.aget,
        raw_ch,
        role: ChannelRole::Strip {
            plane: parsed.plane,
            section: parsed.section,
            strip: parsed.strip,
        },
        line_number,
    });
}

fn push_aux(
    parsed: ParsedAux,
    line_number: usize,
    pending: &mut Vec<PendingChannel>,
    active_agets: &mut HashSet<(u32, u32, u32)>,
    malformed: &mut Vec<MalformedLine>,
) {
    let Some(&raw_ch) = REORDER_FROM_GEOMETRY_TO_GRAW.get(parsed.signal_ch as usize) else {
        malformed.push(MalformedLine { line_number });
        return;
    };
    active_agets.insert((parsed.cobo, parsed.asad, parsed.aget));
    pending.push(PendingChannel {
        cobo: parsed.cobo,
        asad: parsed.asad,
        aget: parsed.aget,
        raw_ch,
        role: ChannelRole::Aux { name: parsed.name },
        line_number,
    });
}

/// `KEY: 値...` 形式のヘッダ行を認識して `header` に書き込む。キーに一致すれば
/// 値のパース可否によらず(未パースなら該当フィールドは `None` のまま)`true`
/// を返す — 認識できたキーは「データ行」ではないので、以降のトークン数判定に
/// は回さない。
fn try_parse_header_scalar(trimmed: &str, header: &mut HeaderScalars) -> bool {
    if let Some(rest) = trimmed.strip_prefix("ANGLES:") {
        header.angles_deg = parse_n_f64::<3>(rest);
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("DIAMOND SIZE:") {
        header.diamond_size_mm = parse_n_f64::<1>(rest).map(|v| v[0]);
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("REFERENCE POINT:") {
        header.reference_point_mm = parse_n_f64::<2>(rest).map(|v| (v[0], v[1]));
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("DRIFT VELOCITY:") {
        header.drift_velocity_cm_per_us = parse_n_f64::<1>(rest).map(|v| v[0]);
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("SAMPLING RATE:") {
        header.sampling_rate_mhz = parse_n_f64::<1>(rest).map(|v| v[0]);
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("TRIGGER DELAY:") {
        header.trigger_delay_us = parse_n_f64::<1>(rest).map(|v| v[0]);
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("DRIFT CAGE ACCEPTANCE:") {
        header.drift_cage_acceptance_mm = parse_n_f64::<2>(rest).map(|v| (v[0], v[1]));
        return true;
    }
    false
}

/// 空白区切りで厳密に `N` 個の `f64` が取れたときだけ `Some`。
fn parse_n_f64<const N: usize>(s: &str) -> Option<[f64; N]> {
    let vals: Vec<f64> = s
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if vals.len() == N {
        let mut out = [0.0_f64; N];
        out.copy_from_slice(&vals);
        Some(out)
    } else {
        None
    }
}

/// 稠密配列を組み立てる。手順: (1) 登場した全 `(cobo,asad,aget)` に FPN を
/// 先に置く(ハードウェア的な真実、`.dat` には書かれない)。(2) ファイル順に
/// STRIP/AUX を適用し、埋まっていたら重複警告 + 先勝ち。
fn build_geometry(
    header: HeaderScalars,
    max_strip: [u16; 3],
    pending: Vec<PendingChannel>,
    active_agets: HashSet<(u32, u32, u32)>,
    mut malformed: Vec<MalformedLine>,
) -> Geometry {
    let cobo_count = active_agets
        .iter()
        .map(|&(cobo, _, _)| cobo)
        .max()
        .map(|m| m as usize + 1)
        .unwrap_or(0);

    let mut asad_count = vec![0u32; cobo_count];
    for &(cobo, asad, _aget) in &active_agets {
        let c = cobo as usize;
        if asad + 1 > asad_count[c] {
            asad_count[c] = asad + 1;
        }
    }

    let mut asad_offset = vec![0u32; cobo_count];
    let mut running: u32 = 0;
    for c in 0..cobo_count {
        asad_offset[c] = running;
        running += asad_count[c];
    }
    let total_asad = running as usize;
    let array_len = total_asad * AGET_CHIPS_PER_ASAD as usize * RAW_CH_PER_AGET as usize;
    let mut channels = vec![ChannelRole::Unmapped; array_len];

    for &(cobo, asad, aget) in &active_agets {
        for (i, &raw) in FPN_RAW_CHANNELS.iter().enumerate() {
            if let Some(idx) = compute_index(&asad_offset, &asad_count, cobo, asad, aget, raw) {
                channels[idx] = ChannelRole::Fpn {
                    index: (i + 1) as u8,
                };
            }
        }
    }

    let mut duplicates = Vec::new();
    for entry in pending {
        match compute_index(
            &asad_offset,
            &asad_count,
            entry.cobo,
            entry.asad,
            entry.aget,
            entry.raw_ch,
        ) {
            Some(idx) => {
                if channels[idx] == ChannelRole::Unmapped {
                    channels[idx] = entry.role;
                } else {
                    duplicates.push(DuplicateChannel {
                        cobo: entry.cobo,
                        asad: entry.asad,
                        aget: entry.aget,
                        raw_ch: entry.raw_ch,
                        line_number: entry.line_number,
                    });
                }
            }
            None => malformed.push(MalformedLine {
                line_number: entry.line_number,
            }),
        }
    }

    Geometry {
        header,
        max_strip,
        channels,
        asad_offset,
        asad_count,
        duplicates,
        malformed,
        unmapped_hits: AtomicU64::new(0),
    }
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// TPCReco `GeometryTPC::Aget_normal2raw`(1315–1327 行付近)の移植版ループ。
    /// `REORDER_FROM_GEOMETRY_TO_GRAW` はこのループの結果を事前計算した定数表
    /// であることを、全 64 入力で確認する。
    fn aget_normal2raw_loop(channel_idx: u32) -> u32 {
        let mut raw = channel_idx;
        for &fpn in &FPN_RAW_CHANNELS {
            if fpn < raw {
                raw += 1;
            }
            if fpn == raw {
                raw += 1;
            }
        }
        raw
    }

    #[test]
    fn reorder_loop_matches_constant_table_for_all_64_inputs() {
        for signal_ch in 0..SIGNAL_CH_PER_AGET {
            assert_eq!(
                aget_normal2raw_loop(signal_ch),
                REORDER_FROM_GEOMETRY_TO_GRAW[signal_ch as usize],
                "mismatch at signal channel {signal_ch}"
            );
        }
    }

    #[test]
    fn plane_parse_accepts_only_uppercase_uvw() {
        assert_eq!(Plane::parse("U"), Some(Plane::U));
        assert_eq!(Plane::parse("V"), Some(Plane::V));
        assert_eq!(Plane::parse("W"), Some(Plane::W));
        assert_eq!(Plane::parse("u"), None);
        assert_eq!(Plane::parse("FPN"), None);
        assert_eq!(Plane::parse(""), None);
    }

    // --- AUX の AGET_CH 規約(設計メモ: parse_aux_line のコメント参照) ---

    #[test]
    fn aux_agent_ch_uses_same_signal_reorder_as_strip_columns() {
        let text = "\
ANGLES: 0.0 0.0 0.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
DRIFT VELOCITY: 1.0
SAMPLING RATE: 1.0
TRIGGER DELAY: 0.0
DRIFT CAGE ACCEPTANCE: -1.0 1.0
AuxTest\t0\t0\t0\t15
";
        let geometry = parse(text);
        // 手計算: REORDER_FROM_GEOMETRY_TO_GRAW[15] = 16
        // (表の並び: idx11=12,12=13,13=14,14=15,15=16 — FPN=11 を過ぎた分だけ +1 シフト)。
        assert_eq!(
            geometry.lookup(0, 0, 0, 16),
            ChannelRole::Aux {
                name: "AuxTest".to_string()
            }
        );
        assert_eq!(geometry.lookup(0, 0, 0, 15), ChannelRole::Unmapped);
    }

    // --- 重複・malformed(ホットパス外なのでインライン文字列で十分) ---

    #[test]
    fn duplicate_channel_key_warns_and_keeps_first() {
        let text = "\
ANGLES: 0.0 0.0 0.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
DRIFT VELOCITY: 1.0
SAMPLING RATE: 1.0
TRIGGER DELAY: 0.0
DRIFT CAGE ACCEPTANCE: -1.0 1.0
U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10
V\t0\t9\t0\t0\t0\t0\t9.0\t9.0\t10
";
        let geometry = parse(text);
        // 先勝ち: 2 行目(V/strip9)は同じ (cobo0,asad0,aget0,raw0) を狙うが無視される。
        assert_eq!(
            geometry.lookup(0, 0, 0, 0),
            ChannelRole::Strip {
                plane: Plane::U,
                section: 0,
                strip: 1
            }
        );
        assert_eq!(geometry.duplicate_warnings().len(), 1);
        let dup = geometry.duplicate_warnings()[0];
        assert_eq!(dup.cobo, 0);
        assert_eq!(dup.asad, 0);
        assert_eq!(dup.aget, 0);
        assert_eq!(dup.raw_ch, 0);
        // 手計算: ヘッダ 7 行(1–7 行目) + U 行(8 行目) + V 行(9 行目)。
        assert_eq!(dup.line_number, 9);
    }

    #[test]
    fn malformed_lines_are_recorded_not_fatal() {
        let text = "\
ANGLES: 0.0 0.0 0.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
DRIFT VELOCITY: 1.0
SAMPLING RATE: 1.0
TRIGGER DELAY: 0.0
DRIFT CAGE ACCEPTANCE: -1.0 1.0
foo bar baz qux quux corge
Z\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10
U\t0\t1\t0\t0\t0\t99\t0.0\t0.0\t10
U\t0\t2\t0\t0\t0\t0\t0.0\t0.0\t10
";
        let geometry = parse(text);
        // 1行目: "foo bar baz qux quux corge" = 6 トークン(10/7/5 いずれにも合わない)
        // 2行目: DIR="Z" は Plane として不正(NEW 10 欄の形はしているが弾かれる)
        // 3行目: AGET_CH=99 は 0..64 の範囲外
        // どれも malformed 行として記録され、4 行目(正しい行)は生きている。
        assert_eq!(geometry.malformed_lines().len(), 3);
        assert_eq!(
            geometry.lookup(0, 0, 0, 0),
            ChannelRole::Strip {
                plane: Plane::U,
                section: 0,
                strip: 2
            }
        );
    }

    #[test]
    fn aget_index_out_of_hardware_range_is_malformed_not_corrupting() {
        // aget=4 は AsAd 1 枚あたりの物理チップ数(4 個, 0..3)を超える。
        // もし境界チェックを忘れると次の AsAd ブロックへ書き込みが漏れて
        // データ破壊になるので、malformed 扱いで弾かれることを確認する。
        let text = "\
ANGLES: 0.0 0.0 0.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
DRIFT VELOCITY: 1.0
SAMPLING RATE: 1.0
TRIGGER DELAY: 0.0
DRIFT CAGE ACCEPTANCE: -1.0 1.0
U\t0\t1\t0\t0\t4\t0\t0.0\t0.0\t10
";
        let geometry = parse(text);
        assert_eq!(geometry.malformed_lines().len(), 1);
        assert_eq!(geometry.cobo_count(), 0);
    }

    // --- Unmapped 可視化フック ---

    #[test]
    fn unmapped_hit_count_increments_on_every_unmapped_lookup() {
        let text = "\
ANGLES: 0.0 0.0 0.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
DRIFT VELOCITY: 1.0
SAMPLING RATE: 1.0
TRIGGER DELAY: 0.0
DRIFT CAGE ACCEPTANCE: -1.0 1.0
U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10
";
        let geometry = parse(text);
        assert_eq!(geometry.unmapped_hit_count(), 0);

        // in-bounds だが未記載(同じ active aget の未使用信号 ch)。
        assert_eq!(geometry.lookup(0, 0, 0, 63), ChannelRole::Unmapped);
        assert_eq!(geometry.unmapped_hit_count(), 1);

        // 完全に範囲外(cobo が存在しない)。
        assert_eq!(geometry.lookup(9, 0, 0, 0), ChannelRole::Unmapped);
        assert_eq!(geometry.unmapped_hit_count(), 2);

        // Strip の再ヒットはカウントしない。
        let _ = geometry.lookup(0, 0, 0, 0);
        assert_eq!(geometry.unmapped_hit_count(), 2);
    }

    #[test]
    fn dump_tsv_does_not_increment_unmapped_hit_count() {
        let text = "\
ANGLES: 0.0 0.0 0.0
DIAMOND SIZE: 1.0
REFERENCE POINT: 0.0 0.0
DRIFT VELOCITY: 1.0
SAMPLING RATE: 1.0
TRIGGER DELAY: 0.0
DRIFT CAGE ACCEPTANCE: -1.0 1.0
U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10
";
        let geometry = parse(text);
        let dump = dump_tsv(&geometry);
        assert!(!dump.is_empty());
        // 1 cobo * 1 asad * 4 aget * 68 raw_ch = 272 行(Unmapped だらけでも
        // ホットパス用カウンタは動かさない設計)。
        assert_eq!(dump.lines().count(), 272);
        assert_eq!(geometry.unmapped_hit_count(), 0);
    }
}
