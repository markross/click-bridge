# click-bridge

Use a Zwift Click v2 controller to shift gears in MyWhoosh on macOS.

Status: **Phase 1**. The `probe` program connects to the Click pods and logs
what they send, so we can confirm the protocol before sending keys to MyWhoosh.

New to Rust? Start with [docs/learn/01-probe.md](docs/learn/01-probe.md).

## Running the probe on a Mac

1. Install Rust (one time):
   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
   Then open a new Terminal window so `cargo` is on your PATH.
2. Quit Zwift and Zwift Companion. A pod talks to one app at a time.
3. Wake both pods by pressing a button on each.
4. From this folder, run:
   ```sh
   cargo run --bin probe
   ```
   The first build downloads dependencies and takes a minute or two.
5. macOS will ask whether Terminal may use Bluetooth. Allow it. If you missed
   the prompt, enable Terminal under System Settings → Privacy & Security →
   Bluetooth, then run the probe again.
6. Press every button on each pod, one at a time, then wait a couple of
   minutes without pressing anything. Press Ctrl-C to stop.

Options:

| Flag | Effect |
| --- | --- |
| `--scan-secs N` | Scan for N seconds before connecting (default 10). |
| `--unlock-ack` | Also send `ff 04 00` after the handshake (said to keep an unlocked left pod unlocked). |
| `--no-keepalive` | Don't resend `RideOn` every 3 seconds. Use it to check whether pods drop after about a minute. |

To save the output for sharing: `cargo run --bin probe 2>&1 | tee probe.log`

## Tests

```sh
cargo test
```
