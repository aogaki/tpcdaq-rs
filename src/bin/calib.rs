//! `calib` — pedestal / pulser run(実機 .graw)から per-channel の較正表を作る CLI(TODO/081)。
//!
//! ```text
//! calib pedestal <graw...> --out <pedestal.json>
//! calib pulser <graw...> --out <pulser.json>
//! calib merge <pedestal.json> <pulser.json> --out <calib.json> [--k <f64=5.0>]
//! ```
//!
//! フレーム切り出し/デコードは本体 `framer::Framer` / `decode::Decoder` をそのまま使う
//! (asset_survey(`reference/_spike/asset_survey`)の高速経路は持ち込まない — KISS)。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tpcdaq::calib::{pedestal, pulser, table};

const USAGE: &str = "usage:\n  \
    calib pedestal <graw...> --out <pedestal.json>\n  \
    calib pulser <graw...> --out <pulser.json>\n  \
    calib merge <pedestal.json> <pulser.json> --out <calib.json> [--k <f64=5.0>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("calib: {msg}");
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (sub, rest) = args.split_first().ok_or("missing subcommand")?;
    match sub.as_str() {
        "pedestal" => run_pedestal(rest),
        "pulser" => run_pulser(rest),
        "merge" => run_merge(rest),
        other => Err(format!("unknown subcommand: {other}")),
    }
}

/// `<positional...> --out <path>` の共通形(pedestal/pulser サブコマンド)を解く。
fn parse_files_and_out(args: &[String]) -> Result<(Vec<PathBuf>, PathBuf), String> {
    let mut positional = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "--out" {
            let v = iter.next().ok_or("--out requires a value")?;
            out = Some(PathBuf::from(v));
        } else {
            positional.push(PathBuf::from(a));
        }
    }
    let out = out.ok_or("missing --out <path>")?;
    if positional.is_empty() {
        return Err("no input .graw files given".to_string());
    }
    Ok((positional, out))
}

fn run_pedestal(args: &[String]) -> Result<(), String> {
    let (files, out) = parse_files_and_out(args)?;
    let computed = pedestal::compute(&files).map_err(|e| e.to_string())?;
    pedestal::write(&computed, &out).map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} channels, {} events)",
        out.display(),
        computed.channels.len(),
        computed.meta.events
    );
    Ok(())
}

fn run_pulser(args: &[String]) -> Result<(), String> {
    let (files, out) = parse_files_and_out(args)?;
    let computed = pulser::compute(&files).map_err(|e| e.to_string())?;
    pulser::write(&computed, &out).map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} channels, {} events)",
        out.display(),
        computed.channels.len(),
        computed.meta.events
    );
    Ok(())
}

fn run_merge(args: &[String]) -> Result<(), String> {
    let mut positional: Vec<String> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut k: f64 = table::DEFAULT_K;
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--out" => {
                let v = iter.next().ok_or("--out requires a value")?;
                out = Some(PathBuf::from(v));
            }
            "--k" => {
                let v = iter.next().ok_or("--k requires a value")?;
                k = v
                    .parse()
                    .map_err(|_| format!("--k value is not a number: {v}"))?;
            }
            other => positional.push(other.to_string()),
        }
    }
    let out = out.ok_or("missing --out <path>")?;
    if positional.len() != 2 {
        return Err(format!(
            "expected exactly 2 positional args (pedestal.json pulser.json), got {}",
            positional.len()
        ));
    }

    let pedestal_table = pedestal::read(Path::new(&positional[0])).map_err(|e| e.to_string())?;
    let pulser_table = pulser::read(Path::new(&positional[1])).map_err(|e| e.to_string())?;
    let merged = table::merge(&pedestal_table, &pulser_table, k);
    table::write(&merged, &out).map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} channels)",
        out.display(),
        merged.channels.len()
    );
    Ok(())
}
