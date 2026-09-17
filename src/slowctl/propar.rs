//! Bronkhorst ProPar ASCII プロトコル(9.17.027 RS232 interface manual §3.3–§3.9)の
//! 純コーデック。**IO は一切持たない**(バイト列 ⇄ [`Request`]/[`Reply`] の変換のみ)。
//!
//! # メッセージ形式(9.17.027 §3.3.3, p.12)
//!
//! `:` + 2 桁 hex(length)+ 2 桁 hex(node)+ 2 桁 hex(command)+ … + `\r\n`。
//! 全バイトは 1 バイナリバイト → 2 桁 hex ASCII に変換される(大文字)。
//! `length` は「length フィールド自身を除く、それ以降の全バイト数」= node + command + payload。
//!
//! 読み出し要求(command `04`)1 項目あたりの構造は §3.8 の "Requesting a parameter" 表どおり:
//! `[return-process][return-parameter][actual-process][actual-parameter][string-length?]`。
//! 応答(command `02`)1 項目あたりは §3.8 の "Answer" 表どおり: `[process][parameter][value...]`。
//!
//! chaining(§3.5)は各バイトの最上位ビット(chain フラグ)で「もう 1 項目続く」ことを示す。
//! process バイトは **同じプロセス番号が続く場合に省略してよい**(マニュアル 9.17.027 §3.9.6 の
//! 実例)。一方で §3.9.7 の実例は同一プロセスでも毎回 process バイトを繰り返している ——
//! この 2 例は矛盾するが、どちらも chain フラグさえ正しく読めば一意に消費バイト数が決まる
//! (「chain=1 なら次の parameter バイトが続く。chain=0 で、かつまだ消費すべき項目が残っていれば
//! 次は必ず新しい process バイト」という読み方は両方の実例と矛盾しない)。したがって
//! **デコード側は省略の有無を判定する必要がない**(chain ビット駆動で自然に両対応する)。
//! エンコード側は KISS のため常に「項目ごとに新しい process バイトを起こす」規約を採用する
//! (これは §3.9.7/§3.9.8 の実例と一致する、より単純な有効なエンコーディング)。
//! 詳細は実装報告(TODO/079 §2.5 完了報告)に記録。

/// パラメータ型(9.17.027 §3.4.2 "Parameter types" 表)。
///
/// `Float` と `Long` はワイヤ上の型タグ(2 bit)が同一(`40h`)で区別できないため、
/// どちらの型として復号するかは呼び出し側が要求時に指定した [`ParType`] に従う
/// (発注書の pseudocode は `parse_reply(line)` だったが、この曖昧さの解消のため
/// 本実装は `parse_reply(line, request)` にした — 逸脱理由は完了報告に記録)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParType {
    /// character(1 byte)。
    U8,
    /// integer(2 byte、big-endian)。
    U16,
    /// long(4 byte、big-endian)。
    U32,
    /// float(4 byte、big-endian、IEEE-754 single precision)。
    F32,
    /// string。読み出し要求時の "expected string length" ヒント(9.17.027 §3.8 Nr8。
    /// 0 = 未指定 — 応答は実際の長さプレフィックスを持つので、このヒントは復号に使わない)。
    Str(u8),
}

/// 読み出し対象パラメータ 1 件(process/param/type)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadItem {
    pub process: u8,
    pub param: u8,
    pub ty: ParType,
}

/// node 宛ての読み出し要求(chaining = `items` 複数)。
///
/// **書き込みコマンド(command `01`/`02`)は型として存在しない**(TODO/079 §1.5 安全境界 —
/// slowctl は監視・記録・アラームのみ、setpoint 等の書き込みは web は疎か DAQ からも行わない)。
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub node: u8,
    pub items: Vec<ReadItem>,
}

/// 読み出した値(型は要求した [`ParType`] に従う)。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    U8(u8),
    U16(u16),
    U32(u32),
    F32(f32),
    Str(String),
}

/// 読み出し応答。`values` は `request.items` と同じ順序・同じ個数。
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub node: u8,
    pub values: Vec<Value>,
}

/// [`parse_reply`] の失敗要因。silent failure を作らない(CLAUDE.md)—
/// hex 不正・length 不一致・node 不一致・型不一致は全て `Err` になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProParError {
    /// `:01xx` プロトコルエラー応答(9.17.027 §3.3.3, p.13)。
    #[error("propar protocol error {code:#04x}: {}", protocol_error_message(*code))]
    Protocol { code: u8 },

    /// ステータス応答(9.17.027 §3.6, p.17)。command=00 は本来「書き込み」への返答だが、
    /// 読み出し要求に対してこれが返る場合は常に異常として扱う(status=0 も含め `Err`)。
    #[error("propar status error {code:#04x}: {}", status_message(*code))]
    Status { code: u8 },

    /// 行が `:` で始まらない、または `\r\n` で終わらない。
    #[error("propar framing error: missing ':' prefix or CRLF terminator")]
    Framing,

    /// hex 桁数が奇数(1 バイト = 2 桁のはず)。
    #[error("propar hex payload has an odd digit count")]
    OddHexDigits,

    /// hex として不正な文字を含む。
    #[error("propar payload contains a non-hex character")]
    InvalidHex,

    /// length フィールドと実バイト数が食い違う。
    #[error("propar length field declared {declared} bytes but {actual} were present")]
    LengthMismatch { declared: usize, actual: usize },

    /// 応答の node が要求と異なる。
    #[error("propar reply node {got:#04x} does not match requested node {expected:#04x}")]
    NodeMismatch { expected: u8, got: u8 },

    /// command バイトが `0x00`/`0x02` 以外。
    #[error("propar reply has unexpected command byte {0:#04x}")]
    UnexpectedCommand(u8),

    /// ペイロードが要求した項目数より短い(chaining 復号の途中で尽きた)。
    #[error("propar reply payload is shorter than the requested item list")]
    Truncated,

    /// ワイヤ上の型タグが要求した [`ParType`] と食い違う(9.17.027 §3.4.2)。
    #[error(
        "propar reply type tag for process {process} parameter {param} does not match the request"
    )]
    TypeMismatch { process: u8, param: u8 },
}

/// `:01xx` プロトコルエラーコード表(9.17.027 §3.3.3, p.13)。未知コードでも
/// silent にせず "unknown error code" を返す。
pub fn protocol_error_message(code: u8) -> &'static str {
    match code {
        1 => "general error",
        2 => "general error",
        3 => "propar protocol error",
        4 => "propar protocol error (or CRC error)",
        5 => "destination node address rejected",
        8 => "general error",
        9 => "response message timeout",
        _ => "unknown error code",
    }
}

/// ステータス応答コード表(9.17.027 §3.6, p.17)。
pub fn status_message(code: u8) -> &'static str {
    match code {
        0x00 => "no error",
        0x01 => "process claimed",
        0x02 => "command error",
        0x03 => "process error",
        0x04 => "parameter error",
        0x05 => "parameter type error",
        0x06 => "parameter value error",
        0x07 => "network not active",
        0x08 => "time-out start character",
        0x09 => "time-out serial line",
        0x0A => "hardware memory error",
        0x0B => "node number error",
        0x0C => "general communication error",
        0x0D => "read only parameter",
        0x0E => "error PC-communication",
        0x0F => "no RS232 connection",
        0x10 => "PC out of memory",
        0x11 => "write only parameter",
        0x12 => "system configuration unknown",
        0x13 => "no free node address",
        0x14 => "wrong interface type",
        0x15 => "error serial port connection",
        0x16 => "error opening communication",
        0x17 => "communication error",
        0x18 => "error interface bus master",
        0x19 => "timeout answer",
        0x1A => "no start character",
        0x1B => "error first digit",
        0x1C => "buffer overflow in host",
        0x1D => "buffer overflow",
        0x1E => "no answer found",
        0x1F => "error closing communication",
        0x20 => "synchronisation error",
        0x21 => "send error",
        0x22 => "protocol error",
        0x23 => "buffer overflow in module",
        _ => "unknown status code",
    }
}

/// ワイヤ上の 2bit 型タグ(9.17.027 §3.4.2: character=00 / integer=01 / float,long=10 / string=11)。
fn type_bits(ty: ParType) -> u8 {
    match ty {
        ParType::U8 => 0,
        ParType::U16 => 1,
        ParType::U32 | ParType::F32 => 2,
        ParType::Str(_) => 3,
    }
}

impl Request {
    /// ASCII 行(`:` … `\r\n`)へ符号化する。
    ///
    /// エンコード規約(このモジュールの doc コメント参照): chaining する項目でも
    /// **毎回新しい process バイトを起こす**(9.17.027 §3.9.7/§3.9.8 と同じ、有効な単純形)。
    ///
    /// return-parameter の「index」フィールド(§3.8 Nr5 の `n`)は本来自由な相関タグだが
    /// (応答はこれをそのまま echo するだけで、値の意味には影響しない)、9.17.027 §3.8 の
    /// 注記 "If no index is required, fill in the FBnr." に従い **常に FBnr(= 実際の
    /// parameter 番号)を index として使う**(§3.9.5 の大半の実例と一致する規約)。
    pub fn to_ascii_line(&self) -> String {
        let mut payload = Vec::new();
        for item in &self.items {
            let t = type_bits(item.ty);
            let param_byte = (t << 5) | (item.param & 0x1F);
            payload.push(item.process & 0x7F); // return-process(chain=0)
            payload.push(param_byte); // return-parameter(index = FBnr、§3.8 の注記どおり)
            payload.push(item.process & 0x7F); // actual-process
            payload.push(param_byte); // actual-parameter(FBnr.)
            if let ParType::Str(len) = item.ty {
                payload.push(len);
            }
        }

        let mut body = Vec::with_capacity(2 + payload.len());
        body.push(self.node);
        body.push(0x04); // command: Request parameter(9.17.027 §3.4.1)
        body.extend_from_slice(&payload);

        let length = body.len() as u8; // 9.17.027 §3.3.1: HOST 実装のデータ部は最大 64 byte
        let mut out = String::with_capacity(1 + 2 + body.len() * 2 + 2);
        out.push(':');
        out.push_str(&format!("{length:02X}"));
        for b in &body {
            out.push_str(&format!("{b:02X}"));
        }
        out.push_str("\r\n");
        out
    }
}

/// `line` を hex デコードする。奇数桁・非 hex 文字は `Err`(silent 禁止)。
fn hex_decode(s: &str) -> Result<Vec<u8>, ProParError> {
    if !s.len().is_multiple_of(2) {
        return Err(ProParError::OddHexDigits);
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i]).ok_or(ProParError::InvalidHex)?;
        let lo = hex_val(bytes[i + 1]).ok_or(ProParError::InvalidHex)?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// `request.items` の型に従って chaining ペイロードを順に復号する。
///
/// process バイトの省略有無に関わらず動く(このモジュールの doc コメント参照): 直前の
/// parameter バイトの chain ビットが 0 のときだけ、次の項目の前に process バイトを 1 つ消費する。
fn decode_items(payload: &[u8], items: &[ReadItem]) -> Result<Vec<Value>, ProParError> {
    let mut idx = 0usize;
    let mut expect_process = true;
    let mut values = Vec::with_capacity(items.len());

    for item in items {
        if expect_process {
            if idx >= payload.len() {
                return Err(ProParError::Truncated);
            }
            idx += 1; // process バイト(エコー情報。値としては使わない)
        }

        let param_byte = *payload.get(idx).ok_or(ProParError::Truncated)?;
        idx += 1;
        let chain = param_byte & 0x80 != 0;
        let wire_type = (param_byte >> 5) & 0x03;
        if wire_type != type_bits(item.ty) {
            return Err(ProParError::TypeMismatch {
                process: item.process,
                param: item.param,
            });
        }

        let value = match item.ty {
            ParType::U8 => {
                let b = *payload.get(idx).ok_or(ProParError::Truncated)?;
                idx += 1;
                Value::U8(b)
            }
            ParType::U16 => {
                let s = payload.get(idx..idx + 2).ok_or(ProParError::Truncated)?;
                idx += 2;
                Value::U16(u16::from_be_bytes([s[0], s[1]]))
            }
            ParType::U32 => {
                let s = payload.get(idx..idx + 4).ok_or(ProParError::Truncated)?;
                idx += 4;
                Value::U32(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
            }
            ParType::F32 => {
                let s = payload.get(idx..idx + 4).ok_or(ProParError::Truncated)?;
                idx += 4;
                Value::F32(f32::from_bits(u32::from_be_bytes([s[0], s[1], s[2], s[3]])))
            }
            ParType::Str(_) => {
                let len = *payload.get(idx).ok_or(ProParError::Truncated)? as usize;
                idx += 1;
                let raw = payload.get(idx..idx + len).ok_or(ProParError::Truncated)?;
                idx += len;
                // 9.17.027 の実例は NUL 終端(serial number)と空白パディング(fluidname /
                // capacity unit)の両方を使う。どちらも表示上は末尾を落とす。
                let s = String::from_utf8_lossy(raw).into_owned();
                let trimmed = s.trim_end_matches(['\0', ' ']).to_string();
                Value::Str(trimmed)
            }
        };
        values.push(value);
        expect_process = !chain;
    }

    Ok(values)
}

/// ASCII 応答行を復号する。
///
/// - `:01xx` の 1 byte エラー応答(§3.3.3)/ status 応答(§3.6)/ length 不一致 / hex 不正 /
///   node 不一致 / 型不一致は全て `Err`(silent 禁止)。
/// - `request` は応答の型解釈に使う(モジュール doc コメント参照 — Float/Long のワイヤ上
///   区別不能性のため必須)。
pub fn parse_reply(line: &str, request: &Request) -> Result<Reply, ProParError> {
    let body = line.strip_suffix("\r\n").ok_or(ProParError::Framing)?;
    let hex = body.strip_prefix(':').ok_or(ProParError::Framing)?;
    let bytes = hex_decode(hex)?;

    let length = *bytes.first().ok_or(ProParError::Truncated)? as usize;

    // 9.17.027 §3.3.3: 特殊な 1 byte エラー応答(node フィールドを持たない)。
    if length == 1 {
        if bytes.len() != 2 {
            return Err(ProParError::LengthMismatch {
                declared: 1,
                actual: bytes.len() - 1,
            });
        }
        return Err(ProParError::Protocol { code: bytes[1] });
    }

    if bytes.len() != 1 + length {
        return Err(ProParError::LengthMismatch {
            declared: length,
            actual: bytes.len() - 1,
        });
    }

    let node = bytes[1];
    if node != request.node {
        return Err(ProParError::NodeMismatch {
            expected: request.node,
            got: node,
        });
    }

    let command = bytes[2];
    match command {
        0x00 => {
            let status = *bytes.get(3).ok_or(ProParError::Truncated)?;
            Err(ProParError::Status { code: status })
        }
        0x02 => {
            let payload = &bytes[3..];
            let values = decode_items(payload, &request.items)?;
            Ok(Reply { node, values })
        }
        other => Err(ProParError::UnexpectedCommand(other)),
    }
}

// ---------------------------------------------------------------------
// 便利関数(9.17.023 §4–§8 で照合済みの process/param、TODO/079 §2.2 表)。
// ---------------------------------------------------------------------

/// 起動時 node scan の識別群(113/12, 113/2, 113/3, 113/5, 113/6)。
pub fn ident_request(node: u8) -> Request {
    Request {
        node,
        items: vec![
            ReadItem {
                process: 113,
                param: 12,
                ty: ParType::U8,
            }, // Identification number
            ReadItem {
                process: 113,
                param: 2,
                ty: ParType::Str(0),
            }, // BHT model number
            ReadItem {
                process: 113,
                param: 3,
                ty: ParType::Str(0),
            }, // Serial number
            ReadItem {
                process: 113,
                param: 5,
                ty: ParType::Str(0),
            }, // Firmware version
            ReadItem {
                process: 113,
                param: 6,
                ty: ParType::Str(0),
            }, // User tag
        ],
    }
}

/// レンジ・単位・較正ガスの識別群(1/13, 1/31, 1/17, 1/14)。
pub fn capacity_request(node: u8) -> Request {
    Request {
        node,
        items: vec![
            ReadItem {
                process: 1,
                param: 13,
                ty: ParType::F32,
            }, // Capacity 100%
            ReadItem {
                process: 1,
                param: 31,
                ty: ParType::Str(0),
            }, // Capacity unit
            ReadItem {
                process: 1,
                param: 17,
                ty: ParType::Str(0),
            }, // Fluid name
            ReadItem {
                process: 1,
                param: 14,
                ty: ParType::U8,
            }, // Sensor type
        ],
    }
}

/// 1 Hz 計測群(33/0, 33/3, 1/0, 1/1, 114/1, 33/7, 1/20, 1/4)。
pub fn measure_request(node: u8) -> Request {
    Request {
        node,
        items: vec![
            ReadItem {
                process: 33,
                param: 0,
                ty: ParType::F32,
            }, // fMeasure
            ReadItem {
                process: 33,
                param: 3,
                ty: ParType::F32,
            }, // fSetpoint(読むだけ)
            ReadItem {
                process: 1,
                param: 0,
                ty: ParType::U16,
            }, // Measure(整数版)
            ReadItem {
                process: 1,
                param: 1,
                ty: ParType::U16,
            }, // Setpoint(整数版、読むだけ)
            ReadItem {
                process: 114,
                param: 1,
                ty: ParType::U32,
            }, // Valve output
            ReadItem {
                process: 33,
                param: 7,
                ty: ParType::F32,
            }, // Temperature
            ReadItem {
                process: 1,
                param: 20,
                ty: ParType::U8,
            }, // Alarm info
            ReadItem {
                process: 1,
                param: 4,
                ty: ParType::U8,
            }, // Control mode(読むだけ)
        ],
    }
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。9.17.027 §3.9 の実例を hex ゴールデンにする。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn single_item(node: u8, process: u8, param: u8, ty: ParType) -> Request {
        Request {
            node,
            items: vec![ReadItem { process, param, ty }],
        }
    }

    // ---- ①9.17.027 §3.9 の実例(hex は一字も変えず定数化)。node 128 = 「先頭機器は
    // 常に応答する」アドレス(9.17.027 §3.2, p.11)。 ----

    /// measure: node 128, process 1, parameter 0, type integer(u16)。9.17.027 p.24
    /// (§3.9.5 "NORMAL OPERATION PARAMETERS" 冒頭の例)。request 側の hex(`:06800401210120\r\n`)
    /// は encode の golden には使わない(下の注記参照) — ANSWER だけを golden にする。
    const MEASURE_ANSWER: &str = ":06800201217D00\r\n";
    // measure = hex 7D00 = 32000 = 100%(マニュアル記載値)。

    /// fmeasure: node 128, process 33, parameter 0, type float(f32)。9.17.027 p.24。
    ///
    /// **マニュアルの表記ゆれ**: この Request(node 128 宛て)の直下に印刷されている
    /// "Answer(example" は node バイトが `03`(request の `80` ではない)——
    /// 中身(process/param/value)は自己整合的で正しいため、恐らく §3.9.3/§3.9.4 で
    /// 使った node=3 の単体例をこの collection 節に流用した際の消し忘れ(完了報告に記載)。
    /// decode テストは実際の node バイト(3)に合わせた `Request` で検証する。
    const FMEASURE_REQUEST: &str = ":06800421402140\r\n";
    const FMEASURE_ANSWER: &str = ":0803022140453B8000\r\n";
    // fmeasure = 453B8000。手計算(IEEE-754 single):
    //   0x453B8000 = 0100_0101_0011_1011_1000_0000_0000_0000
    //   sign=0, exponent=1000_1010b=138 → unbiased 138-127=11
    //   mantissa=011_1011_1000_0000_0000_0000b=0x3B8000 → 1+0x3B8000/2^23=1.46484375
    //   1.46484375 × 2^11 = 3000.0 (マニュアル記載値 "dec 3000" と一致)。

    /// capacity unit: node 128, process 1, parameter 31, type string[7]。9.17.027 p.26。
    const CAPACITY_UNIT_REQUEST: &str = ":078004017F017F07\r\n";
    const CAPACITY_UNIT_ANSWER: &str = ":0C8002017F076B672F68202020\r\n";
    // "6B672F68202020" → "kg/h" + 空白 3(固定長 7 バイトへのパディング)。

    /// fluid name: node 128, process 1, parameter 17, type string[10]。9.17.027 p.25。
    const FLUID_NAME_REQUEST: &str = ":078004017101710A\r\n";
    const FLUID_NAME_ANSWER: &str = ":0F800201710A41695220202020202020\r\n";
    // "41695220202020202020" → "AiR" + 空白 7(固定長 10 バイトへのパディング)。

    /// serial number: node 128, process 113, parameter 3, type string(長さ未指定)。9.17.027 p.30。
    const SERIAL_NUMBER_REQUEST: &str = ":0780047163716300\r\n";
    /// **マニュアルの誤植を 1 byte 訂正**したもの(完了報告に記載): 原文は
    /// `:1080027163004D31353231303633344100\r\n` —— length フィールド(0x10=16)や
    /// process/param(0x71/0x63)は正しいが、その直後の「文字列長」バイトが `00`
    /// になっており、続く 11 byte("M15210634A"+NUL)と矛盾する(0 なら文字列は空のはず)。
    /// 同じ node 128・expected-length 未指定(00)で読む bhtmodel/usertag/customer model の
    /// 3 例も同型の矛盾(長さ 00 なのにデータが続く)を持つ —— 個別のタイプミスではなく
    /// この manual の §3.9.5 節が示す慣習(request 側で長さ未指定にすると answer 側の
    /// 長さフィールドも符号として意味を持たない?)である可能性があるが、確証が無いため
    /// 本実装はこれを仕様として採用しない(実装せず報告 — 完了報告参照)。ここでは単に
    /// 「長さバイトは実データ数と一致するはず」という §3.8 の記述どおりに 1 byte だけ
    /// 訂正(0x00→0x0B=11、payload 全体のバイト数 14 = process+param+len+data(11) と整合)。
    const SERIAL_NUMBER_ANSWER: &str = ":10800271630B4D31353231303633344100\r\n";
    // "4D31353231303633344100" → "M15210634A" + NUL 終端。

    /// valve output: node 128, process 114, parameter 1, type long(u32)。9.17.027 p.26。
    ///
    /// fmeasure と同じ表記ゆれ(上の注記参照): Answer の node バイトは `03`。
    const VALVE_OUTPUT_REQUEST: &str = ":06800472417241\r\n";
    const VALVE_OUTPUT_ANSWER: &str = ":0803027241009DDDDD\r\n";
    // 手計算: 0x009DDDDD = 9*16^5 + 13*16^4 + 13*16^3 + 13*16^2 + 13*16 + 13
    //        = 9437184 + 851968 + 53248 + 3328 + 208 + 13 = 10345949
    //        (マニュアル記載値 "dec 10345949 = 61.7%" と一致)。

    /// iostatus: node 128, process 114, parameter 11, type character(u8)。9.17.027 p.31。
    const IOSTATUS_REQUEST: &str = ":068004720B720B\r\n";
    const IOSTATUS_ANSWER: &str = ":058002720B4F\r\n";
    // 手計算: 0x4F = 4*16 + 15 = 79(マニュアル記載値 "dec 79" と一致)。

    // 注記: measure(process1, parameter0)の request 側だけは encode 側の golden 一致を
    // 取っていない。マニュアルのこの実例は return-parameter の index を 1(§3.8 の
    // 「index 不要なら FBnr を入れる」という注記どおりなら 0 のはず)にしており、他の
    // 全ての単体実例(capacity unit/fluidname/serial number/valve output/iostatus/counter/
    // setpoint)とは異なる選択をしている(index は本来自由な相関タグで応答が意味を持たず
    // echo されるだけ — §3.8 の注記本文および §3.9.7 の Answer 節参照)。本実装は
    // 「index = FBnr」で統一しており(モジュール doc コメント参照)、この 1 例だけ
    // 手で index=1 を選ぶ理由がないため追随しない。ANSWER 側の復号(index を見ない)は
    // 下の `decode_measure_answer_matches_manual_value` で golden 一致を確認する。

    #[test]
    fn decode_measure_answer_matches_manual_value() {
        let req = single_item(0x80, 1, 0, ParType::U16);
        let reply = parse_reply(MEASURE_ANSWER, &req).unwrap();
        assert_eq!(reply.node, 0x80);
        assert_eq!(reply.values, vec![Value::U16(32000)]);
    }

    #[test]
    fn encode_fmeasure_request_matches_manual_hex() {
        let req = single_item(0x80, 33, 0, ParType::F32);
        assert_eq!(req.to_ascii_line(), FMEASURE_REQUEST);
    }

    #[test]
    fn decode_fmeasure_answer_matches_hand_computed_float() {
        // node は 3(上の FMEASURE_ANSWER の doc コメント参照 — マニュアルの表記ゆれ)。
        let req = single_item(0x03, 33, 0, ParType::F32);
        let reply = parse_reply(FMEASURE_ANSWER, &req).unwrap();
        assert_eq!(reply.values, vec![Value::F32(3000.0)]);
    }

    #[test]
    fn encode_capacity_unit_request_matches_manual_hex() {
        let req = single_item(0x80, 1, 31, ParType::Str(7));
        assert_eq!(req.to_ascii_line(), CAPACITY_UNIT_REQUEST);
    }

    #[test]
    fn decode_capacity_unit_answer_trims_trailing_spaces() {
        let req = single_item(0x80, 1, 31, ParType::Str(7));
        let reply = parse_reply(CAPACITY_UNIT_ANSWER, &req).unwrap();
        assert_eq!(reply.values, vec![Value::Str("kg/h".to_string())]);
    }

    #[test]
    fn encode_fluid_name_request_matches_manual_hex() {
        let req = single_item(0x80, 1, 17, ParType::Str(10));
        assert_eq!(req.to_ascii_line(), FLUID_NAME_REQUEST);
    }

    #[test]
    fn decode_fluid_name_answer_trims_trailing_spaces() {
        let req = single_item(0x80, 1, 17, ParType::Str(10));
        let reply = parse_reply(FLUID_NAME_ANSWER, &req).unwrap();
        assert_eq!(reply.values, vec![Value::Str("AiR".to_string())]);
    }

    #[test]
    fn encode_serial_number_request_matches_manual_hex() {
        let req = single_item(0x80, 113, 3, ParType::Str(0));
        assert_eq!(req.to_ascii_line(), SERIAL_NUMBER_REQUEST);
    }

    #[test]
    fn decode_serial_number_answer_trims_trailing_nul() {
        let req = single_item(0x80, 113, 3, ParType::Str(0));
        let reply = parse_reply(SERIAL_NUMBER_ANSWER, &req).unwrap();
        assert_eq!(reply.values, vec![Value::Str("M15210634A".to_string())]);
    }

    #[test]
    fn encode_valve_output_request_matches_manual_hex() {
        let req = single_item(0x80, 114, 1, ParType::U32);
        assert_eq!(req.to_ascii_line(), VALVE_OUTPUT_REQUEST);
    }

    #[test]
    fn decode_valve_output_answer_matches_hand_computed_u32() {
        // node は 3(上の VALVE_OUTPUT_ANSWER の doc コメント参照 — マニュアルの表記ゆれ)。
        let req = single_item(0x03, 114, 1, ParType::U32);
        let reply = parse_reply(VALVE_OUTPUT_ANSWER, &req).unwrap();
        assert_eq!(reply.values, vec![Value::U32(10_345_949)]);
    }

    #[test]
    fn encode_iostatus_request_matches_manual_hex() {
        let req = single_item(0x80, 114, 11, ParType::U8);
        assert_eq!(req.to_ascii_line(), IOSTATUS_REQUEST);
    }

    #[test]
    fn decode_iostatus_answer_matches_hand_computed_u8() {
        let req = single_item(0x80, 114, 11, ParType::U8);
        let reply = parse_reply(IOSTATUS_ANSWER, &req).unwrap();
        assert_eq!(reply.values, vec![Value::U8(79)]);
    }

    // ---- ②:01xx エラー表(9.17.027 §3.3.3, p.13)全コード ----

    #[test]
    fn all_protocol_error_codes_are_recognized() {
        // マニュアル表に載っている番号(1,2,3,4,5,8,9)。6,7 は表に無い = 未定義コード。
        for &code in &[1u8, 2, 3, 4, 5, 8, 9] {
            let line = format!(":01{code:02X}\r\n");
            let req = single_item(0x80, 1, 0, ParType::U16);
            let err = parse_reply(&line, &req).unwrap_err();
            assert_eq!(err, ProParError::Protocol { code });
            assert_ne!(protocol_error_message(code), "unknown error code");
        }
    }

    #[test]
    fn undefined_protocol_error_code_is_not_silently_accepted() {
        // 6, 7 はマニュアル表に無い。デコード自体は成功する(コードとして受理)が、
        // メッセージは "unknown" — silent にしない。
        let line = ":0106\r\n";
        let req = single_item(0x80, 1, 0, ParType::U16);
        let err = parse_reply(line, &req).unwrap_err();
        assert_eq!(err, ProParError::Protocol { code: 6 });
        assert_eq!(protocol_error_message(6), "unknown error code");
    }

    // ---- ③float の big-endian 復号(手計算値)。fmeasure(3000.0)で既に検証済みだが、
    // ここでは status 応答経路(command=00)も一緒に確認する。 ----

    #[test]
    fn status_reply_is_err_with_correct_message() {
        // :04 80 00 0D 05 → node128, command00(status), status=0x0D(read only parameter), index=05
        let line = ":0480000D05\r\n";
        let req = single_item(0x80, 1, 1, ParType::U16);
        let err = parse_reply(line, &req).unwrap_err();
        assert_eq!(err, ProParError::Status { code: 0x0D });
        assert_eq!(status_message(0x0D), "read only parameter");
    }

    // ---- ④chaining の複数値(3 値)。9.17.027 §3.9 に「3 値以上」のクリーンな実例が
    // 無いため(§3.9.6 はページ跨ぎの OCR 抽出が信用できず、ゴールデンとして採用しない —
    // 完了報告に記載)、§3.7/§3.8 のビット配置から手で導出した 3 値チェーンを使う。
    // 構成要素は上の①で個別に検証済みの measure(u16)/fmeasure(f32)/capacity unit(string)。
    // ----

    /// 手計算での導出(node=9, items=[measure(1/0,u16), fmeasure(33/0,f32), capacity unit(1/31,str)])。
    /// retparam=actparam=(type<<5)|(FBnr&0x1F)(このモジュールの `to_ascii_line` の規約 — §3.8
    /// 「index 不要なら FBnr を入れる」に従う)。
    /// item1: retproc=01, param_byte=(1<<5)|00=20, actproc=01, actparam=20
    /// item2: retproc=21(33), param_byte=(2<<5)|00=40, actproc=21, actparam=40
    /// item3: retproc=01, param_byte=(3<<5)|1F=7F, actproc=01, actparam=7F, strlen=00
    /// payload = 01200120 21402140 017F017F00(13 byte)→ body=node(09)+cmd(04)+payload=15byte=0x0F
    const CHAIN3_REQUEST: &str = ":0F09040120012021402140017F017F00\r\n";
    /// 応答(§3.8 Answer 表: 1 項目 = process+parameter+value、"actual" ペアは無い。parameter
    /// バイトは request の retparam をそのまま echo する):
    /// item1: proc=01,param=20,value=7D00(32000) item2: proc=21,param=40,value=453B8000(3000.0)
    /// item3: proc=01,param=7F,strlen=07,"6B672F68202020"("kg/h   ")
    /// payload=20byte→ body=node+cmd+payload=22byte=0x16
    const CHAIN3_ANSWER: &str = ":16090201207D002140453B8000017F076B672F68202020\r\n";

    fn chain3_request() -> Request {
        Request {
            node: 9,
            items: vec![
                ReadItem {
                    process: 1,
                    param: 0,
                    ty: ParType::U16,
                },
                ReadItem {
                    process: 33,
                    param: 0,
                    ty: ParType::F32,
                },
                ReadItem {
                    process: 1,
                    param: 31,
                    ty: ParType::Str(0),
                },
            ],
        }
    }

    #[test]
    fn encode_three_value_chain_matches_hand_derived_hex() {
        assert_eq!(chain3_request().to_ascii_line(), CHAIN3_REQUEST);
    }

    #[test]
    fn decode_three_value_chain_returns_values_in_order() {
        let reply = parse_reply(CHAIN3_ANSWER, &chain3_request()).unwrap();
        assert_eq!(
            reply.values,
            vec![
                Value::U16(32000),
                Value::F32(3000.0),
                Value::Str("kg/h".to_string()),
            ]
        );
    }

    // ---- ⑤fake 経由の node scan は link.rs 側のテストで扱う(このモジュールは IO を持たない)。----

    // ---- ⑥壊れた行は Err、かつカウンタが増える契約(malformed 判定そのものはここで検証。
    // カウンタの実測は link.rs のテストが担当)。 ----

    #[test]
    fn length_field_mismatch_is_err_not_panic() {
        // MEASURE_ANSWER の length を 06→07 に壊す(実バイト数は変わらず 6 のまま)。
        let bad = ":07800201217D00\r\n";
        let req = single_item(0x80, 1, 0, ParType::U16);
        let err = parse_reply(bad, &req).unwrap_err();
        assert_eq!(
            err,
            ProParError::LengthMismatch {
                declared: 7,
                actual: 6
            }
        );
    }

    #[test]
    fn odd_hex_digit_count_is_err() {
        // MEASURE_ANSWER の末尾 1 桁を落として奇数桁にする。
        let bad = ":06800201217D0\r\n";
        let req = single_item(0x80, 1, 0, ParType::U16);
        assert_eq!(
            parse_reply(bad, &req).unwrap_err(),
            ProParError::OddHexDigits
        );
    }

    #[test]
    fn non_hex_character_is_err() {
        let bad = ":0G800201217D00\r\n"; // 'G' は hex でない
        let req = single_item(0x80, 1, 0, ParType::U16);
        assert_eq!(parse_reply(bad, &req).unwrap_err(), ProParError::InvalidHex);
    }

    #[test]
    fn missing_cr_is_framing_error() {
        let bad = ":06800201217D00\n"; // CR 抜け(LF のみ)
        let req = single_item(0x80, 1, 0, ParType::U16);
        assert_eq!(parse_reply(bad, &req).unwrap_err(), ProParError::Framing);
    }

    #[test]
    fn missing_colon_prefix_is_framing_error() {
        let bad = "06800201217D00\r\n";
        let req = single_item(0x80, 1, 0, ParType::U16);
        assert_eq!(parse_reply(bad, &req).unwrap_err(), ProParError::Framing);
    }

    #[test]
    fn node_mismatch_is_err() {
        // 応答の node を 80→81 に壊す。
        let bad = ":06810201217D00\r\n";
        let req = single_item(0x80, 1, 0, ParType::U16);
        assert_eq!(
            parse_reply(bad, &req).unwrap_err(),
            ProParError::NodeMismatch {
                expected: 0x80,
                got: 0x81
            }
        );
    }

    #[test]
    fn type_mismatch_is_err() {
        // measure(u16)を要求したのに、応答の型タグが float(10)になっている(0x21→0x41 相当:
        // 実際には値部の長さも変わるため、ここでは素直に型タグだけ変えた壊れ応答を作る)。
        let bad = ":06800201417D00\r\n"; // parameter byte 0x21→0x41(type bits 01→10)
        let req = single_item(0x80, 1, 0, ParType::U16);
        assert_eq!(
            parse_reply(bad, &req).unwrap_err(),
            ProParError::TypeMismatch {
                process: 1,
                param: 0
            }
        );
    }

    // ---- ident_request / capacity_request / measure_request の形(型・個数・順序)----

    #[test]
    fn ident_request_has_five_items_in_spec_order() {
        let req = ident_request(3);
        assert_eq!(req.node, 3);
        let expect: Vec<(u8, u8, ParType)> = vec![
            (113, 12, ParType::U8),
            (113, 2, ParType::Str(0)),
            (113, 3, ParType::Str(0)),
            (113, 5, ParType::Str(0)),
            (113, 6, ParType::Str(0)),
        ];
        let got: Vec<(u8, u8, ParType)> = req
            .items
            .iter()
            .map(|i| (i.process, i.param, i.ty))
            .collect();
        assert_eq!(got, expect);
    }

    #[test]
    fn capacity_request_has_four_items_in_spec_order() {
        let req = capacity_request(3);
        let expect: Vec<(u8, u8, ParType)> = vec![
            (1, 13, ParType::F32),
            (1, 31, ParType::Str(0)),
            (1, 17, ParType::Str(0)),
            (1, 14, ParType::U8),
        ];
        let got: Vec<(u8, u8, ParType)> = req
            .items
            .iter()
            .map(|i| (i.process, i.param, i.ty))
            .collect();
        assert_eq!(got, expect);
    }

    #[test]
    fn measure_request_has_eight_items_in_spec_order() {
        let req = measure_request(3);
        let expect: Vec<(u8, u8, ParType)> = vec![
            (33, 0, ParType::F32),
            (33, 3, ParType::F32),
            (1, 0, ParType::U16),
            (1, 1, ParType::U16),
            (114, 1, ParType::U32),
            (33, 7, ParType::F32),
            (1, 20, ParType::U8),
            (1, 4, ParType::U8),
        ];
        let got: Vec<(u8, u8, ParType)> = req
            .items
            .iter()
            .map(|i| (i.process, i.param, i.ty))
            .collect();
        assert_eq!(got, expect);
    }
}
