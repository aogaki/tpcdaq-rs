//! pedestal 較正表(TODO/081)。ch 毎の baseline / noise(生 ADC、全 bucket・全イベント、減算なし)。
//!
//! 定義は 068 SUMMARY §0 / `reference/_spike/asset_survey/src/pedestal.rs` の `MomentAccum` を
//! 踏襲する(転記ではなく再実装): mean = 生 ADC の平均、rms = 母標準偏差(n 割り。ROOT の
//! `TH1::GetRMS` と同義)。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    is_fpn, now_rfc3339, read_json, scan_files, tool_version, write_json, CalibError, ChKey, Meta,
};
use crate::msg::{items_from_bytes, unpack_item};

/// 1 ch 分のレコード。フィールド順は発注書の列挙順(`mean, rms, run 数, is_fpn`)に
/// `asad,aget,ch` キーを前置したもの。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PedestalChannel {
    pub asad: u8,
    pub aget: u8,
    pub ch: u8,
    pub mean: f64,
    pub rms: f64,
    /// このチャンネルのサンプルが実際に含まれていた入力ファイルの数(mod.rs 冒頭の解釈を参照)。
    pub runs: u32,
    pub is_fpn: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PedestalTable {
    pub meta: Meta,
    pub channels: Vec<PedestalChannel>,
}

#[derive(Default)]
struct Accum {
    n: u64,
    sum: u64,
    sum_sq: u64,
    files: BTreeSet<usize>,
}

impl Accum {
    fn add(&mut self, adc: u16, file_idx: usize) {
        self.n += 1;
        self.sum += u64::from(adc);
        self.sum_sq += u64::from(adc) * u64::from(adc);
        self.files.insert(file_idx);
    }

    fn mean(&self) -> f64 {
        self.sum as f64 / self.n as f64
    }

    /// 標本標準偏差(母分散、n 割り)。
    fn rms(&self) -> f64 {
        let n = self.n as f64;
        let m = self.sum as f64 / n;
        let var = self.sum_sq as f64 / n - m * m;
        if var <= 0.0 {
            0.0
        } else {
            var.sqrt()
        }
    }
}

/// `paths` の全フレームを走査し、ch 毎の mean/rms/runs/is_fpn を計算する。
pub fn compute(paths: &[PathBuf]) -> Result<PedestalTable, CalibError> {
    let mut accum: BTreeMap<ChKey, Accum> = BTreeMap::new();

    let stats = scan_files(paths, |file_idx, frag| {
        let asad = frag.asad;
        let items = match items_from_bytes(&frag.items) {
            Ok(v) => v,
            Err(e) => {
                // Decoder が組んだ items は常に 4 バイト整列のはずなので理論上到達しないが、
                // silent failure を作らないため報告だけはする(CLAUDE.md)。
                eprintln!("calib: pedestal: items_from_bytes failed for a decoded frame: {e}");
                return;
            }
        };
        for word in items {
            let it = unpack_item(word);
            let key = (asad, it.aget, it.chan);
            accum.entry(key).or_default().add(it.adc, file_idx);
        }
    })?;

    let channels: Vec<PedestalChannel> = accum
        .into_iter()
        .map(|((asad, aget, ch), a)| PedestalChannel {
            asad,
            aget,
            ch,
            mean: a.mean(),
            rms: a.rms(),
            runs: a.files.len() as u32,
            is_fpn: is_fpn(ch),
        })
        .collect();

    Ok(PedestalTable {
        meta: Meta {
            inputs: paths.iter().map(|p| p.display().to_string()).collect(),
            events: stats.events,
            generated_at: now_rfc3339(),
            tool_version: tool_version(),
        },
        channels,
    })
}

pub fn write(table: &PedestalTable, out: &Path) -> Result<(), CalibError> {
    write_json(table, out)
}

pub fn read(path: &Path) -> Result<PedestalTable, CalibError> {
    read_json(path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn accum_mean_and_rms_match_hand_calculation() {
        // 手計算: [10,10,10] → mean=10, var=0 → rms=0。
        let mut a = Accum::default();
        for _ in 0..3 {
            a.add(10, 0);
        }
        assert_eq!(a.mean(), 10.0);
        assert_eq!(a.rms(), 0.0);

        // 手計算: [8,10,12] → mean=10, var=((8-10)^2+(10-10)^2+(12-10)^2)/3=8/3, rms=sqrt(8/3)=1.6329931...
        let mut b = Accum::default();
        for v in [8u16, 10, 12] {
            b.add(v, 0);
        }
        assert_eq!(b.mean(), 10.0);
        assert!((b.rms() - (8.0f64 / 3.0).sqrt()).abs() < 1e-9);
    }
}
