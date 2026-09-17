//! Phase 0 ゴールデン(`tests/fixtures/ice/`)とのバイト一致。TODO/076 §3 Phase 1。
//!
//! ゴールデンは実バイナリ同士の実測(fake_ecc = Ice 3.8.2 / 実 getEccServer =
//! Ice 3.7.11、どちらも localhost、採取手順は同ディレクトリの README.md)。
//! **こちらが正** — `src/ice/` の手書き単体テストがこれに合わせる、逆ではない。
//!
//! ここで踏む事実(README「Confirmed protocol facts」):
//! - `ice_ping` は mode=2(Idempotent)・context 空。fake(3.8)と実 ECC(3.7)で同一バイト。
//! - `StateMachine` の 8 op は mode=0(Normal)・context 空。
//! - `configure` の DataLinkSet XML は 283 B で string size の `0xFF`+int32 形式を踏む。
//! - ValidateConnection は最初のリクエストより前に必ず来る。
//! - 実 ECC の CmdException = replyStatus 1、compact スライス 1 枚、**flags=0x20**、
//!   直後に type id 文字列、続いて `reason`。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use tpcdaq::ice::codec::{InputStream, OutputStream};
use tpcdaq::ice::proto::{
    close_connection_message, parse_user_exception, validate_connection_message, Identity,
    MessageHeader, MessageType, OperationMode, Reply, ReplyBody, Request, HEADER_SIZE,
};
use tpcdaq::ice::IceError;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ice");

/// `configure_req.bin` が運ぶ 2 リンクの DataLinkSet(283 B — 255 B 超)。
const DATA_LINK_SET_XML: &str = concat!(
    "<DataLinkSet>",
    "<DataLink><DataSender id=\"CoBo[0]\"/><DataRouter ",
    "ipAddress=\"cobo0-recv.tpcdaq-fixture.local\" port=\"47401\" type=\"TCP\"/></DataLink>",
    "<DataLink><DataSender id=\"CoBo[1]\"/><DataRouter ",
    "ipAddress=\"cobo1-recv.tpcdaq-fixture.local\" port=\"47402\" type=\"TCP\"/></DataLink>",
    "</DataLinkSet>"
);

/// 実 ECC が返した `CmdException::reason`(62 B)。
const CMD_EXCEPTION_REASON: &str = "Exception: Configuration 'bogus-config-id-999' does not exist.";
const CMD_EXCEPTION_TYPE_ID: &str = "::mdaq::utl::CmdException";

fn golden(name: &str) -> Vec<u8> {
    let path = PathBuf::from(DIR).join(name);
    std::fs::read(&path).unwrap_or_else(|err| panic!("missing golden {}: {err}", path.display()))
}

fn ecc() -> Identity {
    Identity::new("Ecc")
}

/// string 引数だけを詰めた in パラメータ(= encapsulation の中身)。
fn string_params(values: &[&str]) -> Vec<u8> {
    let mut out = OutputStream::new();
    for value in values {
        out.write_string(value).unwrap();
    }
    out.into_bytes()
}

/// (ファイル, requestId, operation, mode, 引数) — README の表そのまま。
fn request_table() -> Vec<(&'static str, i32, &'static str, OperationMode, Vec<u8>)> {
    vec![
        (
            "ice_ping_req.bin",
            1,
            "ice_ping",
            OperationMode::Idempotent,
            Vec::new(),
        ),
        (
            "describe_req.bin",
            2,
            "describe",
            OperationMode::Normal,
            string_params(&["pulser"]),
        ),
        (
            "prepare_req.bin",
            4,
            "prepare",
            OperationMode::Normal,
            string_params(&["pulser"]),
        ),
        (
            "configure_req.bin",
            6,
            "configure",
            OperationMode::Normal,
            string_params(&["pulser", DATA_LINK_SET_XML]),
        ),
        (
            "start_req.bin",
            8,
            "start",
            OperationMode::Normal,
            Vec::new(),
        ),
        (
            "stop_req.bin",
            10,
            "stop",
            OperationMode::Normal,
            Vec::new(),
        ),
        (
            "breakup_req.bin",
            12,
            "breakup",
            OperationMode::Normal,
            Vec::new(),
        ),
        (
            "reset_req.bin",
            14,
            "reset",
            OperationMode::Normal,
            Vec::new(),
        ),
        (
            "getstatus_req.bin",
            16,
            "getStatus",
            OperationMode::Normal,
            Vec::new(),
        ),
        // 実 ECC(3.7.11)側。ice_ping は fake(3.8)とバイト一致のはず。
        (
            "real_ecc_ice_ping_req.bin",
            1,
            "ice_ping",
            OperationMode::Idempotent,
            Vec::new(),
        ),
        (
            "real_ecc_describe_cmdexception_req.bin",
            2,
            "describe",
            OperationMode::Normal,
            string_params(&["bogus-config-id-999"]),
        ),
        (
            "real_ecc_getstatus_req.bin",
            4,
            "getStatus",
            OperationMode::Normal,
            Vec::new(),
        ),
    ]
}

#[test]
fn every_captured_request_is_reproduced_byte_for_byte() {
    for (file, request_id, operation, mode, params) in request_table() {
        let expected = golden(file);
        let built = Request::new(request_id, ecc(), operation, mode, params.clone())
            .to_bytes()
            .unwrap();
        assert_eq!(built, expected, "{file} does not match what we build");
        assert_eq!(built.len(), expected.len(), "{file} length");
    }
}

#[test]
fn every_captured_request_parses_back_to_the_same_fields() {
    for (file, request_id, operation, mode, params) in request_table() {
        let parsed = Request::parse(&golden(file)).unwrap();
        assert_eq!(parsed.request_id, request_id, "{file} requestId");
        assert_eq!(parsed.operation, operation, "{file} operation");
        assert_eq!(parsed.mode, mode, "{file} mode");
        assert_eq!(parsed.identity, ecc(), "{file} identity");
        assert!(parsed.facet.is_empty(), "{file} facet");
        assert!(parsed.context.is_empty(), "{file} context");
        assert_eq!(parsed.params, params, "{file} params");
    }
}

#[test]
fn the_configure_request_uses_the_extended_string_size_for_the_283_byte_xml() {
    assert_eq!(DATA_LINK_SET_XML.len(), 283);

    let bytes = golden("configure_req.bin");
    // "pulser"(size 1 B)の直後が 0xFF + int32(283 = 0x0000_011B)。
    let marker = [0xFF, 0x1B, 0x01, 0x00, 0x00];
    let at = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .expect("extended string-size marker not found in configure_req.bin");
    assert_eq!(&bytes[at..at + 5], &marker);

    let parsed = Request::parse(&bytes).unwrap();
    let mut params = InputStream::new(&parsed.params);
    assert_eq!(params.read_string().unwrap(), "pulser");
    assert_eq!(params.read_string().unwrap(), DATA_LINK_SET_XML);
    assert!(params.is_empty());
}

/// (ファイル, requestId, out encapsulation の中身)。
fn ok_reply_table() -> Vec<(&'static str, i32, Vec<u8>)> {
    vec![
        ("ice_ping_rep.bin", 1, Vec::new()),
        ("describe_rep.bin", 2, Vec::new()),
        ("prepare_rep.bin", 4, Vec::new()),
        ("configure_rep.bin", 6, Vec::new()),
        ("start_rep.bin", 8, Vec::new()),
        ("stop_rep.bin", 10, Vec::new()),
        ("breakup_rep.bin", 12, Vec::new()),
        ("reset_rep.bin", 14, Vec::new()),
        // Status{Described, NoErr} = enum 2 個
        ("getstatus_rep.bin", 16, vec![0x02, 0x00]),
        ("real_ecc_ice_ping_rep.bin", 1, Vec::new()),
        // Status{Idle, WhenDescribe}
        ("real_ecc_getstatus_rep.bin", 4, vec![0x01, 0x01]),
    ]
}

#[test]
fn every_captured_ok_reply_parses_to_its_status_and_payload() {
    for (file, request_id, payload) in ok_reply_table() {
        let bytes = golden(file);
        let reply = Reply::parse(&bytes).unwrap();
        assert_eq!(reply.request_id, request_id, "{file} requestId");
        assert_eq!(reply.body, ReplyBody::Ok(payload.clone()), "{file} body");
        assert_eq!(
            reply.to_bytes().unwrap(),
            bytes,
            "{file} does not round-trip"
        );
        assert_eq!(
            Reply::parse(&bytes).unwrap().into_result().unwrap(),
            payload,
            "{file} out parameters"
        );
    }
}

#[test]
fn the_status_replies_decode_as_two_enumerators() {
    // fake_ecc: State=2(Described)、Error=0(NoErr)。実 ECC: State=1(Idle)、
    // Error=1(WhenDescribe)。enum の値表の突き合わせは Phase 2。
    for (file, expected) in [
        ("getstatus_rep.bin", [2u32, 0]),
        ("real_ecc_getstatus_rep.bin", [1, 1]),
    ] {
        let out = Reply::parse(&golden(file)).unwrap().into_result().unwrap();
        assert_eq!(out.len(), 2, "{file} payload length");
        let mut input = InputStream::new(&out);
        let state = input.read_enum().unwrap();
        let error = input.read_enum().unwrap();
        assert_eq!([state, error], expected, "{file}");
        assert!(input.is_empty(), "{file} trailing bytes");
    }
}

#[test]
fn the_real_ecc_cmdexception_reply_yields_the_type_id_and_the_reason() {
    let bytes = golden("real_ecc_describe_cmdexception_rep.bin");

    // encapsulation の中身の先頭 = slice flags。ヘッダ 14 + requestId 4 +
    // replyStatus 1 + encapsulation ヘッダ 6 = 25。
    assert_eq!(bytes[18], 1, "replyStatus must be 1 (UserException)");
    assert_eq!(bytes[25], 0x20, "slice flags must be IS_LAST_SLICE only");

    let reply = Reply::parse(&bytes).unwrap();
    assert_eq!(reply.request_id, 2);
    let ReplyBody::UserException { type_id, body } = &reply.body else {
        panic!("expected a user exception, got {:?}", reply.body);
    };
    assert_eq!(type_id, CMD_EXCEPTION_TYPE_ID);
    assert_eq!(
        InputStream::new(body).read_string().unwrap(),
        CMD_EXCEPTION_REASON
    );

    // 我々が組み立てても同じバイト列になる(compact flags = 0x20)。
    assert_eq!(reply.to_bytes().unwrap(), bytes);

    match Reply::parse(&bytes).unwrap().into_result().unwrap_err() {
        IceError::UserException { type_id, body } => {
            assert_eq!(type_id, CMD_EXCEPTION_TYPE_ID);
            assert_eq!(
                InputStream::new(&body).read_string().unwrap(),
                CMD_EXCEPTION_REASON
            );
        }
        other => panic!("expected a user exception, got {other:?}"),
    }
}

#[test]
fn parse_user_exception_reads_the_captured_encapsulation_payload() {
    let bytes = golden("real_ecc_describe_cmdexception_rep.bin");
    // 25 B 目から末尾が encapsulation の中身(README: payload 90 B)。
    let payload = &bytes[25..];
    assert_eq!(payload.len(), 90);

    let (type_id, body) = parse_user_exception(payload).unwrap();
    assert_eq!(type_id, CMD_EXCEPTION_TYPE_ID);
    assert_eq!(type_id.len(), 25);
    assert_eq!(
        InputStream::new(&body).read_string().unwrap(),
        CMD_EXCEPTION_REASON
    );
    assert_eq!(CMD_EXCEPTION_REASON.len(), 62);
    // 1(flags) + 1+25(typeId) + 1+62(reason) = 90
    assert_eq!(1 + 1 + type_id.len() + body.len(), payload.len());
}

#[test]
fn the_handshake_messages_are_bare_headers_we_can_build() {
    for (file, expected_type) in [
        ("validate_connection.bin", MessageType::ValidateConnection),
        (
            "real_ecc_validate_connection.bin",
            MessageType::ValidateConnection,
        ),
        ("close_connection.bin", MessageType::CloseConnection),
    ] {
        let bytes = golden(file);
        assert_eq!(bytes.len(), HEADER_SIZE, "{file} length");
        let header = MessageHeader::parse(&bytes).unwrap();
        assert_eq!(header.message_type, expected_type, "{file} type");
        assert_eq!(header.size, HEADER_SIZE, "{file} size");
        assert_eq!(header.compression, 0, "{file} compression");

        let built = match expected_type {
            MessageType::ValidateConnection => validate_connection_message(),
            _ => close_connection_message(),
        };
        assert_eq!(built, bytes, "{file} does not match what we build");
    }

    // 実 ECC と fake_ecc の ValidateConnection は同一バイト。
    assert_eq!(
        golden("validate_connection.bin"),
        golden("real_ecc_validate_connection.bin")
    );
}

#[test]
fn the_real_ecc_and_fake_ecc_ice_ping_are_byte_identical() {
    assert_eq!(
        golden("ice_ping_req.bin"),
        golden("real_ecc_ice_ping_req.bin")
    );
    assert_eq!(
        golden("ice_ping_rep.bin"),
        golden("real_ecc_ice_ping_rep.bin")
    );
    // mode=2 は Ice 3.8 固有の癖ではない(README の確定事実)。
    assert_eq!(
        Request::parse(&golden("real_ecc_ice_ping_req.bin"))
            .unwrap()
            .mode,
        OperationMode::Idempotent
    );
}
