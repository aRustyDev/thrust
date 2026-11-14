"""Listens on 127.0.0.1:<port> and closes every connection it gets, until it is killed. Standard library only.

The stand-in for the local end of `cloudflared access tcp`, for test-migrate-ci.sh (issue #49): all the script asks of it is that the
port accepts a connection. It does not speak Postgres.
"""

import socket
import sys

s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1])))
s.listen(16)
while True:
    conn, _ = s.accept()
    conn.close()
