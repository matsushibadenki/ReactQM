"""Controlled child process for Rust lifecycle regression tests; never used by the app."""
import json
import os
from pathlib import Path
import sys
import time

request = json.loads(sys.stdin.readline())
output = Path(sys.argv[-1])
(output / 'pid').write_text(str(os.getpid()))
mode = request['document']['name']
if mode == 'exit':
    sys.exit(7)
if mode == 'invalid':
    print('invalid json', flush=True)
    time.sleep(30)
if mode == 'oversized':
    print('x' * 9000, flush=True)
    time.sleep(30)
if mode == 'nonconverged':
    print(json.dumps({'kind': 'error', 'schema_version': 1, 'job_id': request['job_id'], 'code': 'not_converged'}), flush=True)
    sys.exit(1)
if mode == 'foreign':
    print(json.dumps({'kind': 'progress', 'schema_version': 1, 'job_id': 'another-job', 'cycle': 1}), flush=True)
    time.sleep(30)
if mode == 'stall':
    time.sleep(30)
if mode == 'flood':
    while True:
        print(json.dumps({'kind': 'progress', 'schema_version': 1, 'job_id': request['job_id'], 'cycle': 1}), flush=True)
