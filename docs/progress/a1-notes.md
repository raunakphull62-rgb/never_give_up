# A1 notes — proposed text for shared docs (do NOT merge yet)

For the L1-docs stage (human will ask). Copy-paste candidates:

## reference.md §5 (new rows after `http_post`)

| `tcp_listen(host, port)` / `listener_port(l)` / `tcp_accept(l)` / `tcp_accept_timeout(l, ms)` | 2 / 1 / 1 / 2 | `port 0` picks a free port; `accept_timeout` is `Conn` or null (timeout); use `is_null(x)` to test null |
| `tcp_connect(host, port, timeout_ms)` | 3 | `timeout_ms == 0` blocks; refused is `E-NET-REFUSED`, timeout `E-NET-TIMEOUT` |
| `conn_read(c, max)` / `conn_read_line(c)` / `conn_read_hex(c, max)` | 2 / 1 / 2 | `read` is lossy UTF-8 (`""` at peer close); `read_line` strips `\n`/`\r\n` (null at close); `read_hex` is exact lowercase hex |
| `conn_write(c, s)` / `conn_write_hex(c, hex)` | 2 / 2 | return bytes written; bad hex is `E-NET-IO` |
| `conn_close(h)` / `conn_set_timeout(c, ms)` / `conn_peer(c)` | 1 / 2 / 1 | `close` accepts any handle, idempotent; `0` clears timeouts; `peer` is `"ip:port"` |
| `udp_bind(host, port)` / `udp_send_to(s, host, port, data)` / `udp_recv_from(s, max, timeout)` | 2 / 4 / 3 | `recv` is `{data, host, port}`; timeout is `E-NET-TIMEOUT` |
| `dns_lookup(host)` | 1 | sorted deduped IP strings; failures are `E-NET-DNS` |
| `is_null(x)` | 1 | `1` for null (timeout/close), else `0`; `str(null)` is `"null"` |

Rules: strings are UTF-8 (no bytes type; use `conn_*_hex` + the `hex`
package for exact bytes); handles are opaque (`<listener>`/`<conn>`/
`<udpsock>`, identity equality, closed at program exit, `E-NET-CLOSED`
on reuse after local close); all denied with `E-SANDBOX-NETWORK` under
`klang run --sandbox`; tests use `127.0.0.1` + port `0` only.

## limitations.md (new item)

- **Sockets (A1)**: blocking only (no threads yet); `conn_read` is one
  syscall (loop for exact sizes); `conn_read`/`udp_recv_from` are lossy
  UTF-8 (use hex for binary); null has no literal (only
  `tcp_accept_timeout`/`conn_read_line` produce it; test with `is_null`);
  `conn_close` is the only close name (polymorphic over all handles).

## errors.md / ai/errors.md

- `E-NET-REFUSED`, `E-NET-TIMEOUT`, `E-NET-ADDR-INVALID`,
  `E-NET-ADDR-IN-USE`, `E-NET-CLOSED`, `E-NET-IO`, `E-NET-DNS` (see
  `docs/progress/a1.md` for when each fires); `E-SANDBOX-NETWORK` for all
  of them under `--sandbox`.
