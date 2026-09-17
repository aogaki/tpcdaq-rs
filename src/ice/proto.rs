//! Ice プロトコル 1.0 のメッセージ。TODO/076 §2.2、
//! 一次資料 = archive.zeroc.com/ice/3.7 「Protocol Messages」。
//!
//! メッセージは **14 B のヘッダ + 本体**。ヘッダは
//! `"IceP"` / protocolMajor 1 / protocolMinor 0 / encodingMajor 1 / encodingMinor 0
//! (ヘッダの encoding は本体の encoding とは別物で、常に 1.0)/ messageType /
//! compressionStatus / messageSize(int32 LE、**ヘッダ込みの総バイト数**)。
//!
//! 壊れ入力(短い / magic 違い / 版数違い / 圧縮 / size 矛盾 / 未知 type)は全部 `Err`。

use std::collections::BTreeMap;
use std::fmt;

use super::codec::{InputStream, OutputStream};
use super::error::IceError;

/// ヘッダ先頭の 4 B。
pub const MAGIC: [u8; 4] = *b"IceP";
pub const PROTOCOL_MAJOR: u8 = 1;
pub const PROTOCOL_MINOR: u8 = 0;
/// ヘッダ内の encoding は常に 1.0(本体の 1.1 とは別)。
pub const HEADER_ENCODING_MAJOR: u8 = 1;
/// ヘッダ内の encoding は常に 1.0(本体の 1.1 とは別)。
pub const HEADER_ENCODING_MINOR: u8 = 0;
/// メッセージヘッダのバイト数。
pub const HEADER_SIZE: usize = 14;
/// 受け入れるメッセージ長の上限(発注書外の安全弁 — 壊れた size で巨大確保しないため)。
/// ECC の最大メッセージは configure の DataLinkSet XML で数 KB。
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// メッセージ種別(ヘッダ byte 8)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Request,
    BatchRequest,
    Reply,
    ValidateConnection,
    CloseConnection,
}

impl MessageType {
    pub fn from_byte(value: u8) -> Result<Self, IceError> {
        match value {
            0 => Ok(MessageType::Request),
            1 => Ok(MessageType::BatchRequest),
            2 => Ok(MessageType::Reply),
            3 => Ok(MessageType::ValidateConnection),
            4 => Ok(MessageType::CloseConnection),
            other => Err(IceError::protocol(format!(
                "unknown message type {other} in the header"
            ))),
        }
    }

    pub fn as_byte(self) -> u8 {
        match self {
            MessageType::Request => 0,
            MessageType::BatchRequest => 1,
            MessageType::Reply => 2,
            MessageType::ValidateConnection => 3,
            MessageType::CloseConnection => 4,
        }
    }
}

/// 14 B のメッセージヘッダ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageHeader {
    pub message_type: MessageType,
    pub compression: u8,
    /// ヘッダ込みの総バイト数。
    pub size: usize,
}

impl MessageHeader {
    /// 先頭 14 B を読む(それより長いバッファを渡してよい)。
    pub fn parse(bytes: &[u8]) -> Result<Self, IceError> {
        let head = bytes.get(..HEADER_SIZE).ok_or_else(|| {
            IceError::protocol(format!(
                "message header needs {HEADER_SIZE} bytes, got {}",
                bytes.len()
            ))
        })?;
        if head[..4] != MAGIC {
            return Err(IceError::protocol(format!(
                "bad magic {:02X?}, expected \"IceP\"",
                &head[..4]
            )));
        }
        if (head[4], head[5]) != (PROTOCOL_MAJOR, PROTOCOL_MINOR) {
            return Err(IceError::protocol(format!(
                "protocol {}.{} is not supported (only 1.0)",
                head[4], head[5]
            )));
        }
        if (head[6], head[7]) != (HEADER_ENCODING_MAJOR, HEADER_ENCODING_MINOR) {
            return Err(IceError::protocol(format!(
                "header encoding {}.{} is not supported (the header is always 1.0)",
                head[6], head[7]
            )));
        }
        let message_type = MessageType::from_byte(head[8])?;
        let compression = head[9];
        if compression != 0 {
            return Err(IceError::protocol(format!(
                "compressed message (compressionStatus {compression}) is not supported"
            )));
        }
        let size = i32::from_le_bytes([head[10], head[11], head[12], head[13]]);
        let size = usize::try_from(size)
            .map_err(|_| IceError::protocol(format!("negative message size {size}")))?;
        if size < HEADER_SIZE {
            return Err(IceError::protocol(format!(
                "message size {size} is smaller than the {HEADER_SIZE} byte header"
            )));
        }
        if size > MAX_MESSAGE_SIZE {
            return Err(IceError::protocol(format!(
                "message size {size} exceeds the {MAX_MESSAGE_SIZE} byte cap"
            )));
        }
        Ok(MessageHeader {
            message_type,
            compression,
            size,
        })
    }
}

/// ヘッダ + 本体のメッセージを組み立てる(size はここで入る)。
pub fn build_message(message_type: MessageType, body: &[u8]) -> Result<Vec<u8>, IceError> {
    let size = HEADER_SIZE + body.len();
    if size > MAX_MESSAGE_SIZE {
        return Err(IceError::protocol(format!(
            "message of {size} bytes exceeds the {MAX_MESSAGE_SIZE} byte cap"
        )));
    }
    let size = i32::try_from(size).map_err(|_| {
        IceError::protocol(format!("message of {size} bytes does not fit in an int"))
    })?;
    let mut out = Vec::with_capacity(HEADER_SIZE + body.len());
    out.extend_from_slice(&MAGIC);
    out.push(PROTOCOL_MAJOR);
    out.push(PROTOCOL_MINOR);
    out.push(HEADER_ENCODING_MAJOR);
    out.push(HEADER_ENCODING_MINOR);
    out.push(message_type.as_byte());
    out.push(0); // 非圧縮(圧縮は使わない)
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

/// ValidateConnection(ヘッダのみ)。サーバが接続直後に送ってくるのと同じ形。
pub fn validate_connection_message() -> Vec<u8> {
    header_only_message(MessageType::ValidateConnection)
}

/// CloseConnection(ヘッダのみ)。
pub fn close_connection_message() -> Vec<u8> {
    header_only_message(MessageType::CloseConnection)
}

/// 本体なしのメッセージ。`build_message` は本体が空なら決して失敗しない。
fn header_only_message(message_type: MessageType) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_SIZE);
    out.extend_from_slice(&MAGIC);
    out.push(PROTOCOL_MAJOR);
    out.push(PROTOCOL_MINOR);
    out.push(HEADER_ENCODING_MAJOR);
    out.push(HEADER_ENCODING_MINOR);
    out.push(message_type.as_byte());
    out.push(0);
    out.extend_from_slice(&(HEADER_SIZE as i32).to_le_bytes());
    out
}

/// Ice のオブジェクト識別子。ECC は `name = "Ecc"` / `category = ""`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub category: String,
}

impl Identity {
    /// category 空の識別子。
    pub fn new(name: impl Into<String>) -> Self {
        Identity {
            name: name.into(),
            category: String::new(),
        }
    }
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.category.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{}/{}", self.category, self.name)
        }
    }
}

/// 呼び出しモード(Request の byte)。1 = Nonmutating は Ice 3.7 で非推奨のため扱わない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationMode {
    Normal,
    Idempotent,
}

impl OperationMode {
    pub fn from_byte(value: u8) -> Result<Self, IceError> {
        match value {
            0 => Ok(OperationMode::Normal),
            2 => Ok(OperationMode::Idempotent),
            other => Err(IceError::protocol(format!(
                "unsupported operation mode {other} (0 = normal, 2 = idempotent)"
            ))),
        }
    }

    pub fn as_byte(self) -> u8 {
        match self {
            OperationMode::Normal => 0,
            OperationMode::Idempotent => 2,
        }
    }
}

/// twoway リクエスト 1 本。`params` は **encapsulation の中身**(6 B ヘッダを含まない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub request_id: i32,
    pub identity: Identity,
    pub facet: String,
    pub operation: String,
    pub mode: OperationMode,
    pub context: BTreeMap<String, String>,
    pub params: Vec<u8>,
}

impl Request {
    /// facet 無し・context 空のリクエスト(我々が出すのはこの形だけ)。
    pub fn new(
        request_id: i32,
        identity: Identity,
        operation: impl Into<String>,
        mode: OperationMode,
        params: Vec<u8>,
    ) -> Self {
        Request {
            request_id,
            identity,
            facet: String::new(),
            operation: operation.into(),
            mode,
            context: BTreeMap::new(),
            params,
        }
    }

    /// ヘッダ込みの完全なメッセージにする。
    pub fn to_bytes(&self) -> Result<Vec<u8>, IceError> {
        let mut out = OutputStream::new();
        out.write_int(self.request_id);
        out.write_string(&self.identity.name)?;
        out.write_string(&self.identity.category)?;
        if self.facet.is_empty() {
            out.write_string_seq(&[])?;
        } else {
            out.write_string_seq(&[self.facet.as_str()])?;
        }
        out.write_string(&self.operation)?;
        out.write_byte(self.mode.as_byte());
        out.write_dict(&self.context)?;
        out.write_encapsulation(&self.params)?;
        build_message(MessageType::Request, out.as_bytes())
    }

    /// ヘッダ込みの完全なメッセージを読む(テストと偽サーバ用)。
    pub fn parse(message: &[u8]) -> Result<Self, IceError> {
        let body = message_body(message, MessageType::Request)?;
        let mut input = InputStream::new(body);
        let request_id = input.read_int()?;
        let identity = Identity {
            name: input.read_string()?,
            category: input.read_string()?,
        };
        let facet = read_facet(&mut input)?;
        let operation = input.read_string()?;
        let mode = OperationMode::from_byte(input.read_byte()?)?;
        let context = input.read_dict()?;
        let params = input.read_encapsulation()?.to_vec();
        expect_end(&input, "request")?;
        Ok(Request {
            request_id,
            identity,
            facet,
            operation,
            mode,
            context,
            params,
        })
    }
}

/// facet は要素 0 か 1 の string seq。2 つ以上は仕様違反。
fn read_facet(input: &mut InputStream<'_>) -> Result<String, IceError> {
    let facets = input.read_string_seq()?;
    match facets.len() {
        0 => Ok(String::new()),
        1 => Ok(facets[0].clone()),
        n => Err(IceError::protocol(format!(
            "facet sequence has {n} elements, expected 0 or 1"
        ))),
    }
}

/// ヘッダを検査して本体を返す(size がバイト数と食い違ったら `Err`)。
fn message_body(message: &[u8], expected: MessageType) -> Result<&[u8], IceError> {
    let header = MessageHeader::parse(message)?;
    if header.message_type != expected {
        return Err(IceError::protocol(format!(
            "expected a {expected:?} message, got {:?}",
            header.message_type
        )));
    }
    if header.size != message.len() {
        return Err(IceError::protocol(format!(
            "header size {} disagrees with the {} bytes on hand",
            header.size,
            message.len()
        )));
    }
    Ok(&message[HEADER_SIZE..])
}

/// 本体を読み切ったことを確かめる(余りは黙って捨てない)。
fn expect_end(input: &InputStream<'_>, what: &str) -> Result<(), IceError> {
    if input.is_empty() {
        return Ok(());
    }
    Err(IceError::protocol(format!(
        "{} bytes left over after the {what} body",
        input.remaining()
    )))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyStatus {
    Ok,
    UserException,
    ObjectNotExist,
    FacetNotExist,
    OperationNotExist,
    UnknownLocalException,
    UnknownUserException,
    UnknownException,
}

impl ReplyStatus {
    pub fn from_byte(value: u8) -> Result<Self, IceError> {
        match value {
            0 => Ok(ReplyStatus::Ok),
            1 => Ok(ReplyStatus::UserException),
            2 => Ok(ReplyStatus::ObjectNotExist),
            3 => Ok(ReplyStatus::FacetNotExist),
            4 => Ok(ReplyStatus::OperationNotExist),
            5 => Ok(ReplyStatus::UnknownLocalException),
            6 => Ok(ReplyStatus::UnknownUserException),
            7 => Ok(ReplyStatus::UnknownException),
            other => Err(IceError::protocol(format!(
                "unknown reply status {other} (0-7 are defined)"
            ))),
        }
    }

    pub fn as_byte(self) -> u8 {
        match self {
            ReplyStatus::Ok => 0,
            ReplyStatus::UserException => 1,
            ReplyStatus::ObjectNotExist => 2,
            ReplyStatus::FacetNotExist => 3,
            ReplyStatus::OperationNotExist => 4,
            ReplyStatus::UnknownLocalException => 5,
            ReplyStatus::UnknownUserException => 6,
            ReplyStatus::UnknownException => 7,
        }
    }
}

impl fmt::Display for ReplyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyBody {
    Ok(Vec<u8>),
    UserException {
        type_id: String,
        body: Vec<u8>,
    },
    RequestFailed {
        status: ReplyStatus,
        identity: Identity,
        facet: String,
        operation: String,
    },
    Unknown {
        status: ReplyStatus,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub request_id: i32,
    pub body: ReplyBody,
}

impl Reply {
    /// ヘッダ込みの完全なメッセージを読む。
    pub fn parse(message: &[u8]) -> Result<Self, IceError> {
        let body = message_body(message, MessageType::Reply)?;
        let mut input = InputStream::new(body);
        let request_id = input.read_int()?;
        let status = ReplyStatus::from_byte(input.read_byte()?)?;
        let body = match status {
            ReplyStatus::Ok => ReplyBody::Ok(input.read_encapsulation()?.to_vec()),
            ReplyStatus::UserException => {
                let (type_id, body) = parse_user_exception(input.read_encapsulation()?)?;
                ReplyBody::UserException { type_id, body }
            }
            ReplyStatus::ObjectNotExist
            | ReplyStatus::FacetNotExist
            | ReplyStatus::OperationNotExist => {
                // RequestFailedData は encapsulation に入らず生で並ぶ。
                let identity = Identity {
                    name: input.read_string()?,
                    category: input.read_string()?,
                };
                let facet = read_facet(&mut input)?;
                let operation = input.read_string()?;
                ReplyBody::RequestFailed {
                    status,
                    identity,
                    facet,
                    operation,
                }
            }
            ReplyStatus::UnknownLocalException
            | ReplyStatus::UnknownUserException
            | ReplyStatus::UnknownException => ReplyBody::Unknown {
                status,
                message: input.read_string()?,
            },
        };
        expect_end(&input, "reply")?;
        Ok(Reply { request_id, body })
    }

    /// ヘッダ込みの完全なメッセージにする(偽サーバとテスト用)。
    pub fn to_bytes(&self) -> Result<Vec<u8>, IceError> {
        let mut out = OutputStream::new();
        out.write_int(self.request_id);
        match &self.body {
            ReplyBody::Ok(payload) => {
                out.write_byte(ReplyStatus::Ok.as_byte());
                out.write_encapsulation(payload)?;
            }
            ReplyBody::UserException { type_id, body } => {
                out.write_byte(ReplyStatus::UserException.as_byte());
                let mut slice = OutputStream::new();
                // compact format(Ice 3.7 の既定): slice 1 枚・slice size なし。
                slice.write_byte(SLICE_FLAG_IS_LAST_SLICE | SLICE_FLAG_HAS_TYPE_ID_STRING);
                slice.write_string(type_id)?;
                slice.write_bytes(body);
                out.write_encapsulation(slice.as_bytes())?;
            }
            ReplyBody::RequestFailed {
                status,
                identity,
                facet,
                operation,
            } => {
                out.write_byte(status.as_byte());
                out.write_string(&identity.name)?;
                out.write_string(&identity.category)?;
                if facet.is_empty() {
                    out.write_string_seq(&[])?;
                } else {
                    out.write_string_seq(&[facet.as_str()])?;
                }
                out.write_string(operation)?;
            }
            ReplyBody::Unknown { status, message } => {
                out.write_byte(status.as_byte());
                out.write_string(message)?;
            }
        }
        build_message(MessageType::Reply, out.as_bytes())
    }

    /// 呼び出しの結果に畳む(Ok なら out encapsulation の中身、それ以外は [`IceError`])。
    pub fn into_result(self) -> Result<Vec<u8>, IceError> {
        match self.body {
            ReplyBody::Ok(payload) => Ok(payload),
            ReplyBody::UserException { type_id, body } => {
                Err(IceError::UserException { type_id, body })
            }
            ReplyBody::RequestFailed {
                status,
                identity,
                facet,
                operation,
            } => Err(IceError::RequestFailed {
                kind: status,
                identity,
                facet,
                operation,
            }),
            ReplyBody::Unknown { status, message } => Err(IceError::Unknown {
                kind: status,
                message,
            }),
        }
    }
}

/// エンコーディング 1.1 の slice flags(bit0 = type id が文字列 / bit1 = index /
/// bit4 = slice size あり / bit5 = 最終 slice)。TODO/076 §2.2。
pub const SLICE_FLAG_HAS_TYPE_ID_STRING: u8 = 0x01;
/// slice flags bit1。
pub const SLICE_FLAG_HAS_TYPE_ID_INDEX: u8 = 0x02;
/// slice flags bit4。
pub const SLICE_FLAG_HAS_SLICE_SIZE: u8 = 0x10;
/// slice flags bit5。
pub const SLICE_FLAG_IS_LAST_SLICE: u8 = 0x20;

/// エンコーディング 1.1 の例外 encapsulation から **最初の slice の type id** と
/// その後ろの生バイトを取り出す。
///
/// - 例外 slice の type id は **常に string**(Ice 自身の decoder も flags の
///   type-id ビットを見ずに string を読む)。compact / sliced のどちらでも同じ。
/// - `SLICE_FLAG_HAS_SLICE_SIZE`(sliced format)なら type id の後ろに int32 の
///   slice size が入るので読み飛ばす。**size の数え方(自分自身を含むか)には
///   解釈幅があるため、読み飛ばしの寸法には使わない** — メンバはそのまま続く。
/// - メンバの解釈(CmdException なら `reason` の string)は呼び手の仕事。
pub fn parse_user_exception(payload: &[u8]) -> Result<(String, Vec<u8>), IceError> {
    let mut input = InputStream::new(payload);
    let flags = input.read_byte()?;
    let type_id = input.read_string()?;
    if flags & SLICE_FLAG_HAS_SLICE_SIZE != 0 {
        let _slice_size = input.read_int()?;
    }
    Ok((type_id, input.read_remaining().to_vec()))
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 期待バイトは Ice 3.7 Protocol Messages から手で起こす)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// `"::mdaq::utl::CmdException"` = 25 B(reference/20190315_patched の
    /// MDaq/src/mdaq/utl/CmdException.ice。継承なし = slice 1 枚)。
    const CMD_EXCEPTION: &str = "::mdaq::utl::CmdException";

    #[test]
    fn a_validate_connection_message_is_the_bare_header() {
        assert_eq!(
            validate_connection_message(),
            vec![
                0x49, 0x63, 0x65, 0x50, // "IceP"
                0x01, 0x00, // プロトコル 1.0
                0x01, 0x00, // ヘッダの encoding は常に 1.0
                0x03, // type 3 = ValidateConnection
                0x00, // 非圧縮
                0x0E, 0x00, 0x00, 0x00, // size = 14(ヘッダのみ)
            ]
        );
    }

    #[test]
    fn a_close_connection_message_is_the_bare_header() {
        let bytes = close_connection_message();
        assert_eq!(bytes[8], 0x04);
        assert_eq!(bytes.len(), HEADER_SIZE);
        let header = MessageHeader::parse(&bytes).unwrap();
        assert_eq!(header.message_type, MessageType::CloseConnection);
        assert_eq!(header.size, HEADER_SIZE);
    }

    #[test]
    fn a_header_with_a_foreign_magic_is_rejected() {
        let mut bytes = validate_connection_message();
        bytes[0] = b'X';
        let err = MessageHeader::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("magic"), "{err}");
    }

    #[test]
    fn a_header_with_another_protocol_version_is_rejected() {
        let mut bytes = validate_connection_message();
        bytes[5] = 0x01; // protocolMinor = 1
        let err = MessageHeader::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("protocol"), "{err}");
    }

    #[test]
    fn a_compressed_message_is_rejected_loudly() {
        let mut bytes = validate_connection_message();
        bytes[9] = 0x02; // compressionStatus
        let err = MessageHeader::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("compress"), "{err}");
    }

    #[test]
    fn a_header_shorter_than_fourteen_bytes_is_rejected() {
        let bytes = validate_connection_message();
        assert!(MessageHeader::parse(&bytes[..13]).is_err());
    }

    #[test]
    fn a_size_below_the_header_or_beyond_the_cap_is_rejected() {
        let mut bytes = validate_connection_message();
        bytes[10] = 0x0D; // size = 13
        assert!(MessageHeader::parse(&bytes).is_err());

        let mut bytes = validate_connection_message();
        bytes[10..14].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(MessageHeader::parse(&bytes).is_err());

        let mut bytes = validate_connection_message();
        bytes[10..14].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(MessageHeader::parse(&bytes).is_err());
    }

    #[test]
    fn an_unknown_message_type_is_rejected() {
        let mut bytes = validate_connection_message();
        bytes[8] = 0x05;
        let err = MessageHeader::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("message type"), "{err}");
    }

    #[test]
    fn an_ice_ping_request_matches_the_hand_computed_bytes() {
        let request = Request::new(
            1,
            Identity::new("Ecc"),
            "ice_ping",
            OperationMode::Normal,
            Vec::new(),
        );
        // body = requestId 4 + name 4 + category 1 + facet 1 + operation 9
        //      + mode 1 + context 1 + 空 encapsulation 6 = 27 → size = 14 + 27 = 41 = 0x29
        assert_eq!(
            request.to_bytes().unwrap(),
            vec![
                0x49, 0x63, 0x65, 0x50, 0x01, 0x00, 0x01, 0x00, //
                0x00, // type 0 = Request
                0x00, //
                0x29, 0x00, 0x00, 0x00, // size = 41
                0x01, 0x00, 0x00, 0x00, // requestId = 1
                0x03, 0x45, 0x63, 0x63, // identity.name = "Ecc"
                0x00, // identity.category = ""
                0x00, // facet = 要素 0 の seq
                0x08, 0x69, 0x63, 0x65, 0x5F, 0x70, 0x69, 0x6E, 0x67, // "ice_ping"
                0x00, // mode = Normal
                0x00, // context = 空 dict
                0x06, 0x00, 0x00, 0x00, 0x01, 0x01, // 空 encapsulation(1.1)
            ]
        );
    }

    #[test]
    fn a_request_with_a_long_parameter_round_trips() {
        // configure の DataLinkSet XML は 255 B を超える(TODO/076 §5-4)。
        let xml = format!("<DataLinkSet>{}</DataLinkSet>", "x".repeat(300));
        let mut params = crate::ice::codec::OutputStream::new();
        params.write_string("configId").unwrap();
        params.write_string(&xml).unwrap();

        let mut context = BTreeMap::new();
        context.insert("k".to_string(), "v".to_string());

        let request = Request {
            request_id: 7,
            identity: Identity {
                name: "Ecc".to_string(),
                category: "rc".to_string(),
            },
            facet: "f1".to_string(),
            operation: "configure".to_string(),
            mode: OperationMode::Idempotent,
            context,
            params: params.into_bytes(),
        };

        let bytes = request.to_bytes().unwrap();
        assert_eq!(bytes[8], 0x00);
        assert_eq!(
            u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize,
            bytes.len()
        );
        assert_eq!(Request::parse(&bytes).unwrap(), request);

        let mut params = crate::ice::codec::InputStream::new(&request.params);
        assert_eq!(params.read_string().unwrap(), "configId");
        assert_eq!(params.read_string().unwrap(), xml);
    }

    #[test]
    fn a_request_whose_size_disagrees_with_its_bytes_is_rejected() {
        let request = Request::new(
            1,
            Identity::new("Ecc"),
            "ice_ping",
            OperationMode::Normal,
            Vec::new(),
        );
        let mut bytes = request.to_bytes().unwrap();
        bytes.push(0x00);
        let err = Request::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("size"), "{err}");
    }

    #[test]
    fn a_reply_that_is_not_a_reply_message_is_rejected() {
        let bytes = validate_connection_message();
        assert!(Reply::parse(&bytes).is_err());
    }

    #[test]
    fn an_ok_reply_hands_back_the_out_encapsulation_payload() {
        let reply = Reply {
            request_id: 3,
            body: ReplyBody::Ok(vec![0x02, 0x05]), // State=2, Error=5 相当の 2 B
        };
        // size = 14 + requestId 4 + status 1 + encapsulation(6 + 2) = 27 = 0x1B
        assert_eq!(
            reply.to_bytes().unwrap(),
            vec![
                0x49, 0x63, 0x65, 0x50, 0x01, 0x00, 0x01, 0x00, //
                0x02, // type 2 = Reply
                0x00, //
                0x1B, 0x00, 0x00, 0x00, //
                0x03, 0x00, 0x00, 0x00, // requestId = 3
                0x00, // replyStatus = 0 (Ok)
                0x08, 0x00, 0x00, 0x00, 0x01, 0x01, // encapsulation size = 8
                0x02, 0x05,
            ]
        );
        let parsed = Reply::parse(&reply.to_bytes().unwrap()).unwrap();
        assert_eq!(parsed, reply);
        assert_eq!(parsed.into_result().unwrap(), vec![0x02, 0x05]);
    }

    #[test]
    fn a_void_operation_replies_with_an_empty_encapsulation() {
        let reply = Reply {
            request_id: 11,
            body: ReplyBody::Ok(Vec::new()),
        };
        let bytes = reply.to_bytes().unwrap();
        assert_eq!(bytes.len(), 14 + 4 + 1 + 6);
        assert!(Reply::parse(&bytes)
            .unwrap()
            .into_result()
            .unwrap()
            .is_empty());
    }

    /// compact format(Ice 3.7 の既定): slice は 1 枚、slice size なし。
    #[test]
    fn a_user_exception_in_compact_format_yields_the_type_id_and_the_members() {
        let mut payload = Vec::new();
        payload.push(SLICE_FLAG_IS_LAST_SLICE | SLICE_FLAG_HAS_TYPE_ID_STRING); // 0x21
        payload.push(CMD_EXCEPTION.len() as u8); // 25 = 0x19
        payload.extend_from_slice(CMD_EXCEPTION.as_bytes());
        payload.extend_from_slice(&[0x04, b'b', b'o', b'o', b'm']); // reason = "boom"

        let (type_id, body) = parse_user_exception(&payload).unwrap();
        assert_eq!(type_id, CMD_EXCEPTION);
        assert_eq!(body, vec![0x04, b'b', b'o', b'o', b'm']);
    }

    /// sliced format: slice size 付き(TODO/076 §2.2 の「両対応」)。
    /// size の数え方(自分自身を含むか)には解釈幅があるので **読み飛ばしに使わない** —
    /// メンバはそのまま後ろに続くものとして読む。
    #[test]
    fn a_user_exception_in_sliced_format_yields_the_same_type_id_and_members() {
        let mut payload = Vec::new();
        payload.push(SLICE_FLAG_IS_LAST_SLICE | SLICE_FLAG_HAS_SLICE_SIZE | 0x02); // 0x32
        payload.push(CMD_EXCEPTION.len() as u8);
        payload.extend_from_slice(CMD_EXCEPTION.as_bytes());
        payload.extend_from_slice(&9i32.to_le_bytes()); // slice size
        payload.extend_from_slice(&[0x04, b'b', b'o', b'o', b'm']);

        let (type_id, body) = parse_user_exception(&payload).unwrap();
        assert_eq!(type_id, CMD_EXCEPTION);
        assert_eq!(body, vec![0x04, b'b', b'o', b'o', b'm']);
    }

    #[test]
    fn a_user_exception_reply_round_trips_and_becomes_a_user_exception_error() {
        let reply = Reply {
            request_id: 5,
            body: ReplyBody::UserException {
                type_id: CMD_EXCEPTION.to_string(),
                body: vec![0x04, b'b', b'o', b'o', b'm'],
            },
        };
        let bytes = reply.to_bytes().unwrap();
        assert_eq!(bytes[18], 0x01); // replyStatus = 1
                                     // ヘッダ 14 + requestId 4 + status 1 + encapsulation(6 + flags 1 + typeId 26 + reason 5)
        assert_eq!(bytes.len(), 14 + 4 + 1 + 6 + 1 + 26 + 5);
        assert_eq!(Reply::parse(&bytes).unwrap(), reply);

        let err = Reply::parse(&bytes).unwrap().into_result().unwrap_err();
        match err {
            IceError::UserException { type_id, body } => {
                assert_eq!(type_id, CMD_EXCEPTION);
                assert_eq!(body, vec![0x04, b'b', b'o', b'o', b'm']);
            }
            other => panic!("expected a user exception, got {other:?}"),
        }
    }

    #[test]
    fn a_user_exception_with_a_1_0_encapsulation_is_rejected() {
        let mut bytes = Reply {
            request_id: 5,
            body: ReplyBody::UserException {
                type_id: CMD_EXCEPTION.to_string(),
                body: vec![0x00],
            },
        }
        .to_bytes()
        .unwrap();
        bytes[24] = 0x00; // encapsulation の minor(14 + 4 + 1 + 4 + 1 = 24)
        assert!(Reply::parse(&bytes).is_err());
    }

    #[test]
    fn an_object_not_exist_reply_carries_the_identity_facet_and_operation() {
        let reply = Reply {
            request_id: 9,
            body: ReplyBody::RequestFailed {
                status: ReplyStatus::ObjectNotExist,
                identity: Identity {
                    name: "Ecc".to_string(),
                    category: "rc".to_string(),
                },
                facet: "f1".to_string(),
                operation: "describe".to_string(),
            },
        };
        let bytes = reply.to_bytes().unwrap();
        assert_eq!(bytes[18], 0x02);
        // status の後ろは encapsulation ではなく生の struct:
        // name 4 + category 3 + facet seq(1 + 3) + operation 9 = 20
        assert_eq!(bytes.len(), 14 + 4 + 1 + 20);
        assert_eq!(Reply::parse(&bytes).unwrap(), reply);

        match Reply::parse(&bytes).unwrap().into_result().unwrap_err() {
            IceError::RequestFailed {
                kind,
                identity,
                facet,
                operation,
            } => {
                assert_eq!(kind, ReplyStatus::ObjectNotExist);
                assert_eq!(identity.name, "Ecc");
                assert_eq!(identity.category, "rc");
                assert_eq!(facet, "f1");
                assert_eq!(operation, "describe");
            }
            other => panic!("expected a request-failed error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_exception_reply_carries_the_1_0_encoded_string() {
        let reply = Reply {
            request_id: 2,
            body: ReplyBody::Unknown {
                status: ReplyStatus::UnknownException,
                message: "std::bad_alloc".to_string(),
            },
        };
        let bytes = reply.to_bytes().unwrap();
        assert_eq!(bytes[18], 0x07);
        assert_eq!(bytes.len(), 14 + 4 + 1 + 1 + 14);
        assert_eq!(Reply::parse(&bytes).unwrap(), reply);

        match Reply::parse(&bytes).unwrap().into_result().unwrap_err() {
            IceError::Unknown { kind, message } => {
                assert_eq!(kind, ReplyStatus::UnknownException);
                assert_eq!(message, "std::bad_alloc");
            }
            other => panic!("expected an unknown-exception error, got {other:?}"),
        }
    }

    #[test]
    fn a_reply_status_above_seven_is_rejected() {
        let mut bytes = Reply {
            request_id: 1,
            body: ReplyBody::Ok(Vec::new()),
        }
        .to_bytes()
        .unwrap();
        bytes[18] = 0x08;
        let err = Reply::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("reply status"), "{err}");
    }

    #[test]
    fn a_reply_with_trailing_bytes_after_its_payload_is_rejected() {
        let mut bytes = Reply {
            request_id: 1,
            body: ReplyBody::Unknown {
                status: ReplyStatus::UnknownLocalException,
                message: "x".to_string(),
            },
        }
        .to_bytes()
        .unwrap();
        bytes.push(0xAA);
        let size = bytes.len() as i32;
        bytes[10..14].copy_from_slice(&size.to_le_bytes());
        assert!(Reply::parse(&bytes).is_err());
    }

    #[test]
    fn operation_modes_and_message_types_round_trip_through_their_bytes() {
        assert_eq!(OperationMode::Normal.as_byte(), 0);
        assert_eq!(OperationMode::Idempotent.as_byte(), 2);
        assert_eq!(OperationMode::from_byte(0).unwrap(), OperationMode::Normal);
        assert_eq!(
            OperationMode::from_byte(2).unwrap(),
            OperationMode::Idempotent
        );
        // 1 = Nonmutating(Ice 3.7 で非推奨)。黙って Idempotent に丸めない。
        assert!(OperationMode::from_byte(1).is_err());

        for (byte, expected) in [
            (0u8, MessageType::Request),
            (1, MessageType::BatchRequest),
            (2, MessageType::Reply),
            (3, MessageType::ValidateConnection),
            (4, MessageType::CloseConnection),
        ] {
            assert_eq!(MessageType::from_byte(byte).unwrap(), expected);
            assert_eq!(expected.as_byte(), byte);
        }
    }
}
