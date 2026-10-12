//! CAEN DT1415ET(8 ch Floating Reversible 1kV/1mA Desktop Power Supply for MPGD)読み出し
//! コーデック(User Manual rev.16, 2024-11-12、TCP :1470 / USB VCP 9600 8N1、同一 ASCII)。
//! **純関数のみ(IO 無し)**: MON コマンドの生成と応答の解析、それと ch の ON / OFF だけを持つ。
//!
//! # 書けるのは ch の ON / OFF だけ(SPEC §11 安全境界 v1.42、TODO/124)
//!
//! SET を生成する関数は [`set_power`](`$CMD:SET,CH:X,PAR:ON|OFF`)**1 つだけ**。値の引数を
//! 持たないので、VSET / ISET / RUP / RDWN / TRIP / PDWN / BDILKM / BDCLR / BDCFWR… は
//! **型として作れない**(電圧値・電流上限・ランプは機器側で設定する — ユーザー裁定 2026-10-12)。
//! それ以外は読み出し(`$CMD:MON,...`)のみ。
//!
//! # Command Format(User Manual rev.16 p.22)
//!
//! 要求: `$CMD:MON,CH:X,PAR:YYY<CR,LF>`(チャンネル単位)/ `$CMD:MON,PAR:YYY<CR,LF>`(モジュール単位)。
//! `X` は `0..NUMCH`(個別 ch)または `NUMCH`(全 ch まとめて読む、p.22 "CH = NUMCH command
//! related to all channels (group)")。**このチケットの対象 DT1415ET は NUMCH = 8 固定**
//! (p.22 "NUMCH = 8" — 型名にモデル名を焼き込んでいる時点でこのモジュールは DT1415ET 専用)。
//!
//! 応答: `#CMD:OK,VAL:*<CR,LF>`(単 ch)/ `#CMD:OK,VAL:*;*;*;*<CR,LF>`(全 ch、p.22)。
//! エラー(p.22): `#CMD:ERR` / `#CH:ERR` / `#PAR:ERR` / `#VAL:ERR` / `#LOC:ERR`。

use std::io::{self, Read, Write};

/// DT1415ET のチャンネル数(User Manual rev.16 p.22 "NUMCH = 8")。
pub const NUM_CH: u8 = 8;

/// `CH` フィールドにこの値を入れると全チャンネルまとめて読む(p.22 "CH = NUMCH command
/// related to all channels (group)")。DT1415ET では `NUM_CH` と同じ値(= 8)。
pub const ALL_CH: u8 = NUM_CH;

/// チャンネル単位の MON パラメータ(User Manual rev.16 p.22–23 の表。**読み出し用のみ**
/// — SET 側にしか無いパラメータ(VAL 引数を伴う書き込み)はここに存在しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChPar {
    Vset,
    Vmin,
    Vmax,
    Vdec,
    Vres,
    Vmon,
    Iset,
    Imin,
    Imax,
    Imon,
    Imres,
    Isres,
    ImRange,
    ImDec,
    IsDec,
    SwVmax,
    Rup,
    RupMin,
    RupMax,
    RupDec,
    RupRes,
    Rdwn,
    RdwMin,
    RdwMax,
    RdwRes,
    RdwDec,
    Trip,
    TripMin,
    TripMax,
    TripRes,
    TripDec,
    Pdwn,
    Status,
    ChToGr,
    OnOrd,
    OffOrd,
    Zcdtc,
    Zcadj,
}

impl ChPar {
    /// `PAR:` フィールドの文字列(p.22–23 の表、原文の綴りそのまま)。
    fn token(self) -> &'static str {
        match self {
            ChPar::Vset => "VSET",
            ChPar::Vmin => "VMIN",
            ChPar::Vmax => "VMAX",
            ChPar::Vdec => "VDEC",
            ChPar::Vres => "VRES",
            ChPar::Vmon => "VMON",
            ChPar::Iset => "ISET",
            ChPar::Imin => "IMIN",
            ChPar::Imax => "IMAX",
            ChPar::Imon => "IMON",
            ChPar::Imres => "IMRES",
            ChPar::Isres => "ISRES",
            ChPar::ImRange => "IMRANGE",
            ChPar::ImDec => "IMDEC",
            ChPar::IsDec => "ISDEC",
            ChPar::SwVmax => "SWVMAX",
            ChPar::Rup => "RUP",
            ChPar::RupMin => "RUPMIN",
            ChPar::RupMax => "RUPMAX",
            ChPar::RupDec => "RUPDEC",
            ChPar::RupRes => "RUPRES",
            ChPar::Rdwn => "RDWN",
            ChPar::RdwMin => "RDWMIN",
            ChPar::RdwMax => "RDWMAX",
            ChPar::RdwRes => "RDWRES",
            ChPar::RdwDec => "RDWDEC",
            ChPar::Trip => "TRIP",
            ChPar::TripMin => "TRIPMIN",
            ChPar::TripMax => "TRIPMAX",
            ChPar::TripRes => "TRIPRES",
            ChPar::TripDec => "TRIPDEC",
            ChPar::Pdwn => "PDWN",
            ChPar::Status => "STATUS",
            ChPar::ChToGr => "CHTOGR",
            ChPar::OnOrd => "ONORD",
            ChPar::OffOrd => "OFFORD",
            ChPar::Zcdtc => "ZCDTC",
            ChPar::Zcadj => "ZCADJ",
        }
    }
}

/// モジュール単位の MON パラメータ(User Manual rev.16 p.24 の表。`BDCFRD0..4` は複合フォーマット
/// のため対象外 — 発注書 §2 に無い)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BdPar {
    Name,
    Nch,
    Frel,
    Snum,
    Ilk,
    IlkMode,
    Ctr,
    Alarm,
}

impl BdPar {
    fn token(self) -> &'static str {
        match self {
            BdPar::Name => "BDNAME",
            BdPar::Nch => "BDNCH",
            BdPar::Frel => "BDFREL",
            BdPar::Snum => "BDSNUM",
            BdPar::Ilk => "BDILK",
            BdPar::IlkMode => "BDILKM",
            BdPar::Ctr => "BDCTR",
            BdPar::Alarm => "BDALARM",
        }
    }
}

/// チャンネル単位の MON 要求行を組み立てる(p.22: `$CMD:MON,CH:X,PAR:YYY<CR,LF>`)。
/// `ch` は呼び出し側の責任で `0..=ALL_CH` の範囲にする(このモジュールは検証しない —
/// 発注書に範囲検証の要求は無い)。
pub fn mon_ch(ch: u8, par: ChPar) -> String {
    format!("$CMD:MON,CH:{ch},PAR:{}\r\n", par.token())
}

/// モジュール単位の MON 要求行を組み立てる(p.24: `$CMD:MON,PAR:YYY<CR,LF>`)。
pub fn mon_bd(par: BdPar) -> String {
    format!("$CMD:MON,PAR:{}\r\n", par.token())
}

/// ch の ON / OFF(p.25 の SET 表: `$CMD:SET,CH:X,PAR:ON` / `PAR:OFF`、応答は `#CMD:OK`)。
///
/// **このコーデックで唯一の SET 生成関数**(SPEC §11 v1.42、TODO/124)。`ch` の範囲は呼び出し
/// 側が守る —— `CH:8`(= [`ALL_CH`])は**全 ch まとめて**の意味になるので、ドライバは
/// `ch < nch` を確かめてから呼ぶ。
pub fn set_power(ch: u8, on: bool) -> String {
    format!("$CMD:SET,CH:{ch},PAR:{}\r\n", if on { "ON" } else { "OFF" })
}

/// [`parse_reply`] の失敗要因(p.22 のエラー表 + silent failure を作らない CLAUDE.md 方針)。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HvError {
    /// `#CMD:ERR` — Wrong command Format or command not recognized。
    #[error("hivolta #CMD:ERR (wrong command format or command not recognized)")]
    Cmd,
    /// `#VAL:ERR` — Wrong set value (<Min or >Max)。MON 応答としては通常出ないが表にある通り実装。
    #[error("hivolta #VAL:ERR (wrong value)")]
    Val,
    /// `#LOC:ERR` — Command SET with module in LOCAL mode。
    #[error("hivolta #LOC:ERR (command rejected: module in LOCAL mode)")]
    Loc,
    /// `#CH:ERR` — Channel Field not present or wrong Channel value。
    #[error("hivolta #CH:ERR (channel field missing or invalid)")]
    Ch,
    /// `#PAR:ERR` — Field parameter not present or parameter not recognized。
    #[error("hivolta #PAR:ERR (parameter field missing or unrecognized)")]
    Par,
    /// 上記 5 種のどれでもない壊れた行(終端無し・`#` 無し・`VAL:` 無し・値の個数/中身が不正 等)。
    #[error("hivolta malformed reply: {0}")]
    Malformed(String),
}

/// 応答行を復号する。`#CMD:OK,VAL:*` は単一値の `Vec` (len 1)、`#CMD:OK,VAL:*;*;*;*` は
/// セミコロン区切りをそのまま `Vec` に、`#CMD:OK`(VAL 無し)は空 `Vec` を返す。
///
/// 値の中身の意味づけ(数値かどうか、YES/NO か LOCAL/REMOTE か)は呼び出し側が `ChPar`/`BdPar`
/// に応じて行う(このモジュールは文字列のまま返す — ProPar と違い DT1415ET の MON 応答に
/// ワイヤ上の型タグは無いため)。
pub fn parse_reply(line: &str) -> Result<Vec<String>, HvError> {
    let body = line
        .strip_suffix("\r\n")
        .ok_or_else(|| HvError::Malformed(format!("missing CRLF terminator: {line:?}")))?;

    match body {
        "#CMD:ERR" => return Err(HvError::Cmd),
        "#VAL:ERR" => return Err(HvError::Val),
        "#LOC:ERR" => return Err(HvError::Loc),
        "#CH:ERR" => return Err(HvError::Ch),
        "#PAR:ERR" => return Err(HvError::Par),
        "#CMD:OK" => return Ok(Vec::new()),
        _ => {}
    }

    if !body.starts_with('#') {
        return Err(HvError::Malformed(format!("missing '#' prefix: {body:?}")));
    }

    let Some(values) = body.strip_prefix("#CMD:OK,VAL:") else {
        return Err(HvError::Malformed(format!(
            "unrecognized response (missing 'VAL:'): {body:?}"
        )));
    };

    let parts: Vec<String> = values.split(';').map(str::to_string).collect();
    if parts.iter().any(String::is_empty) {
        return Err(HvError::Malformed(format!(
            "value list has an empty entry: {body:?}"
        )));
    }
    Ok(parts)
}

/// `status_ignore` の 1 規則: この ch の指定フラグを alarm 判定から外す(TODO/121 §D)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusIgnore {
    pub ch: u8,
    pub flags: Vec<String>,
}

/// ch ごとの「alarm から外すビット」(index = ch)。
pub type IgnoreMasks = [u16; 8];

/// 規則を ch 別マスクにする。**外せるのは `UNV`(bit5)と `OVV`(bit4)だけ**。
/// それ以外のフラグ名・範囲外の ch はエラー(安全側)。
pub fn ignore_masks(rules: &[StatusIgnore]) -> Result<IgnoreMasks, String> {
    let mut masks = [0u16; 8];
    for rule in rules {
        let slot = masks
            .get_mut(usize::from(rule.ch))
            .ok_or_else(|| format!("status_ignore: ch {} is out of range 0..=7", rule.ch))?;
        for flag in &rule.flags {
            *slot |= match flag.as_str() {
                "OVV" => 1 << 4,
                "UNV" => 1 << 5,
                other => {
                    return Err(format!(
                        "status_ignore: flag {other:?} cannot be ignored (only UNV / OVV)"
                    ))
                }
            };
        }
    }
    Ok(masks)
}

/// 起動時の告知用: マスクが 0 でない ch について `(ch, 無視されるフラグ名)` を返す。
/// alarm を黙らせる設定は黙らせずに告知する(silent 禁止)。
pub fn ignore_notices(masks: &IgnoreMasks) -> Vec<(u8, Vec<&'static str>)> {
    let mut notices = Vec::new();
    for (ch, mask) in (0u8..).zip(masks.iter()) {
        let mut flags = Vec::new();
        if mask & (1 << 4) != 0 {
            flags.push("OVV");
        }
        if mask & (1 << 5) != 0 {
            flags.push("UNV");
        }
        if !flags.is_empty() {
            notices.push((ch, flags));
        }
    }
    notices
}

/// チャンネル STATUS ビットマスク(User Manual rev.16 p.23、10 進数で読む)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChStatus(pub u16);

impl ChStatus {
    /// `ignore` のビットを落とした値(alarm 判定用。出力する STATUS 値には使わない)。
    pub fn without(self, ignore: u16) -> Self {
        Self(self.0 & !ignore)
    }
    pub fn on(self) -> bool {
        self.0 & (1 << 0) != 0
    }
    pub fn ramp_up(self) -> bool {
        self.0 & (1 << 1) != 0
    }
    pub fn ramp_down(self) -> bool {
        self.0 & (1 << 2) != 0
    }
    pub fn ovc(self) -> bool {
        self.0 & (1 << 3) != 0
    }
    pub fn ovv(self) -> bool {
        self.0 & (1 << 4) != 0
    }
    pub fn unv(self) -> bool {
        self.0 & (1 << 5) != 0
    }
    pub fn trip(self) -> bool {
        self.0 & (1 << 6) != 0
    }
    pub fn ovp(self) -> bool {
        self.0 & (1 << 7) != 0
    }
    pub fn twn(self) -> bool {
        self.0 & (1 << 8) != 0
    }
    pub fn ovt(self) -> bool {
        self.0 & (1 << 9) != 0
    }
    pub fn kill(self) -> bool {
        self.0 & (1 << 10) != 0
    }
    pub fn intlk(self) -> bool {
        self.0 & (1 << 11) != 0
    }
    pub fn isdis(self) -> bool {
        self.0 & (1 << 12) != 0
    }
    pub fn fail(self) -> bool {
        self.0 & (1 << 13) != 0
    }
    pub fn lock(self) -> bool {
        self.0 & (1 << 14) != 0
    }

    /// 立っているビットの名前を bit 順に並べる(power 応答の `flags`、TODO/124)。
    ///
    /// 綴りは UI の STATUS 表(`ui/src/app/slow/slow-table.ts` の `STATUS_BITS`)と同じ。
    /// 名前の無い bit 15 は `B15` として出す(黙って捨てない)。
    pub fn flag_names(self) -> Vec<&'static str> {
        const NAMES: [&str; 16] = [
            "ON", "UP", "DOWN", "OVC", "OVV", "UNV", "TRIP", "OVP", "TWN", "OVT", "KILL", "INTLK",
            "DIS", "FAIL", "LOCK", "B15",
        ];
        (0u16..)
            .zip(NAMES)
            .filter(|(bit, _)| self.0 & (1 << bit) != 0)
            .map(|(_, name)| name)
            .collect()
    }
}

/// `PAR:STATUS` の VAL(10 進文字列)を復号する。
pub fn parse_status(s: &str) -> Result<ChStatus, HvError> {
    s.trim()
        .parse::<u16>()
        .map(ChStatus)
        .map_err(|_| HvError::Malformed(format!("bad STATUS value: {s:?}")))
}

/// モジュール BDALARM ビットマスク(User Manual rev.16 p.24 "Meaning of Board Alarm value"、
/// マスク `0x22C0` で定義された 4 ビットのみ意味を持つ)。
///
/// **解釈注記**(完了報告に記載): p.24 の表は値を `0x22C0` と 16 進で書くが、これは概念的な
/// ビットマスクの提示であり、実際のワイヤ上の読み値が 16 進 ASCII か 10 進 ASCII かは明記が無い。
/// `$CMD:MON,PAR:BDALARM` の応答フォーマット行は `( XXXXX )` — p.23 の STATUS(10 進と明記)と
/// 同じプレースホルダ表記であるため、本実装は BDALARM も 10 進 ASCII として読む(`parse_status`
/// と同じ復号)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BdAlarm(pub u16);

impl BdAlarm {
    pub fn trip(self) -> bool {
        self.0 & (1 << 6) != 0
    }
    pub fn ovp(self) -> bool {
        self.0 & (1 << 7) != 0
    }
    pub fn ovt(self) -> bool {
        self.0 & (1 << 9) != 0
    }
    pub fn int_comm_fail(self) -> bool {
        self.0 & (1 << 13) != 0
    }
}

/// `PAR:BDALARM` の VAL(10 進文字列。上の解釈注記参照)を復号する。
pub fn parse_alarm(s: &str) -> Result<BdAlarm, HvError> {
    s.trim()
        .parse::<u16>()
        .map(BdAlarm)
        .map_err(|_| HvError::Malformed(format!("bad BDALARM value: {s:?}")))
}

/// [`HvLink::exchange`] の失敗要因(IO エラーとプロトコルエラーを吸収する。079 の
/// `ProParLink`/`LinkError` と同じ形)。
#[derive(Debug, thiserror::Error)]
pub enum HvLinkError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("hivolta error: {0}")]
    Protocol(#[from] HvError),
}

/// 同期 HiVolta リンク。1 要求送って 1 応答(`<CR,LF>` まで)読む。トランスポート(TCP :1470 /
/// USB VCP)は呼び出し側が `T` として渡す(Phase 1 の `ProParLink` と同じ形。component 配線は
/// 別チケット — 本票では触らない)。
pub struct HvLink<T: Read + Write> {
    io: T,
}

impl<T: Read + Write> HvLink<T> {
    pub fn new(io: T) -> Self {
        Self { io }
    }

    /// `line`(`mon_ch`/`mon_bd` の出力そのもの、CRLF 込み)を送って 1 行読み、復号する。
    /// タイムアウトの実時間は呼び出し側が `T`(例: TCP ソケットの read timeout)で設定する。
    pub fn exchange(&mut self, line: &str) -> Result<Vec<String>, HvLinkError> {
        self.io.write_all(line.as_bytes())?;

        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.io.read(&mut byte) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "hivolta: EOF before CRLF",
                    )
                    .into());
                }
                Ok(_) => {
                    buf.push(byte[0]);
                    if buf.ends_with(b"\r\n") {
                        break;
                    }
                    if buf.len() > 256 {
                        return Err(
                            HvError::Malformed("reply exceeded 256 bytes".to_string()).into()
                        );
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    return Err(e.into());
                }
                Err(e) => return Err(e.into()),
            }
        }

        let reply_line = String::from_utf8_lossy(&buf).into_owned();
        parse_reply(&reply_line).map_err(Into::into)
    }
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。User Manual rev.16 pp.22–25 の実例をゴールデンにする。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// 告知は mask != 0 の ch だけ。ch 2 = UNV、ch 5 = OVV+UNV(bit4=16, bit5=32)。
    #[test]
    fn ignore_notices_lists_only_channels_with_a_mask() {
        let mut masks: IgnoreMasks = [0; 8];
        masks[2] = 1 << 5;
        masks[5] = (1 << 4) | (1 << 5);
        assert_eq!(
            ignore_notices(&masks),
            vec![(2, vec!["UNV"]), (5, vec!["OVV", "UNV"])]
        );
        assert!(ignore_notices(&[0; 8]).is_empty());
    }

    // ---- ①要求文字列の完全一致(ch 単体 / 全 ch / ボード)。p.22 の表そのまま。 ----

    #[test]
    fn mon_ch_single_channel_matches_manual_template() {
        // p.22: "$CMD:MON,CH:X,PAR:VSET" で X=0(個別 ch)。
        assert_eq!(mon_ch(0, ChPar::Vset), "$CMD:MON,CH:0,PAR:VSET\r\n");
    }

    #[test]
    fn mon_ch_all_channels_uses_numch_as_ch_value() {
        // p.22: "CH = NUMCH command related to all channels (group)"、NUMCH = 8。
        assert_eq!(ALL_CH, 8);
        assert_eq!(mon_ch(ALL_CH, ChPar::Vmon), "$CMD:MON,CH:8,PAR:VMON\r\n");
    }

    #[test]
    fn mon_ch_covers_representative_params_from_the_manual_table() {
        assert_eq!(mon_ch(3, ChPar::Iset), "$CMD:MON,CH:3,PAR:ISET\r\n");
        assert_eq!(mon_ch(3, ChPar::Imon), "$CMD:MON,CH:3,PAR:IMON\r\n");
        assert_eq!(mon_ch(3, ChPar::Status), "$CMD:MON,CH:3,PAR:STATUS\r\n");
        assert_eq!(mon_ch(3, ChPar::Trip), "$CMD:MON,CH:3,PAR:TRIP\r\n");
        assert_eq!(mon_ch(3, ChPar::Pdwn), "$CMD:MON,CH:3,PAR:PDWN\r\n");
        assert_eq!(mon_ch(3, ChPar::Rup), "$CMD:MON,CH:3,PAR:RUP\r\n");
        assert_eq!(mon_ch(3, ChPar::Rdwn), "$CMD:MON,CH:3,PAR:RDWN\r\n");
        assert_eq!(mon_ch(3, ChPar::ImRange), "$CMD:MON,CH:3,PAR:IMRANGE\r\n");
    }

    #[test]
    fn mon_bd_matches_manual_template() {
        // p.24 の表そのまま。
        assert_eq!(mon_bd(BdPar::Name), "$CMD:MON,PAR:BDNAME\r\n");
        assert_eq!(mon_bd(BdPar::Nch), "$CMD:MON,PAR:BDNCH\r\n");
        assert_eq!(mon_bd(BdPar::Frel), "$CMD:MON,PAR:BDFREL\r\n");
        assert_eq!(mon_bd(BdPar::Snum), "$CMD:MON,PAR:BDSNUM\r\n");
        assert_eq!(mon_bd(BdPar::Ilk), "$CMD:MON,PAR:BDILK\r\n");
        assert_eq!(mon_bd(BdPar::IlkMode), "$CMD:MON,PAR:BDILKM\r\n");
        assert_eq!(mon_bd(BdPar::Ctr), "$CMD:MON,PAR:BDCTR\r\n");
        assert_eq!(mon_bd(BdPar::Alarm), "$CMD:MON,PAR:BDALARM\r\n");
    }

    // ---- ②応答 `#CMD:OK,VAL:...`(単 / 複数 / 小数 / 文字列値 YES・LOCAL)。p.22。 ----

    #[test]
    fn parse_reply_single_value() {
        assert_eq!(
            parse_reply("#CMD:OK,VAL:3\r\n").unwrap(),
            vec!["3".to_string()]
        );
    }

    #[test]
    fn parse_reply_decimal_value() {
        // p.22: "VAL: value must have a Format compatible with resolution and range"
        // (VSET/VMON 等は小数、単位 V)。
        assert_eq!(
            parse_reply("#CMD:OK,VAL:1000.00\r\n").unwrap(),
            vec!["1000.00".to_string()]
        );
    }

    #[test]
    fn parse_reply_all_channels_semicolon_list() {
        // p.22: "#CMD:OK,VAL:*;*;*;* command Ok *;*;*;* = values Ch0..NUMCH for command to all Channels"。
        // NUMCH=8 の DT1415ET なので 8 値。
        assert_eq!(
            parse_reply("#CMD:OK,VAL:0.00;0.00;0.00;0.00;0.00;0.00;0.00;0.00\r\n").unwrap(),
            vec!["0.00"; 8]
        );
    }

    #[test]
    fn parse_reply_string_value_yes() {
        // p.24: "$CMD:MON,PAR:BDILK Read out INTERLOCK status ( YES/NO )"。
        assert_eq!(
            parse_reply("#CMD:OK,VAL:YES\r\n").unwrap(),
            vec!["YES".to_string()]
        );
    }

    #[test]
    fn parse_reply_string_value_local() {
        // p.24: "$CMD:MON,PAR:BDCTR Read out Control Mode (LOCAL/REMOTE )"。
        assert_eq!(
            parse_reply("#CMD:OK,VAL:LOCAL\r\n").unwrap(),
            vec!["LOCAL".to_string()]
        );
    }

    #[test]
    fn parse_reply_bare_ok_has_no_values() {
        // p.22: "#CMD:OK command Ok"(VAL 無し)。
        assert_eq!(parse_reply("#CMD:OK\r\n").unwrap(), Vec::<String>::new());
    }

    // ---- ③全エラー種(p.22)。 ----

    #[test]
    fn all_error_strings_are_recognized() {
        assert_eq!(parse_reply("#CMD:ERR\r\n").unwrap_err(), HvError::Cmd);
        assert_eq!(parse_reply("#CH:ERR\r\n").unwrap_err(), HvError::Ch);
        assert_eq!(parse_reply("#PAR:ERR\r\n").unwrap_err(), HvError::Par);
        assert_eq!(parse_reply("#VAL:ERR\r\n").unwrap_err(), HvError::Val);
        assert_eq!(parse_reply("#LOC:ERR\r\n").unwrap_err(), HvError::Loc);
    }

    // ---- ④壊れ行(終端無し・`#` 無し・`VAL:` 無し・値の個数/中身が不正)。 ----

    #[test]
    fn missing_crlf_terminator_is_malformed() {
        let err = parse_reply("#CMD:OK,VAL:3").unwrap_err();
        assert!(matches!(err, HvError::Malformed(_)));
    }

    #[test]
    fn missing_hash_prefix_is_malformed() {
        let err = parse_reply("CMD:OK,VAL:3\r\n").unwrap_err();
        assert!(matches!(err, HvError::Malformed(_)));
    }

    #[test]
    fn missing_val_prefix_is_malformed() {
        let err = parse_reply("#CMD:OK,FOO:3\r\n").unwrap_err();
        assert!(matches!(err, HvError::Malformed(_)));
    }

    #[test]
    fn empty_value_entry_in_list_is_malformed() {
        // "0;;0" — 全 ch 応答のはずが途中の値が空(壊れたフレーム)。
        let err = parse_reply("#CMD:OK,VAL:0;;0\r\n").unwrap_err();
        assert!(matches!(err, HvError::Malformed(_)));
    }

    // ---- ⑤STATUS: p.23 の表の全 15 ビットを 1 つずつ立てて述語を検証。 ----

    #[test]
    fn status_bits_match_the_manual_table_row_by_row() {
        type BitRow = (u16, fn(ChStatus) -> bool);
        let table: [BitRow; 15] = [
            (0, ChStatus::on),
            (1, ChStatus::ramp_up),
            (2, ChStatus::ramp_down),
            (3, ChStatus::ovc),
            (4, ChStatus::ovv),
            (5, ChStatus::unv),
            (6, ChStatus::trip),
            (7, ChStatus::ovp),
            (8, ChStatus::twn),
            (9, ChStatus::ovt),
            (10, ChStatus::kill),
            (11, ChStatus::intlk),
            (12, ChStatus::isdis),
            (13, ChStatus::fail),
            (14, ChStatus::lock),
        ];

        for (bit, predicate) in table {
            let status = parse_status(&(1u16 << bit).to_string()).unwrap();
            assert!(
                predicate(status),
                "bit {bit} should make its predicate true"
            );
            // 他のビットは全部 0 なので、全述語のうち true になるのは 1 個だけのはず。
            let true_count = table.iter().filter(|(_, p)| p(status)).count();
            assert_eq!(true_count, 1, "only bit {bit}'s predicate should be true");
        }
    }

    #[test]
    fn status_all_zero_is_all_false() {
        let status = parse_status("0").unwrap();
        assert!(!status.on());
        assert!(!status.trip());
        assert!(!status.lock());
    }

    #[test]
    fn status_malformed_value_is_err() {
        assert!(parse_status("not-a-number").is_err());
    }

    // ---- ⑥BDALARM: p.24 "Meaning of Board Alarm value" 表の全 4 ビット。 ----

    #[test]
    fn bdalarm_bits_match_the_manual_table_row_by_row() {
        type BitRow = (u16, fn(BdAlarm) -> bool);
        let table: [BitRow; 4] = [
            (6, BdAlarm::trip),
            (7, BdAlarm::ovp),
            (9, BdAlarm::ovt),
            (13, BdAlarm::int_comm_fail),
        ];

        for (bit, predicate) in table {
            let alarm = parse_alarm(&(1u16 << bit).to_string()).unwrap();
            assert!(predicate(alarm), "bit {bit} should make its predicate true");
        }
    }

    #[test]
    fn bdalarm_full_mask_from_manual_sets_all_four_bits() {
        // p.24: "0x22C0 = alarm bits mask"。手計算: 0x22C0 = 0010_0010_1100_0000b
        //   = bit13 + bit9 + bit7 + bit6 (8192+512+128+64=8896=0x22C0)。
        let alarm = parse_alarm("8896").unwrap();
        assert!(alarm.trip());
        assert!(alarm.ovp());
        assert!(alarm.ovt());
        assert!(alarm.int_comm_fail());
    }

    // ---- ⑧power ON/OFF(TODO/124、SPEC §11 v1.42 の唯一の例外)。p.25 の SET 表。 ----

    /// p.25: `$CMD:SET,CH:X,PAR:ON` / `$CMD:SET,CH:X,PAR:OFF`(終端は p.22 の `<CR,LF>`)。
    #[test]
    fn set_power_matches_the_manual_on_and_off_commands() {
        assert_eq!(set_power(3, true), "$CMD:SET,CH:3,PAR:ON\r\n");
        assert_eq!(set_power(0, false), "$CMD:SET,CH:0,PAR:OFF\r\n");
        assert_eq!(set_power(7, false), "$CMD:SET,CH:7,PAR:OFF\r\n");
    }

    /// `set_power` の**定義域まるごと**(u8 × bool = 512 通り)が PAR:ON / PAR:OFF にしか
    /// 写らない —— 値の引数が無いので VSET / ISET / RUP 等は型として作れない。
    #[test]
    fn set_power_can_only_ever_produce_par_on_or_par_off() {
        for ch in 0..=u8::MAX {
            for on in [true, false] {
                let line = set_power(ch, on);
                let expected =
                    format!("$CMD:SET,CH:{ch},PAR:{}\r\n", if on { "ON" } else { "OFF" });
                assert_eq!(line, expected);
            }
        }
    }

    /// 本体側(テスト節より前)のソースで `"$CMD:SET` を作る文字列リテラルは 1 個だけ
    /// (= [`set_power`])、ドライバ側は 0 個(コーデック経由でしか SET を作らない)。
    /// 「SET を生成する関数を足していない」ことの機械的な見張り。
    #[test]
    fn set_power_is_the_only_set_command_in_the_hivolta_sources() {
        fn production_part(source: &str) -> &str {
            let end = source.find("#[cfg(test)]").unwrap_or(source.len());
            &source[..end]
        }
        let codec = production_part(include_str!("hivolta.rs"));
        let driver = production_part(include_str!("hivolta_driver.rs"));
        assert_eq!(codec.matches("\"$CMD:SET").count(), 1);
        assert_eq!(driver.matches("\"$CMD:SET").count(), 0);
        assert_eq!(driver.matches("CMD:SET").count(), 0);
    }

    /// `flags` は立っているビットの名前(UI の STATUS 表と同じ綴り)。名前の無い bit 15 は `B15`。
    #[test]
    fn flag_names_list_every_set_bit_in_bit_order() {
        assert!(ChStatus(0).flag_names().is_empty());
        // 33 = bit0 + bit5(実機 ch0、TODO/121)。
        assert_eq!(ChStatus(33).flag_names(), vec!["ON", "UNV"]);
        // 4096 = bit12(前面スイッチ OFF)。
        assert_eq!(ChStatus(4096).flag_names(), vec!["DIS"]);
        // 0x7FFF = bit0..=14 全部。
        assert_eq!(
            ChStatus(0x7FFF).flag_names(),
            vec![
                "ON", "UP", "DOWN", "OVC", "OVV", "UNV", "TRIP", "OVP", "TWN", "OVT", "KILL",
                "INTLK", "DIS", "FAIL", "LOCK"
            ]
        );
        // 0x8003 = bit15 + bit1 + bit0。黙って捨てない。
        assert_eq!(ChStatus(0x8003).flag_names(), vec!["ON", "UP", "B15"]);
    }

    // ---- ⑦Link: fake を挟んで 1 往復、空応答(タイムアウト)。生の $CMD:SET は fake 側で確認。 ----

    #[test]
    fn link_exchange_round_trips_through_fake() {
        use crate::slowctl::fake_hv::FakeHv;

        let mut fake = FakeHv::new();
        fake.set_response(mon_ch(0, ChPar::Vmon), "#CMD:OK,VAL:503.20\r\n");

        let mut link = HvLink::new(fake);
        let values = link.exchange(&mon_ch(0, ChPar::Vmon)).unwrap();
        assert_eq!(values, vec!["503.20".to_string()]);
    }

    #[test]
    fn link_exchange_times_out_on_silent_device() {
        use crate::slowctl::fake_hv::FakeHv;

        let mut fake = FakeHv::new();
        fake.set_silent();

        let mut link = HvLink::new(fake);
        let err = link.exchange(&mon_ch(0, ChPar::Vmon)).unwrap_err();
        assert!(matches!(err, HvLinkError::Io(_)));
    }
}
