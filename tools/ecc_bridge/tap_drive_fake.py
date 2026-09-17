#!/usr/bin/env python3
"""tap_drive_fake.py — TODO/076 Phase 0 golden capture driver (fake_ecc session).

Sends a fixed sequence of JSON REQ/REP actions to `ecc_bridge` (ZMQ REQ) that
walks the ECC state machine through one full lap: describe -> prepare ->
configure(2 links, XML > 255 B) -> start -> stop -> breakup -> reset ->
status. Pair with `tap_ice.py serve` sitting between `ecc_bridge` and
`fake_ecc` to capture the resulting Ice wire bytes — see README.md in
tests/fixtures/ice/ for the full recipe and the captured fixtures this
produced.

Needs pyzmq (`pip install pyzmq`); not a repo dependency, just a capture tool.
"""
import json
import sys
import zmq

endpoint = sys.argv[1]  # e.g. tcp://127.0.0.1:47312
config_id = sys.argv[2] if len(sys.argv) > 2 else "pulser"

ctx = zmq.Context()
sock = ctx.socket(zmq.REQ)
sock.connect(endpoint)
sock.setsockopt(zmq.RCVTIMEO, 15000)
sock.setsockopt(zmq.SNDTIMEO, 15000)

# 2-link DataLinkSet whose XML exceeds 255 bytes on purpose, so the
# `configure` request exercises Ice's 0xFF + int32 extended string-size form
# (TODO/076 Phase 0 requirement). router_ip values are synthetic hostnames
# chosen only to push the length over 255 B, not real receiver addresses.
links = [
    {"sender": "CoBo[0]", "router_ip": "cobo0-recv.tpcdaq-fixture.local", "router_port": 47401, "type": "TCP"},
    {"sender": "CoBo[1]", "router_ip": "cobo1-recv.tpcdaq-fixture.local", "router_port": 47402, "type": "TCP"},
]

steps = [
    {"action": "describe", "config_id": config_id},
    {"action": "prepare", "config_id": config_id},
    {"action": "configure", "config_id": config_id, "links": links},
    {"action": "start"},
    {"action": "stop"},
    {"action": "breakup"},
    {"action": "reset"},
    {"action": "status"},
]

for step in steps:
    req = json.dumps(step)
    print(f">>> {req}")
    sock.send_string(req)
    rep = sock.recv_string()
    print(f"<<< {rep}")

sock.close()
ctx.term()
