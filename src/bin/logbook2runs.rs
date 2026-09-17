//! logbook2runs — DAQ の `logbook.jsonl`(SPEC §9)から minitpc-analysis
//! `runs/runs.yaml` を生成する CLI(TODO/082)。
//!
//! 使い方: `logbook2runs <logbook.jsonl> [--defaults runs_defaults.toml] \
//! [--out runs.yaml] [--data-root <dir>]`
//!
//! 変換ロジックは純関数 [`tpcdaq::runs_yaml`] にあり、この bin は
//! 引数解釈・ファイル読み書き・**silent failure を作らない**ための stderr 報告
//! (壊れた JSONL 行 / 組み立て中の警告)だけを担う。

use std::process::ExitCode;

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args::parse(&raw_args) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("logbook2runs: {msg}");
            eprintln!("{}", args::USAGE);
            return ExitCode::from(2);
        }
    };

    match run(&parsed) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("logbook2runs: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &args::Args) -> Result<(), String> {
    let content = std::fs::read_to_string(&args.logbook)
        .map_err(|e| format!("failed to read {}: {e}", args.logbook.display()))?;

    let (lines, corrupt) = tpcdaq::runs_yaml::parse_logbook_lines(&content);
    for c in &corrupt {
        eprintln!(
            "logbook2runs: {}:{}: corrupt line skipped: {}",
            args.logbook.display(),
            c.line_no,
            c.message
        );
    }
    if !corrupt.is_empty() {
        eprintln!("logbook2runs: {} corrupt line(s) skipped", corrupt.len());
    }

    let defaults = match &args.defaults {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            tpcdaq::runs_yaml::parse_defaults_toml(&text)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?
        }
        None => tpcdaq::runs_yaml::Defaults::default(),
    };

    let data_root = match &args.data_root {
        Some(d) => d.clone(),
        None => args
            .logbook
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".to_string()),
    };

    let (doc, warnings) = tpcdaq::runs_yaml::build_runs_doc(&lines, &defaults, &data_root);
    for w in &warnings {
        eprintln!("logbook2runs: {w}");
    }

    let yaml = tpcdaq::runs_yaml::render(&doc);

    match &args.out {
        Some(path) => {
            std::fs::write(path, &yaml)
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
        }
        None => {
            print!("{yaml}");
        }
    }

    Ok(())
}

mod args {
    use std::path::PathBuf;

    pub const USAGE: &str = "usage: logbook2runs <logbook.jsonl> [--defaults runs_defaults.toml] \
         [--out runs.yaml] [--data-root <dir>]";

    #[derive(Debug, Clone, PartialEq)]
    pub struct Args {
        pub logbook: PathBuf,
        pub defaults: Option<PathBuf>,
        pub out: Option<PathBuf>,
        pub data_root: Option<String>,
    }

    /// `raw`(プログラム名を除いた argv)を解釈する。
    pub fn parse(raw: &[String]) -> Result<Args, String> {
        let mut positional: Vec<&String> = Vec::new();
        let mut defaults = None;
        let mut out = None;
        let mut data_root = None;

        let mut iter = raw.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--defaults" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--defaults requires a value".to_string())?;
                    defaults = Some(PathBuf::from(value));
                }
                "--out" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--out requires a value".to_string())?;
                    out = Some(PathBuf::from(value));
                }
                "--data-root" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--data-root requires a value".to_string())?;
                    data_root = Some(value.clone());
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown option: {other}"));
                }
                _ => positional.push(arg),
            }
        }

        if positional.len() != 1 {
            return Err(format!(
                "expected exactly 1 positional argument (logbook.jsonl), got {}",
                positional.len()
            ));
        }

        Ok(Args {
            logbook: PathBuf::from(positional[0]),
            defaults,
            out,
            data_root,
        })
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used)]
    mod tests {
        use super::*;

        #[test]
        fn parses_positional_only() {
            let raw = vec!["logbook.jsonl".to_string()];
            let a = parse(&raw).unwrap();
            assert_eq!(a.logbook, PathBuf::from("logbook.jsonl"));
            assert_eq!(a.defaults, None);
            assert_eq!(a.out, None);
            assert_eq!(a.data_root, None);
        }

        #[test]
        fn parses_all_flags() {
            let raw = vec![
                "logbook.jsonl".to_string(),
                "--defaults".to_string(),
                "runs_defaults.toml".to_string(),
                "--out".to_string(),
                "runs.yaml".to_string(),
                "--data-root".to_string(),
                "/data/tpcdaq".to_string(),
            ];
            let a = parse(&raw).unwrap();
            assert_eq!(a.logbook, PathBuf::from("logbook.jsonl"));
            assert_eq!(a.defaults, Some(PathBuf::from("runs_defaults.toml")));
            assert_eq!(a.out, Some(PathBuf::from("runs.yaml")));
            assert_eq!(a.data_root, Some("/data/tpcdaq".to_string()));
        }

        #[test]
        fn rejects_unknown_flag() {
            let raw = vec!["logbook.jsonl".to_string(), "--bogus".to_string()];
            assert!(parse(&raw).is_err());
        }

        #[test]
        fn rejects_missing_positional() {
            let raw: Vec<String> = vec![];
            assert!(parse(&raw).is_err());
        }

        #[test]
        fn rejects_a_flag_missing_its_value() {
            let raw = vec!["logbook.jsonl".to_string(), "--out".to_string()];
            assert!(parse(&raw).is_err());
        }
    }
}
