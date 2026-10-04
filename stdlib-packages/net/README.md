# net

Socket address types and validation. Live TCP/UDP/DNS is a documented
Phase 2 addition.

## API

- `NetAddr { host, port }` — `net_addr`, `net_local(port)`
- `net_port_ok(port)`, `net_is_valid(addr)`, `net_to_string(addr)`
- `net_save(addr, path) -> i32` — persist `host:port` via `io`

## Test

```sh
klang run net_test.klang
```
