#!/usr/bin/env python3
"""tap_drive_real.py — TODO/076 Phase 0 golden capture driver (real ECC session).

Drives `ecc_bridge` (pointed at a real getEccServer, 3.7.x) through:
  1. describe with a config_id that has no describe-<id>.xcfg in the
     config-repo -> real ECC replies with a CmdException (over Ice, not just
     a bridge-layer error) -- this is the fixture we need fake_ecc can't give
     us (fake_ecc is an Ice 3.8 process; the 3.7 wire form must come from the
     real server).
  2. status -> a real getStatus Reply.

Needs pyzmq (`pip install pyzmq`); not a repo dependency, just a capture tool.
"""
import json
import sys
import zmq

endpoint = sys.argv[1]
bad_config_id = sys.argv[2] if len(sys.argv) > 2 else "bogus-config-id-999"

ctx = zmq.Context()
sock = ctx.socket(zmq.REQ)
sock.connect(endpoint)
sock.setsockopt(zmq.RCVTIMEO, 20000)
sock.setsockopt(zmq.SNDTIMEO, 20000)

steps = [
    {"action": "describe", "config_id": bad_config_id},
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
