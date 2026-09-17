#!/usr/bin/env python3
"""tap_ice.py — Ice-message-framed TCP tap (TODO/076 Phase 0, SPEC §8.2 golden capture).

Sits between our C++ `ecc_bridge` (client) and an ECC (fake_ecc or the real
getEccServer), forwards every byte unchanged, and additionally writes each
*complete* Ice protocol message to its own file so later phases (076 Phase 1
codec/proto/client) get exact, reproducible wire fixtures instead of a hand
transcription.

Framing is the one piece of the Ice protocol this tool actually has to
understand: every message starts with the fixed 14-byte header (magic "IceP",
protocolMajor/Minor, encodingMajor/Minor, messageType, compressionStatus,
then messageSize as a little-endian int32 that counts the header itself). We
read exactly that many bytes, forward them, and log them — see SPEC v1.29 /
TODO/076 §2.2 for the header field layout this mirrors.

Usage:
    # one shot: listen once, forward one connection to the target, capture
    # every message into --out, exit when either side closes.
    tap_ice.py serve --listen-port 47310 --target-host 127.0.0.1 \\
        --target-port 47311 --out tests/fixtures/ice/_capture

    # print an annotated hexdump of one captured message (or a whole dir)
    tap_ice.py annotate tests/fixtures/ice/_capture/003_c2s_Request.bin

This is a Phase-0 spike tool, not a general Ice proxy: single connection,
no compression, no batch requests, no retry. Only ever points at
127.0.0.1 — never at GDED or any other remote host.
"""

import argparse
import asyncio
import os
import struct
import sys

MAGIC = b"IceP"
MSG_TYPE_NAMES = {
    0: "Request",
    1: "BatchRequest",
    2: "Reply",
    3: "ValidateConnection",
    4: "CloseConnection",
}
REPLY_STATUS_NAMES = {
    0: "OK",
    1: "UserException",
    2: "ObjectNotExist",
    3: "FacetNotExist",
    4: "OperationNotExist",
    5: "UnknownLocalException",
    6: "UnknownUserException",
    7: "UnknownException",
}


# --- framing ----------------------------------------------------------------


async def read_one_message(reader: asyncio.StreamReader) -> bytes:
    """Read exactly one Ice protocol message (header + body). Empty bytes at EOF."""
    header = await reader.readexactly(14)
    if header[0:4] != MAGIC:
        raise ValueError(f"not an Ice message (magic={header[0:4]!r})")
    size = struct.unpack_from("<i", header, 10)[0]
    if size < 14:
        raise ValueError(f"messageSize {size} smaller than header")
    body = await reader.readexactly(size - 14) if size > 14 else b""
    return header + body


# --- capture / forward --------------------------------------------------------

_seq = [0]


def _next_seq() -> int:
    _seq[0] += 1
    return _seq[0]


async def pump(reader, writer, direction, out_dir, manifest):
    """Read messages from `reader`, forward each to `writer`, and save a copy."""
    while True:
        try:
            msg = await read_one_message(reader)
        except asyncio.IncompleteReadError:
            break
        if not msg:
            break
        seq = _next_seq()
        mtype = msg[8]
        tname = MSG_TYPE_NAMES.get(mtype, f"Unknown{mtype}")
        req_id = struct.unpack_from("<i", msg, 14)[0] if mtype in (0, 2) and len(msg) >= 18 else None
        fname = f"{seq:03d}_{direction}_{tname}.bin"
        with open(os.path.join(out_dir, fname), "wb") as f:
            f.write(msg)
        line = f"{seq}\t{direction}\t{tname}\t{len(msg)}\t{req_id if req_id is not None else '-'}\n"
        manifest.write(line)
        manifest.flush()
        print(f"tap_ice: {fname} ({len(msg)} B, requestId={req_id})", file=sys.stderr)
        writer.write(msg)
        await writer.drain()
    writer.close()


async def serve(listen_port: int, target_host: str, target_port: int, out_dir: str):
    os.makedirs(out_dir, exist_ok=True)
    manifest_path = os.path.join(out_dir, "manifest.tsv")
    accepted = asyncio.Event()

    async def on_client(client_reader, client_writer):
        peer = client_writer.get_extra_info("peername")
        print(f"tap_ice: client connected from {peer}", file=sys.stderr)
        target_reader, target_writer = await asyncio.open_connection(target_host, target_port)
        with open(manifest_path, "w") as manifest:
            manifest.write("seq\tdirection\tmessageType\tbytes\trequestId\n")
            await asyncio.gather(
                pump(client_reader, target_writer, "c2s", out_dir, manifest),
                pump(target_reader, client_writer, "s2c", out_dir, manifest),
            )
        print("tap_ice: connection closed, shutting down", file=sys.stderr)
        accepted.set()
        server.close()

    server = await asyncio.start_server(on_client, "127.0.0.1", listen_port)
    sock = server.sockets[0]
    print(f"tap_ice: listening on 127.0.0.1:{sock.getsockname()[1]} -> "
          f"{target_host}:{target_port}, capturing into {out_dir}", file=sys.stderr)
    async with server:
        await server.serve_forever()


# --- annotate -----------------------------------------------------------------


def read_varsize(buf: bytes, off: int):
    """Ice 1.1 variable-length size: 1 byte, or 0xFF + int32 LE for >=255."""
    n = buf[off]
    if n == 0xFF:
        n = struct.unpack_from("<i", buf, off + 1)[0]
        return n, off + 5
    return n, off + 1


def read_string(buf: bytes, off: int):
    n, off = read_varsize(buf, off)
    s = buf[off:off + n].decode("utf-8", errors="replace")
    return s, off + n


def hexdump(buf: bytes, indent: str = "  ") -> str:
    lines = []
    for i in range(0, len(buf), 16):
        chunk = buf[i:i + 16]
        hexpart = " ".join(f"{b:02x}" for b in chunk)
        asciipart = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        lines.append(f"{indent}{i:04x}  {hexpart:<47}  {asciipart}")
    return "\n".join(lines)


def annotate_header(buf: bytes) -> str:
    magic, pMaj, pMin, eMaj, eMin, mtype, comp = struct.unpack_from("<4sBBBBBB", buf, 0)
    size = struct.unpack_from("<i", buf, 10)[0]
    tname = MSG_TYPE_NAMES.get(mtype, f"Unknown{mtype}")
    return (
        f"header (14 B):\n"
        f"  magic={magic!r} protocol={pMaj}.{pMin} encoding={eMaj}.{eMin} "
        f"messageType={mtype} ({tname}) compression={comp} messageSize={size}\n"
        f"{hexdump(buf[0:14])}"
    )


def annotate_encaps(buf: bytes, off: int, label: str) -> str:
    size = struct.unpack_from("<i", buf, off)[0]
    major, minor = buf[off + 4], buf[off + 5]
    payload = buf[off + 6:off + size]
    out = [f"{label} encapsulation: size={size} (payload {len(payload)} B) "
           f"encoding={major}.{minor}", hexdump(payload) if payload else "  (empty)"]
    return "\n".join(out)


def annotate_request(buf: bytes) -> str:
    out = [annotate_header(buf)]
    off = 14
    req_id = struct.unpack_from("<i", buf, off)[0]
    off += 4
    name, off = read_string(buf, off)
    category, off = read_string(buf, off)
    facet_count, off = read_varsize(buf, off)
    facets = []
    for _ in range(facet_count):
        f, off = read_string(buf, off)
        facets.append(f)
    operation, off = read_string(buf, off)
    mode = buf[off]
    off += 1
    ctx_count, off = read_varsize(buf, off)
    ctx = {}
    for _ in range(ctx_count):
        k, off = read_string(buf, off)
        v, off = read_string(buf, off)
        ctx[k] = v
    out.append(
        f"requestId={req_id} identity={{name={name!r}, category={category!r}}} "
        f"facet={facets} operation={operation!r} mode={mode} "
        f"({'Idempotent' if mode == 2 else 'Normal' if mode == 0 else mode}) "
        f"context={ctx}"
    )
    out.append(annotate_encaps(buf, off, "params"))
    return "\n".join(out)


def annotate_cmd_exception(payload: bytes) -> str:
    """CmdException has no base slices: flags byte, type-id string, `reason` string.

    NOTE (2026-09-17 real-ECC capture, TODO/076 Phase 0): the observed flags byte
    is 0x20 (FLAG_IS_LAST_SLICE only, per Ice/Protocol.h's FLAG_HAS_TYPE_ID_STRING
    = 1<<0 / FLAG_HAS_TYPE_ID_INDEX = 1<<1 / FLAG_HAS_SLICE_SIZE = 1<<4 /
    FLAG_IS_LAST_SLICE = 1<<5) -- i.e. *neither* type-id bit is set -- yet a
    length-prefixed string ("::mdaq::utl::CmdException") is unconditionally
    present right after the flags byte, before `reason`. We parse it
    unconditionally here because we know CmdException's concrete shape (single
    slice, one string member), not because we have confirmed what an unset
    type-id-bits pair is supposed to mean in general. Phase 1 must verify this
    against Ice's real marshaling source (not just Protocol.h's flag constants)
    before hard-coding an interpretation -- see TODO/076 §5 risk 1 and the
    Phase 0 handoff report.
    """
    off = 0
    flags = payload[off]
    off += 1
    type_id, off = read_string(payload, off)
    reason, off = read_string(payload, off)
    return (f"  slice flags=0x{flags:02x} (stringTypeId bit={bool(flags & 0x01)}, "
            f"indexTypeId bit={bool(flags & 0x02)}, hasSliceSize bit={bool(flags & 0x10)}, "
            f"isLastSlice bit={bool(flags & 0x20)}) -- see NOTE above: a type-id "
            f"string follows regardless of the type-id bits observed here\n"
            f"  typeId={type_id!r}\n"
            f"  reason={reason!r}")


def annotate_reply(buf: bytes) -> str:
    out = [annotate_header(buf)]
    off = 14
    req_id = struct.unpack_from("<i", buf, off)[0]
    off += 4
    status = buf[off]
    off += 1
    sname = REPLY_STATUS_NAMES.get(status, f"Unknown{status}")
    out.append(f"requestId={req_id} replyStatus={status} ({sname})")
    if status in (0, 1):
        out.append(annotate_encaps(buf, off, "results" if status == 0 else "exception"))
        if status == 1:
            size = struct.unpack_from("<i", buf, off)[0]
            payload = buf[off + 6:off + size]
            out.append(annotate_cmd_exception(payload))
    elif status in (2, 3, 4):
        name, off = read_string(buf, off)
        category, off = read_string(buf, off)
        facet, off = read_string(buf, off)
        operation, off = read_string(buf, off)
        out.append(f"identity={{name={name!r}, category={category!r}}} facet={facet!r} "
                    f"operation={operation!r}")
    else:
        msg, off = read_string(buf, off)
        out.append(f"message={msg!r}")
    return "\n".join(out)


def annotate_file(path: str) -> str:
    with open(path, "rb") as f:
        buf = f.read()
    mtype = buf[8]
    header = f"=== {path} ({len(buf)} B) ===\n"
    if mtype == 0:
        return header + annotate_request(buf)
    if mtype == 2:
        return header + annotate_reply(buf)
    return header + annotate_header(buf)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("serve", help="tap one connection: client -> tap -> target")
    s.add_argument("--listen-port", type=int, required=True)
    s.add_argument("--target-host", default="127.0.0.1")
    s.add_argument("--target-port", type=int, required=True)
    s.add_argument("--out", required=True)

    a = sub.add_parser("annotate", help="print an annotated hexdump of captured message(s)")
    a.add_argument("paths", nargs="+")

    args = ap.parse_args()
    if args.cmd == "serve":
        if args.target_host != "127.0.0.1" and args.target_host != "localhost":
            print("tap_ice: refusing non-localhost target (GDED must never be touched "
                  "from this tool)", file=sys.stderr)
            sys.exit(2)
        asyncio.run(serve(args.listen_port, args.target_host, args.target_port, args.out))
    elif args.cmd == "annotate":
        for p in args.paths:
            print(annotate_file(p))
            print()


if __name__ == "__main__":
    main()
