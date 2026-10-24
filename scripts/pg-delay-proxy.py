# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""A TCP proxy that delays every byte it forwards, in both directions, by a fixed time (stdlib only).

    uv run scripts/pg-delay-proxy.py <listen-port> <target-port> <one-way-delay-ms>

It stands in for the round trip a Worker pays to reach Postgres through Hyperdrive, the VPC service and the tunnel
(research/0004: `select 1` p50 9 to 10 ms from IAD), so that `scripts/contract.sh native-pg bench` on loopback can show
what holding a row lock across several round trips costs. Order is preserved; the delay is per chunk and does not queue
up behind itself (a chunk waits until its own arrival time plus the delay). Used only when `THRUST_PG_LATENCY_MS` is set
for `contract.sh`; it listens on 127.0.0.1 and forwards to 127.0.0.1.
"""
import asyncio
import sys
import time


async def pump(reader, writer, delay):
    queue = asyncio.Queue()

    async def read_side():
        try:
            while data := await reader.read(65536):
                queue.put_nowait((time.monotonic() + delay, data))
        finally:
            queue.put_nowait((0.0, None))

    task = asyncio.create_task(read_side())
    try:
        while True:
            due, data = await queue.get()
            if data is None:
                break
            wait = due - time.monotonic()
            if wait > 0:
                await asyncio.sleep(wait)
            writer.write(data)
            await writer.drain()
    except (ConnectionError, asyncio.CancelledError):
        pass
    finally:
        task.cancel()
        try:
            writer.close()
        except Exception:
            pass


async def main(listen_port, target_port, delay_ms):
    delay = delay_ms / 1000.0

    async def handle(client_r, client_w):
        try:
            server_r, server_w = await asyncio.open_connection("127.0.0.1", target_port)
        except OSError:
            client_w.close()
            return
        await asyncio.gather(pump(client_r, server_w, delay), pump(server_r, client_w, delay))

    server = await asyncio.start_server(handle, "127.0.0.1", listen_port)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    listen, target, ms = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3])
    asyncio.run(main(listen, target, ms))
