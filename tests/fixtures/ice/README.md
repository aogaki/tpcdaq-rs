# Ice wire fixtures — TODO/076 Phase 0 (golden capture)

Synthetic (no detector, no GDED) captures of the raw Ice protocol bytes that
flow between our C++ `tools/ecc_bridge/ecc_bridge` (the Ice **client**) and
an ECC state-machine servant, taken with a small protocol-framed TCP tap
(`tools/ecc_bridge/tap_ice.py`). They are the ground truth for TODO/076
Phase 1 (`src/ice/` codec + proto + client) — hand-written unit tests should
match these bytes, not the other way around.

Two capture sessions, both **localhost only** (127.0.0.1), never GDED
(192.168.144.101 was never touched):

- **fake_ecc session** — `fake_ecc` (our Ice servant test double, built
  against Homebrew Ice **3.8.2**) walked through one full describe -> ... ->
  reset lap plus a `status` call. Gives all 9 required ops in their normal
  (`ok:true`) shape.
- **real ECC session** — the real `getEccServer` (GET 20190315_patched,
  built from source against Ice **3.7.11**, `reference/_spike/prefix/bin/`)
  driven through a `describe` with a config id that has no matching
  `describe-<id>.xcfg`, to get a genuine `CmdException` **from the real 3.7
  server**, plus its `getStatus` reply. fake_ecc is an Ice 3.8 process, so it
  cannot stand in for this — SPEC/TODO/076 explicitly call for the 3.7 wire
  form here.

`ecc_bridge` forces `ice_encodingVersion(Ice::Encoding_1_1)` on its proxy
(`ecc_bridge.cpp:119`), so every encapsulation in every file below is
encoding 1.1, on both legs, regardless of which Ice version the peer runs —
consistent with CLAUDE.md / SPEC §8.2 ("encoding 1.1 固定").

## Files

All requests/replies are exactly one Ice protocol message (14 B header +
body, framed by the header's own `messageSize` field — no application-level
splitting needed). `<op>_req.bin` / `<op>_rep.bin` is the request the bridge
sent and the reply it got back for that Ice operation, picked out of the
full walk (see "How this was captured" for the full manifest — every
non-`getStatus`, non-`ice_ping` bridge action also triggers an *extra*,
separate `getStatus` call right after it, to fill in the JSON reply's
`state`/`ecc_error` fields; those extra calls are not shape-different from
`getstatus_{req,rep}.bin` so they were not kept as separate files).

| op | files | bytes (req/rep) | requestId | mode | notes |
|---|---|---|---|---|---|
| `ice_ping` (built-in) | `ice_ping_{req,rep}.bin` | 41 / 25 | 1 | **2 (Idempotent)** | no context; automatic, sent once at `ecc_bridge` connect |
| `describe` | `describe_{req,rep}.bin` | 48 / 25 | 2 | 0 (Normal) | configId="pulser" |
| `prepare` | `prepare_{req,rep}.bin` | 47 / 25 | 4 | 0 | configId="pulser" |
| `configure` | `configure_{req,rep}.bin` | **337** / 25 | 6 | 0 | 2-link DataLinkSet XML, 283 B > 255 B — uses the `0xFF + int32` extended string-size form |
| `start` | `start_{req,rep}.bin` | 38 / 25 | 8 | 0 | no args |
| `stop` | `stop_{req,rep}.bin` | 37 / 25 | 10 | 0 | no args |
| `breakup` | `breakup_{req,rep}.bin` | 40 / 25 | 12 | 0 | no args |
| `reset` | `reset_{req,rep}.bin` | 38 / 25 | 14 | 0 | no args |
| `getStatus` | `getstatus_{req,rep}.bin` | 42 / 27 | 16 | 0 | reply payload = `Status{state, error}`, 2 B |

Bonus files (not one of the 9 ops, but directly relevant to TODO/076 §5
risks and asked for in the handoff report):

| file | bytes | what |
|---|---|---|
| `validate_connection.bin` | 14 | fake_ecc's `ValidateConnection`, sent server -> client immediately on TCP accept, before any request |
| `close_connection.bin` | 14 | `ecc_bridge`'s `CloseConnection`, sent client -> server from `Ice::Communicator::destroy()` on shutdown (captured from the fake_ecc session) |
| `real_ecc_validate_connection.bin` | 14 | same as above, from the real (3.7.11) ECC |
| `real_ecc_ice_ping_{req,rep}.bin` | 41 / 25 | `ice_ping` against the real ECC — byte-identical shape to the fake_ecc one (mode 2, empty context), confirms this isn't an Ice-3.8-only artifact |
| `real_ecc_describe_cmdexception_{req,rep}.bin` | 61 / **115** | `describe("bogus-config-id-999")` against the real ECC — genuine `CmdException` reply, see below |
| `real_ecc_getstatus_{req,rep}.bin` | 42 / 27 | real ECC's `getStatus` reply after the failed describe: `Status{Idle, WhenDescribe}` = `01 01` |

## How this was captured

Both sessions used the same recipe: real binaries, all on `127.0.0.1`, no
GDED, ports 4731x/4741x chosen to avoid the default 47200 (bridge REP) /
46002 (real ECC) so as not to collide with other concurrent work.

```
client (ecc_bridge)  <--ZMQ REQ/REP (JSON)-->  tap_drive_{fake,real}.py
ecc_bridge  --Ice, encoding 1.1-->  tap_ice.py serve  --Ice-->  {fake_ecc | real getEccServer}
```

1. Build `ecc_bridge` / `fake_ecc` (Ice 3.8.2, Homebrew keg, `make -j`):

   ```
   cd tools/ecc_bridge
   make -j8 TPCDAQ_ICE_DIR=<repo>/reference/20190315_patched
   ```

2. **fake_ecc session** (all four processes on localhost only):

   ```
   ./fake_ecc --host 127.0.0.1 --port 47311 --no-data-link &
   # --no-data-link: our synthetic DataLinkSet targets (cobo{0,1}-recv...) are
   # not real listeners; this flag makes start() validate-and-skip the TCP
   # connect instead of trying and failing (see fake_ecc.cpp comment (d)).

   python3 tap_ice.py serve --listen-port 47310 --target-host 127.0.0.1 \
       --target-port 47311 --out /tmp/cap_fake &

   ./ecc_bridge --bind tcp://127.0.0.1:47312 \
       --ecc-proxy "Ecc:tcp -h 127.0.0.1 -p 47310" &
   # connecting here is what produces the ValidateConnection + ice_ping pair

   python3 tap_drive_fake.py tcp://127.0.0.1:47312 pulser
   ```

   `tap_drive_fake.py` walks describe -> prepare -> configure(2 links) ->
   start -> stop -> breakup -> reset -> status; every reply came back
   `ok:true` (full transcript in git history of this capture / rerun to
   reproduce). The requestId sequence in `/tmp/cap_fake/manifest.tsv` is
   `1`=ice_ping, `2`/`3`=describe/its-getStatus, `4`/`5`=prepare/its-getStatus,
   `6`/`7`=configure/its-getStatus, `8`/`9`=start/..., `10`/`11`=stop/...,
   `12`/`13`=breakup/..., `14`/`15`=reset/..., `16`=the explicit `status`
   action (this last one has no extra getStatus after it, since it *is* one).
   The `<op>_{req,rep}.bin` files above are the odd-numbered (op) request
   plus its own reply, **not** the observe-only getStatus that
   `ecc_bridge.cpp`'s `handle()` fires after every state-changing action to
   fill in the JSON reply.

3. **real ECC session** (config-repo = a copy of
   `reference/_spike/run/configs/`, which has `describe-zCobo-ZC706.xcfg`
   but deliberately nothing named `describe-bogus-config-id-999.xcfg`):

   ```
   DYLD_LIBRARY_PATH=<repo>/reference/_spike/prefix/lib:/opt/homebrew/opt/ice@3.7/lib:/opt/homebrew/lib \
     <repo>/reference/_spike/prefix/bin/getEccServer \
     --address 127.0.0.1:47411 --config-repo-url ./configs &

   python3 tap_ice.py serve --listen-port 47410 --target-host 127.0.0.1 \
       --target-port 47411 --out /tmp/cap_real &

   ./ecc_bridge --bind tcp://127.0.0.1:47413 \
       --ecc-proxy "Ecc:tcp -h 127.0.0.1 -p 47410" &

   python3 tap_drive_real.py tcp://127.0.0.1:47413 bogus-config-id-999
   ```

   Bridge-level JSON observed:
   ```
   >>> {"action": "describe", "config_id": "bogus-config-id-999"}
   <<< {"ok":false,"state":"Idle","error":"Exception: Configuration 'bogus-config-id-999' does not exist.","ecc_error":"WHEN_DESCRIBE"}
   >>> {"action": "status"}
   <<< {"ok":true,"state":"Idle","error":"","ecc_error":"WHEN_DESCRIBE"}
   ```
   i.e. the real ECC's `BackEnd::onDescribe` throws (a plain `std::exception`,
   "Configuration '...' does not exist.", from `ConfigManager`), which
   `CATCH_SM_EXCEPTIONS(SM::WHEN_DESCRIBE)` (`BackEnd.cpp`) turns into an
   `SM::Exception` (prefixing "Exception: "), which
   `GetEccImpl::describe` (`GetEccImpl.cpp:289-298`) catches and rethrows as
   `mdaq::utl::CmdException(e.errorMessage)` — that is the exception that
   actually crosses the wire (reply status 1, `UserException`), confirmed
   below.

## Confirmed protocol facts (TODO/076 §5 risks this closes)

- **`ice_ping` mode = 2 (Idempotent), context = empty**, identically on both
  the fake (3.8) and real (3.7) ECC. (`Ice::Object`'s built-in `ice_ping` is
  declared `idempotent` in `Ice.ice`; this is not GET/GetEcc-specific.)
- **All 8 `GetEcc.ice::StateMachine` operations we use are mode = 0
  (Normal)** — none of `describe/prepare/configure/start/stop/breakup/
  reset/getStatus` is declared `idempotent` in `GetEcc.ice` (only
  `getDataLinks`, which we don't call, is). Context is empty on every
  request in both sessions.
- **`configure`'s DataLinkSet XML does hit the extended string-size form**:
  in `configure_req.bin`, right after the `configId` string ("pulser", one
  size byte `06`), the DataLinkSet XML (283 B) is prefixed `ff 1b 01 00 00`
  — the `0xFF` marker followed by a little-endian int32 (`0x0000011b` = 283)
  — exactly the ">= 255" form TODO/076 §2.2 describes.
- **ValidateConnection precedes everything**: both sessions show the
  server sending a bare 14-byte `ValidateConnection` (type 3) immediately on
  TCP accept, before the client's first `ice_ping` request. `ecc_bridge`
  (via the Ice runtime) waits for it before issuing requests, exactly as
  §2.2 describes.
- **CloseConnection is client-initiated here**: `ecc_bridge`'s own
  `Ice::Communicator::destroy()` (its `EccClient` destructor) sends a bare
  14-byte `CloseConnection` (type 4) when the process shuts down (captured
  in the fake_ecc session as `close_connection.bin`). We never observed the
  *server* sending one in either session (nothing exercised the "server
  hangs up" path — that would need a real ACM idle timeout or an explicit
  server-side shutdown, out of scope for Phase 0).
- **`CmdException`'s wire shape is a single **compact-format** slice**
  (`real_ecc_describe_cmdexception_rep.bin`, reply status 1 =
  `UserException`): flags byte `0x20`. Per `Ice/Protocol.h`
  (`FLAG_HAS_TYPE_ID_STRING=1<<0`, `FLAG_HAS_TYPE_ID_INDEX=1<<1`,
  `FLAG_HAS_SLICE_SIZE=1<<4`, `FLAG_IS_LAST_SLICE=1<<5`), `0x20` sets
  **only** `FLAG_IS_LAST_SLICE` — no slice-size field (compact, not
  sliced-with-size) — which matches "Ice 3.7 の既定 = compact format".
  **Open question for Phase 1**: even though neither type-id flag bit is
  set, a length-prefixed string `"::mdaq::utl::CmdException"` (25 B) is
  present immediately after the flags byte, followed by the `reason` string
  (62 B, `"Exception: Configuration 'bogus-config-id-999' does not
  exist."`) — `1(flags) + 1+25(typeId) + 1+62(reason) = 90 B`, exactly the
  encapsulation payload size. We parse this fixture correctly in
  `tap_ice.py annotate` (it knows `CmdException`'s concrete shape: one
  slice, one string member), but **we have not confirmed from Ice's actual
  marshaling source why the type-id string appears despite the flag bits
  reading "absent"** — Phase 1 must verify this against real Ice source
  before hard-coding a slice-flags interpretation in `src/ice/codec.rs`;
  treat "flags byte + always try a type-id string first" as a hypothesis
  backed by exactly one real sample, not a confirmed rule. (TODO/076 §5
  risk 1.)

## Annotated hexdumps

Regenerate any of these with:
```
python3 tools/ecc_bridge/tap_ice.py annotate tests/fixtures/ice/<file>.bin
```

### Handshake

```
=== validate_connection.bin (14 B) — fake_ecc, server -> client ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=3 (ValidateConnection) compression=0 messageSize=14
  0000  49 63 65 50 01 00 01 00 03 00 0e 00 00 00        IceP..........

=== close_connection.bin (14 B) — ecc_bridge, client -> server (fake_ecc session, on shutdown) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=4 (CloseConnection) compression=0 messageSize=14
  0000  49 63 65 50 01 00 01 00 04 00 0e 00 00 00        IceP..........
```

### ice_ping (fake_ecc, Ice 3.8.2)

```
=== ice_ping_req.bin (41 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=41
  0000  49 63 65 50 01 00 01 00 00 00 29 00 00 00        IceP......)...
requestId=1 identity={name='Ecc', category=''} facet=[] operation='ice_ping' mode=2 (Idempotent) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== ice_ping_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=1 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)
```

### describe (fake_ecc)

```
=== describe_req.bin (48 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=48
  0000  49 63 65 50 01 00 01 00 00 00 30 00 00 00        IceP......0...
requestId=2 identity={name='Ecc', category=''} facet=[] operation='describe' mode=0 (Normal) context={}
params encapsulation: size=13 (payload 7 B) encoding=1.1
  0000  06 70 75 6c 73 65 72                             .pulser

=== describe_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=2 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)
```

### prepare (fake_ecc)

```
=== prepare_req.bin (47 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=47
  0000  49 63 65 50 01 00 01 00 00 00 2f 00 00 00        IceP....../...
requestId=4 identity={name='Ecc', category=''} facet=[] operation='prepare' mode=0 (Normal) context={}
params encapsulation: size=13 (payload 7 B) encoding=1.1
  0000  06 70 75 6c 73 65 72                             .pulser

=== prepare_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=4 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)
```

### configure (fake_ecc) — the >255 B case

```
=== configure_req.bin (337 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=337
  0000  49 63 65 50 01 00 01 00 00 00 51 01 00 00        IceP......Q...
requestId=6 identity={name='Ecc', category=''} facet=[] operation='configure' mode=0 (Normal) context={}
params encapsulation: size=301 (payload 295 B) encoding=1.1
  0000  06 70 75 6c 73 65 72 ff 1b 01 00 00 3c 44 61 74  .pulser.....<Dat
  0010  61 4c 69 6e 6b 53 65 74 3e 3c 44 61 74 61 4c 69  aLinkSet><DataLi
  0020  6e 6b 3e 3c 44 61 74 61 53 65 6e 64 65 72 20 69  nk><DataSender i
  0030  64 3d 22 43 6f 42 6f 5b 30 5d 22 2f 3e 3c 44 61  d="CoBo[0]"/><Da
  0040  74 61 52 6f 75 74 65 72 20 69 70 41 64 64 72 65  taRouter ipAddre
  0050  73 73 3d 22 63 6f 62 6f 30 2d 72 65 63 76 2e 74  ss="cobo0-recv.t
  0060  70 63 64 61 71 2d 66 69 78 74 75 72 65 2e 6c 6f  pcdaq-fixture.lo
  0070  63 61 6c 22 20 70 6f 72 74 3d 22 34 37 34 30 31  cal" port="47401
  0080  22 20 74 79 70 65 3d 22 54 43 50 22 2f 3e 3c 2f  " type="TCP"/></
  0090  44 61 74 61 4c 69 6e 6b 3e 3c 44 61 74 61 4c 69  DataLink><DataLi
  00a0  6e 6b 3e 3c 44 61 74 61 53 65 6e 64 65 72 20 69  nk><DataSender i
  00b0  64 3d 22 43 6f 42 6f 5b 31 5d 22 2f 3e 3c 44 61  d="CoBo[1]"/><Da
  00c0  74 61 52 6f 75 74 65 72 20 69 70 41 64 64 72 65  taRouter ipAddre
  00d0  73 73 3d 22 63 6f 62 6f 31 2d 72 65 63 76 2e 74  ss="cobo1-recv.t
  00e0  70 63 64 61 71 2d 66 69 78 74 75 72 65 2e 6c 6f  pcdaq-fixture.lo
  00f0  63 61 6c 22 20 70 6f 72 74 3d 22 34 37 34 30 32  cal" port="47402
  0100  22 20 74 79 70 65 3d 22 54 43 50 22 2f 3e 3c 2f  " type="TCP"/></
  0110  44 61 74 61 4c 69 6e 6b 3e 3c 2f 44 61 74 61 4c  DataLink></DataL
  0120  69 6e 6b 53 65 74 3e                             inkSet>

=== configure_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=6 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)
```

Byte-for-byte: `06` = size of "pulser" (6), then the 6 ASCII bytes; then
`ff 1b 01 00 00` = extended-size marker + `0x0000011b` (283) LE; then the
283-byte DataLinkSet XML verbatim (`<DataLinkSet><DataLink><DataSender
id="CoBo[0]"/>...`).

### start / stop / breakup / reset (fake_ecc) — all void, no args

```
=== start_req.bin (38 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=38
  0000  49 63 65 50 01 00 01 00 00 00 26 00 00 00        IceP......&...
requestId=8 identity={name='Ecc', category=''} facet=[] operation='start' mode=0 (Normal) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== start_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=8 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== stop_req.bin (37 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=37
  0000  49 63 65 50 01 00 01 00 00 00 25 00 00 00        IceP......%...
requestId=10 identity={name='Ecc', category=''} facet=[] operation='stop' mode=0 (Normal) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== stop_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=10 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== breakup_req.bin (40 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=40
  0000  49 63 65 50 01 00 01 00 00 00 28 00 00 00        IceP......(...
requestId=12 identity={name='Ecc', category=''} facet=[] operation='breakup' mode=0 (Normal) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== breakup_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=12 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== reset_req.bin (38 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=38
  0000  49 63 65 50 01 00 01 00 00 00 26 00 00 00        IceP......&...
requestId=14 identity={name='Ecc', category=''} facet=[] operation='reset' mode=0 (Normal) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== reset_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=14 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)
```

### getStatus (fake_ecc) — reply carries the `Status{State, Error}` struct

```
=== getstatus_req.bin (42 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=42
  0000  49 63 65 50 01 00 01 00 00 00 2a 00 00 00        IceP......*...
requestId=16 identity={name='Ecc', category=''} facet=[] operation='getStatus' mode=0 (Normal) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== getstatus_rep.bin (27 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=27
  0000  49 63 65 50 01 00 01 00 02 00 1b 00 00 00        IceP..........
requestId=16 replyStatus=0 (OK)
results encapsulation: size=8 (payload 2 B) encoding=1.1
  0000  02 00                                            ..
```

`02 00` = `State::Described` (ordinal 2, 0-based: Off,Idle,Described,...),
`Error::NoErr` (ordinal 0) — matches the bridge JSON reply at that point
(`{"state":"Described","ecc_error":"NO_ERR"}`, after the full describe ->
... -> reset walk). Each `enum` is encoded as a single byte since both
`State` (7 values) and `Error` (10 values) are under 255 enumerators.

### Real ECC (Ice 3.7.11) — `ice_ping`, `CmdException`, `getStatus`

```
=== real_ecc_ice_ping_req.bin (41 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=41
  0000  49 63 65 50 01 00 01 00 00 00 29 00 00 00        IceP......)...
requestId=1 identity={name='Ecc', category=''} facet=[] operation='ice_ping' mode=2 (Idempotent) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== real_ecc_ice_ping_rep.bin (25 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=25
  0000  49 63 65 50 01 00 01 00 02 00 19 00 00 00        IceP..........
requestId=1 replyStatus=0 (OK)
results encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== real_ecc_describe_cmdexception_req.bin (61 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=61
  0000  49 63 65 50 01 00 01 00 00 00 3d 00 00 00        IceP......=...
requestId=2 identity={name='Ecc', category=''} facet=[] operation='describe' mode=0 (Normal) context={}
params encapsulation: size=26 (payload 20 B) encoding=1.1
  0000  13 62 6f 67 75 73 2d 63 6f 6e 66 69 67 2d 69 64  .bogus-config-id
  0010  2d 39 39 39                                      -999

=== real_ecc_describe_cmdexception_rep.bin (115 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=115
  0000  49 63 65 50 01 00 01 00 02 00 73 00 00 00        IceP......s...
requestId=2 replyStatus=1 (UserException)
exception encapsulation: size=96 (payload 90 B) encoding=1.1
  0000  20 19 3a 3a 6d 64 61 71 3a 3a 75 74 6c 3a 3a 43   .::mdaq::utl::C
  0010  6d 64 45 78 63 65 70 74 69 6f 6e 3e 45 78 63 65  mdException>Exce
  0020  70 74 69 6f 6e 3a 20 43 6f 6e 66 69 67 75 72 61  ption: Configura
  0030  74 69 6f 6e 20 27 62 6f 67 75 73 2d 63 6f 6e 66  tion 'bogus-conf
  0040  69 67 2d 69 64 2d 39 39 39 27 20 64 6f 65 73 20  ig-id-999' does 
  0050  6e 6f 74 20 65 78 69 73 74 2e                    not exist.
  slice flags=0x20 (stringTypeId bit=False, indexTypeId bit=False, hasSliceSize bit=False, isLastSlice bit=True)
  typeId='::mdaq::utl::CmdException'
  reason="Exception: Configuration 'bogus-config-id-999' does not exist."

=== real_ecc_getstatus_req.bin (42 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=0 (Request) compression=0 messageSize=42
  0000  49 63 65 50 01 00 01 00 00 00 2a 00 00 00        IceP......*...
requestId=4 identity={name='Ecc', category=''} facet=[] operation='getStatus' mode=0 (Normal) context={}
params encapsulation: size=6 (payload 0 B) encoding=1.1
  (empty)

=== real_ecc_getstatus_rep.bin (27 B) ===
header (14 B):
  magic=b'IceP' protocol=1.0 encoding=1.0 messageType=2 (Reply) compression=0 messageSize=27
  0000  49 63 65 50 01 00 01 00 02 00 1b 00 00 00        IceP..........
requestId=4 replyStatus=0 (OK)
results encapsulation: size=8 (payload 2 B) encoding=1.1
  0000  01 01                                            ..
```

`01 01` = `State::Idle` (1), `Error::WhenDescribe` (1) — matches the bridge
JSON (`{"state":"Idle","ecc_error":"WHEN_DESCRIBE"}`), confirming the real
ECC's `handleStateMachineException` path (state does **not** move, the
error flag is set to the phase's code) exactly as `ecc_core.hpp`'s
set/clear-rule comment (based on `BackEnd.cpp`) describes.

Real ECC's `requestId` sequence in this session: `1`=ice_ping,
`2`/`3`=describe(CmdException)/its-getStatus(observe),
`4`=the explicit `status` action.

## Deviations from the order sheet

None in scope. Two choices made where the order sheet left the "how" open
(TCP tap language/location, and the exact ports) are recorded above and were
picked within what §3 Phase 0 explicitly allowed ("Python asyncio 数十行、
または Rust example" / "ポートは他の作業と衝突しないもの... 例
47310/47311"). Everything else (which 9 ops, the >255 B configure
requirement, capturing both fake_ecc and a real 3.7 ECC, confirming
`ice_ping`'s mode) follows the order sheet directly.

One thing flagged rather than decided: the `CmdException` slice-flags
byte/type-id-string relationship above is reported as an open question, not
resolved — see "Confirmed protocol facts" and the Phase 0 handoff report.
