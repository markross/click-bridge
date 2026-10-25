# Learning Rust with click-bridge, part 1: the probe

This walks through every piece of the Phase 1 code and explains the Rust
ideas it uses. It assumes you can program but have never written Rust. Read
it with the source files open beside it.

Contents:

1. [How a Rust project is laid out](#1-how-a-rust-project-is-laid-out)
2. [`zwift.rs`: constants and types](#2-zwiftrs-constants-and-types)
3. [Functions, expressions and closures](#3-functions-expressions-and-closures)
4. [Enums and `match`](#4-enums-and-match)
5. [`Option` and the `?` operator](#5-option-and-the--operator)
6. [Borrowing: the protobuf decoder](#6-borrowing-the-protobuf-decoder)
7. [Tests](#7-tests)
8. [`probe.rs`: errors with `anyhow`](#8-probers-errors-with-anyhow)
9. [Async code with `tokio`](#9-async-code-with-tokio)
10. [Ownership across tasks: `clone` and `move`](#10-ownership-across-tasks-clone-and-move)
11. [The notification loop](#11-the-notification-loop)
12. [Exercises](#12-exercises)
13. [The first real run: a bug](#13-the-first-real-run-a-bug)
14. [Reading the real log](#14-reading-the-real-log)

---

## 1. How a Rust project is laid out

```
click-bridge/
├── Cargo.toml          the project manifest: name, version, dependencies
├── Cargo.lock          exact dependency versions (generated; commit it)
└── src/
    ├── lib.rs          the library crate's root
    ├── zwift.rs        the `zwift` module: protocol code, no Bluetooth
    └── bin/
        └── probe.rs    a binary (program) called `probe`
```

**Cargo** is Rust's build tool and package manager, like npm or pip rolled
together with a compiler driver. The commands you'll use most:

| Command | What it does |
| --- | --- |
| `cargo build` | Compile everything. Output goes in `target/`. |
| `cargo run --bin probe` | Build, then run the `probe` program. |
| `cargo test` | Build and run all `#[test]` functions. |
| `cargo check` | Type-check without producing a binary. Fastest feedback. |
| `cargo clippy` | Lint: points out non-idiomatic or suspicious code. |
| `cargo fmt` | Reformat code to the standard style. |

A **crate** is Rust's unit of compilation, roughly "a package". This project
holds two crates that share a name:

- a **library crate** rooted at `src/lib.rs`, called `click_bridge` in code
  (Cargo turns the dash in `click-bridge` into an underscore);
- a **binary crate** for each file in `src/bin/`. `probe.rs` becomes a
  program called `probe`.

Splitting it this way means the protocol logic in the library can be unit
tested on any computer, while `probe.rs` does the Bluetooth work that needs
real hardware. Phase 2 will add a second binary that reuses the same library.

Dependencies listed in `Cargo.toml` (`btleplug`, `tokio`, and so on) are also
crates, downloaded from [crates.io](https://crates.io) on first build.

`src/lib.rs` contains one line of real code:

```rust
pub mod zwift;
```

`mod zwift;` tells the compiler "there is a module called `zwift`; its code
is in `zwift.rs`". `pub` makes it public, so the probe can use it.
Everything in Rust is **private by default**: anything you want visible
outside its module needs `pub`.

## 2. `zwift.rs`: constants and types

```rust
use uuid::Uuid;

pub const ZWIFT_COMPANY_ID: u16 = 0x094A;
```

- `use uuid::Uuid;` imports the `Uuid` type from the `uuid` crate so we can
  write `Uuid` instead of `uuid::Uuid`.
- `const` defines a compile-time constant. Constants must state their type.
- Rust's integer types say their size and sign: `u8`, `u16`, `u32`, `u64` are
  unsigned 8 to 64 bit; `i8` to `i64` are signed. There is no implicit
  conversion between them. Mixing a `u16` and a `u32` is a compile error until
  you convert explicitly, which catches a whole class of bugs.

```rust
pub const SERVICE_LEGACY: Uuid = Uuid::from_u128(0x00000001_19ca_4651_86e5_fa29dcdd09d1);
```

A Bluetooth UUID is a 128-bit number, so we write it as one big hex literal.
Underscores inside number literals are ignored, which lets us keep the
familiar `xxxxxxxx-xxxx-...` grouping. `from_u128` is a `const fn`, a function
the compiler can run at build time, which is why it's allowed in a `const`.

```rust
pub const RIDE_ON: &[u8] = b"RideOn";
pub const UNLOCK_ACK: &[u8] = &[0xff, 0x04, 0x00];
```

- `&[u8]` is a **slice**: a borrowed view of a sequence of bytes. It's a
  pointer plus a length; it doesn't own the bytes.
- `b"RideOn"` is a **byte string literal**: the ASCII bytes `52 69 64 65 4f 6e`
  rather than a text string.
- Rust distinguishes `&[u8]` (raw bytes) from `&str` (text that is guaranteed
  to be valid UTF-8). Bluetooth sends bytes, so we use bytes.

```rust
pub const RIDE_BUTTONS: &[(u32, &str)] = &[
    (0x00001, "LEFT"),
    ...
];
```

A slice of **tuples**. `(u32, &str)` is a pair of a number and a string.
This is our lookup table from bitmap bit to button name.

## 3. Functions, expressions and closures

```rust
pub fn is_zwift_controller(
    name: Option<&str>,
    services: &[Uuid],
    manufacturer_ids: &[u16],
) -> bool {
    name.is_some_and(|n| n.to_ascii_lowercase().contains("zwift"))
        || services.iter().any(|s| *s == SERVICE_LEGACY || *s == SERVICE_FC82)
        || manufacturer_ids.contains(&ZWIFT_COMPANY_ID)
}
```

- Parameters are `name: Type`. The return type follows `->`.
- **There's no `return`.** In Rust almost everything is an *expression* that
  produces a value, and the last expression in a function body, written
  *without* a trailing semicolon, is the return value. Adding a `;` would turn
  it into a statement and the function would return nothing (`()`), which
  the compiler would reject because we promised a `bool`.
- `Option<&str>` means "maybe a string". Not every Bluetooth device
  advertises a name. Section 5 covers `Option` properly.
- `|n| n.to_ascii_lowercase().contains("zwift")` is a **closure**: an
  anonymous function. `|n|` is the parameter list. `is_some_and` calls it only
  if there is a name.
- `services.iter().any(...)` walks the slice with an **iterator** and returns
  `true` if the closure is true for any element. Iterators are everywhere in
  Rust and compile down to the same machine code as a hand-written loop.
- `*s`: the iterator hands out *references* to each `Uuid` (`&Uuid`), and `*`
  **dereferences**, giving the `Uuid` itself to compare. More on references in
  section 6.

## 4. Enums and `match`

```rust
#[derive(Debug, PartialEq, Eq)]
pub enum HandshakeReply {
    RideOn,
    Locked,
    Other,
}
```

An `enum` is a type that is exactly one of a fixed set of **variants**. So far
it's like enums in other languages.

`#[derive(...)]` asks the compiler to write some standard code for us:

- `Debug` lets you print the value with `{:?}`, for logging and test failures.
- `PartialEq`, `Eq` let you compare two values with `==`. Tests need this.

These are **traits**. A trait is a named set of behaviour that a type can
implement, like an interface in Java or a protocol in Swift. `derive` is a
shortcut for implementing common ones automatically.

Rust enums become more powerful when variants carry data:

```rust
pub enum Buttons {
    Ride { bitmap: u32, pressed: Vec<String> },
    ClickV1 { plus: bool, minus: bool },
}
```

A `Buttons` value is *either* a Ride-style message with a bitmap and a list
of pressed button names, *or* a Click v1 message with two booleans. You can't
read `plus` from a `Ride` message by mistake, because the compiler won't let
you reach it without checking which variant you have.

- `Vec<T>` is a growable array that **owns** its contents (it's on the heap,
  and freed automatically when the `Vec` goes away).
- `String` is owned, growable text, while `&str` is a borrowed view of text
  someone else owns. The names in the `RIDE_BUTTONS` table are `&'static str`:
  references to text baked into the program binary that lives for the whole
  run (`'static` is a **lifetime**, which is how Rust proves references never
  outlive their data; you'll rarely write one yourself). `pressed` holds
  `String`s instead because unmapped bits get a name built at runtime, such
  as `format!("BIT_0x{bit:x}")`, and text built at runtime has to be owned by
  someone. Section 14 covers why this changed.

You get data out of an enum with `match`:

```rust
match zwift::classify_sync_reply(&n.value) {
    HandshakeReply::RideOn => println!("handshake OK (unlocked)"),
    HandshakeReply::Locked => println!("LOCKED ..."),
    HandshakeReply::Other => {}
}
```

`match` must be **exhaustive**: every variant needs an arm. If we later add a
variant, every `match` that forgot it becomes a compile error, which tells
you exactly where to update the code. `{}` is an empty block: "do nothing".

`classify_sync_reply` shows `if`/`else if` used as an expression; the whole
`if` chain is the function's return value:

```rust
pub fn classify_sync_reply(data: &[u8]) -> HandshakeReply {
    if data.starts_with(RIDE_ON) {
        HandshakeReply::RideOn
    } else if data.first() == Some(&MSG_CHALLENGE) {
        HandshakeReply::Locked
    } else {
        HandshakeReply::Other
    }
}
```

## 5. `Option` and the `?` operator

Rust has no `null`. A value that might be missing has type `Option<T>`,
which is just an enum from the standard library:

```rust
enum Option<T> {
    Some(T),
    None,
}
```

`T` is a **generic** type parameter: `Option<u32>` is "maybe a `u32`",
`Option<&str>` is "maybe a string". You must handle `None` before you can use
the value, so null-pointer crashes can't happen.

`decode_buttons` returns `Option<Buttons>`: a message that isn't a button
message gives `None`.

```rust
pub fn decode_buttons(data: &[u8]) -> Option<Buttons> {
    let (&kind, payload) = data.split_first()?;
    match kind {
        MSG_RIDE_KEYPAD => {
            let bitmap = varint_field(payload, 1)? as u32;
            Some(Buttons::Ride { bitmap, pressed: pressed_ride_buttons(bitmap) })
        }
        ...
        _ => None,
    }
}
```

Line by line:

- `data.split_first()` returns `Option<(&u8, &[u8])>`: the first byte and the
  rest, or `None` if `data` is empty.
- The **`?` operator** means "if this is `None`, return `None` from the
  whole function right now; otherwise unwrap the `Some` and carry on". It
  replaces a lot of `if x == null return null` boilerplate. (It works the same
  way with `Result` for errors, as section 8 shows.)
- `let (&kind, payload) = ...` is **destructuring**: it splits the tuple into
  two variables. The `&` in `&kind` copies the byte out of the reference, so
  `kind` is a plain `u8`.
- `match kind { MSG_RIDE_KEYPAD => ... }` matches against constants. `_` is
  the catch-all arm.
- `as u32` converts between number types. Here the protobuf varint is a `u64`
  and the bitmap is 32 bits.
- `Buttons::Ride { bitmap, pressed: ... }`: writing just `bitmap` is shorthand
  for `bitmap: bitmap`.

## 6. Borrowing: the protobuf decoder

This is Rust's most distinctive idea, so it's worth going slowly.

**Ownership:** every value has exactly one owner. When the owner goes out of
scope, the value is freed. There's no garbage collector and no manual `free`.

**Borrowing:** you can lend a value out with a **reference** instead of
giving it away:

- `&T` is a shared reference: read-only, and you may have many at once.
- `&mut T` is a mutable reference: read-write, and only one may exist at a
  time, with no shared references alongside it.

The compiler (the "borrow checker") enforces these rules, which rules out
data races and use-after-free bugs at compile time.

The decoder reads protobuf, the binary format the Click uses. A protobuf
message is a sequence of fields; each starts with a *key* (field number and
wire type) encoded as a **varint**, a variable-length integer that uses 7
bits per byte, with the top bit meaning "more bytes follow".

```rust
fn read_varint(buf: &mut &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = buf.split_first()?;
        *buf = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}
```

- `buf: &mut &[u8]` reads as "a mutable reference to a byte slice". The
  slice itself is read-only, but the *caller's variable* that holds the slice
  can be changed. `*buf = rest;` moves the caller's view forward past the
  byte we just consumed, like advancing a cursor.
- `let mut value`: variables are **immutable by default**. You need `mut` to
  change one. This makes it obvious which variables change as you read code.
- `0u64` is the number 0 with its type written as a suffix.
- `(0..64).step_by(7)` is a range iterator: 0, 7, 14, ... 63.
- `u64::from(byte & 0x7f)` widens the `u8` to `u64` before shifting, so bits
  don't fall off the top.
- `return Some(value);` is an early return. The final `None` is reached only
  for a malformed varint longer than 10 bytes.

`varint_field` uses it:

```rust
fn varint_field(mut buf: &[u8], field: u64) -> Option<u64> {
    while !buf.is_empty() {
        let key = read_varint(&mut buf)?;
        let (number, wire_type) = (key >> 3, key & 0x7);
        match wire_type {
            0 => { /* varint: read it, return it if it's our field */ }
            1 => buf = buf.get(8..)?,            // 8-byte field: skip
            2 => { /* length-prefixed: read the length, skip that many */ }
            5 => buf = buf.get(4..)?,            // 4-byte field: skip
            _ => return None,
        }
    }
    None
}
```

- `mut buf: &[u8]` is different from `&mut`: here *our local copy* of the
  slice is mutable, so we can move it forward. The caller's slice isn't
  affected.
- `read_varint(&mut buf)` lends our `buf` mutably to `read_varint`, which
  advances it. When `read_varint` returns, the loan ends and we can use `buf`
  again.
- `buf.get(8..)` returns the slice from index 8 onward, or `None` if it's too
  short. Using `get` instead of indexing `buf[8..]` means a truncated message
  gives `None` instead of crashing. One of the tests checks exactly this.

## 7. Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleared_bits_are_pressed() {
        let bitmap: u32 = !0x20 & !0x2000;
        let Some(Buttons::Ride { pressed, .. }) = decode_buttons(&ride_frame(bitmap)) else {
            panic!("not a ride frame");
        };
        assert_eq!(pressed, vec!["B", "SHIFT_UP_R"]);
    }
}
```

- `#[cfg(test)]` compiles the module only for `cargo test`, so tests add
  nothing to the real program.
- `use super::*;` imports everything from the parent module (`zwift`),
  including private functions. Tests in Rust usually live next to the code
  they test, in the same file.
- `#[test]` marks a test function. `cargo test` finds and runs them all.
- `!0x20` is bitwise NOT: all bits set except `0x20`. So this bitmap means
  "B and SHIFT_UP_R are pressed, nothing else".
- `let PATTERN = value else { ... };` is **let-else**: if the value matches
  the pattern, its parts become variables (`pressed`); if not, the `else`
  block runs and must leave the function (here by `panic!`, which fails the
  test). `..` means "ignore the other fields".
- `assert_eq!` fails the test and prints both sides if they differ. The `!`
  marks a **macro**. Macros generate code at compile time; `println!`,
  `vec!` and `format!` are macros too.

## 8. `probe.rs`: errors with `anyhow`

`Option` covers "might be missing". For "might fail, and here's why", Rust
uses `Result<T, E>`:

```rust
enum Result<T, E> {
    Ok(T),
    Err(E),
}
```

Rust has no exceptions. Functions that can fail return a `Result`, and `?`
passes errors up to the caller, just as it passes `None` up for `Option`.

The `anyhow` crate provides a `Result<T>` whose error type can hold any
error plus a chain of context messages. It's the usual choice for
applications. Libraries usually define their own specific error types
instead.

```rust
let central = manager
    .adapters()
    .await?
    .into_iter()
    .next()
    .context("no Bluetooth adapter found")?;
```

Read it as a pipeline:

1. `manager.adapters()` asks for the list of Bluetooth adapters. It's async,
   so `.await` waits for it (section 9). It returns a `Result`, and `?` exits
   if it failed.
2. `.into_iter().next()` takes the first adapter, giving an `Option`.
3. `.context("...")` from anyhow turns `None` into an error with that
   message, so the second `?` can exit with a helpful error.

Other anyhow tools:

- `bail!("message")` returns an error immediately.
- `.context("connect failed")?` on a `Result` adds a message on top of the
  underlying error. Printing with `{err:#}` shows the whole chain, for
  example `connect failed: Peripheral not found`.

`main` itself returns `Result<()>`. `()` (the "unit" type) means "no value".
If `main` returns an `Err`, Rust prints it and exits with a failure code.

### Arguments and structs

```rust
struct Options {
    scan_secs: u64,
    unlock_ack: bool,
    keepalive: bool,
}
```

A `struct` groups named fields, like a class with no methods yet.
`parse_args` fills one in:

```rust
let mut args = std::env::args().skip(1);
while let Some(arg) = args.next() {
    match arg.as_str() {
        "--scan-secs" => {
            opts.scan_secs = args.next().context("--scan-secs needs a value")?.parse()?
        }
        ...
        other => bail!("unknown argument: {other}"),
    }
}
```

- `std::env::args()` is an iterator over the command-line arguments;
  `.skip(1)` drops the program name.
- `while let Some(arg) = args.next()` loops until the iterator returns
  `None`.
- `arg` is a `String` (owned text); `.as_str()` borrows it as `&str` so it
  can be matched against string literals.
- `.parse()?` converts text to a number. The compiler works out that it
  should parse a `u64` because that's the type of `opts.scan_secs`. This is
  **type inference**: Rust figures out most types from how values are used.
- `other => ...` is a catch-all arm that also binds the value to a name.

### Traits must be imported

```rust
use btleplug::api::{Central, Manager as _, Peripheral as _, ...};
```

Methods like `peripheral.connect()` are defined by the `Peripheral` *trait*,
and Rust only lets you call trait methods if the trait is in scope. `as _`
imports a trait's methods without its name, which avoids clashing with the
concrete `Peripheral` type we import from `btleplug::platform`. If you ever
see "no method named X found... items from traits can only be used if the
trait is in scope", a missing `use` is the cause.

## 9. Async code with `tokio`

Bluetooth is slow and event-driven: you ask for something and the answer
arrives later. Rust handles this with `async`/`await`:

- An `async fn` doesn't run when called. It returns a **future**, a
  description of work that hasn't happened yet.
- `.await` runs a future until it finishes, letting other tasks run while it
  waits.
- Futures need an **executor** to drive them. Rust's standard library doesn't
  include one; `tokio` is the most widely used.

```rust
#[tokio::main]
async fn main() -> Result<()> {
```

`#[tokio::main]` is a macro that wraps `main` to start the tokio runtime and
run our async `main` on it.

```rust
tasks.push(tokio::spawn(async move { ... }));
```

`tokio::spawn` starts a **task**: a future that runs concurrently with
everything else, like a lightweight thread. Each pod gets its own task, so
both pods are handled at the same time. `tokio::spawn` returns a handle we
keep, so we can call `task.abort()` on Ctrl-C.

```rust
tokio::signal::ctrl_c().await?;
```

This future finishes when you press Ctrl-C, so `main` sits here while the pod
tasks do the work.

## 10. Ownership across tasks: `clone` and `move`

This is where ownership becomes practical.

```rust
for (label, peripheral) in pods.iter().cloned() {
    let (unlock_ack, keepalive) = (opts.unlock_ack, opts.keepalive);
    tasks.push(tokio::spawn(async move {
        if let Err(err) = run_pod(&label, peripheral, start, unlock_ack, keepalive).await {
            println!("[{label}] error: {err:#}");
        }
    }));
}
```

A spawned task might outlive the function that spawned it, so the compiler
won't let it *borrow* anything from that function. It has to **own**
everything it uses. That's what `async move` means: the block takes ownership
of every variable it mentions.

- `pods.iter().cloned()` hands out a *copy* of each `(label, peripheral)`
  pair, so the task can own its copy while `pods` keeps the originals. We need
  the originals after Ctrl-C to disconnect.
- Cloning a btleplug `Peripheral` is cheap. It's a handle to the shared
  device, not a copy of the device.
- `opts` is copied into two plain variables first because `async move` would
  otherwise try to move all of `opts` into the first task, leaving nothing for
  the second. `bool` values are `Copy`, meaning they're duplicated
  automatically rather than moved.
- `start` is an `Instant`, also `Copy`.

The keepalive does the same thing inside `run_pod`:

```rust
let (peripheral, sync_rx, label) = (peripheral.clone(), sync_rx.clone(), label.to_owned());
tokio::spawn(async move { ... });
```

`label` arrives as `&str`, a borrowed string. `.to_owned()` makes an owned
`String` copy the task can keep. The new variables **shadow** the old ones:
Rust lets you reuse a name with `let`, and the new binding hides the old one
from that point on, but only inside the `if keepalive { ... }` block. After
the block, `peripheral` means the original again. The rest of `run_pod`
happens to use only the notification stream, so moving the originals would
also compile today; cloning keeps `run_pod` free to use the peripheral later
(Phase 2 will) without the keepalive having taken it.

If you forget a `clone`, the compiler says something like "borrowed value
does not live long enough" or "use of moved value". These errors are
frustrating at first, but they're pointing at real bugs: in C or C++, the
same mistake compiles and crashes later.

## 11. The notification loop

```rust
let mut notifications = peripheral.notifications().await?;
peripheral.subscribe(&async_char).await?;
peripheral.subscribe(&sync_tx).await?;
```

A **stream** is the async version of an iterator: values arrive over time.
We get the stream *before* subscribing and sending `RideOn`, so we can't miss
the reply.

```rust
let mut held = BTreeSet::new();
while let Some(n) = notifications.next().await {
    ...
    } else if let Some(buttons) = zwift::decode_buttons(&n.value) {
        let now: BTreeSet<String> = match buttons {
            Buttons::Ride { pressed, .. } => pressed.into_iter().collect(),
            Buttons::ClickV1 { plus, minus } => [(plus, "PLUS"), (minus, "MINUS")]
                .into_iter()
                .filter(|(on, _)| *on)
                .map(|(_, b)| b.to_string())
                .collect(),
        };
        for b in now.difference(&held) {
            println!("{t:8.2} [{label}] PRESS {b}");
        }
        for b in held.difference(&now) {
            println!("{t:8.2} [{label}] RELEASE {b}");
        }
        held = now;
    }
}
```

- `.next().await` waits for the next notification. `StreamExt` (imported
  from `futures`) is the trait that provides `next()` on streams.
- `if let Some(buttons) = ...` is a one-armed `match`: run the block only if
  decoding returned `Some`.
- A `BTreeSet` is a sorted set. The pod reports the *whole* state of its
  buttons in every message, so to print press and release *events* we compare
  the new set (`now`) with the previous one (`held`). `now.difference(&held)`
  is "buttons held now but not before", which means just pressed. Phase 2
  sends a shift key on exactly that edge.
- `.collect()` gathers an iterator into a collection. The `BTreeSet<String>`
  type annotation on `now` tells it which collection to build.
- `held = now;` **moves** `now` into `held`. No copy is made, and `now` can't
  be used after this line, which is fine because the loop body ends there.
- `{t:8.2}` in `println!` formats `t` with width 8 and 2 decimal places.
  `{source:5}` pads to 5 characters. Variables can go straight inside the
  braces.

`find_char` and `hex` at the bottom of the file use the same iterator tools:
`find` returns the first match as an `Option`, `or_else` tries a fallback if
it was `None`, `cloned()` turns `Option<&Characteristic>` into an owned
`Option<Characteristic>`, and `map` + `collect` + `join` build the hex string.

## 12. Exercises

Each takes 5 to 15 minutes. Run `cargo test` and `cargo clippy` after each one.

1. **Read a compiler error.** In `main`, change
   `for (label, peripheral) in pods.iter().cloned() {` to
   `for (label, peripheral) in pods {` and run `cargo build`. You'll get
   `error[E0382]: borrow of moved value: pods`. Read the message top to
   bottom: it shows where the `for` loop took ownership of `pods` and where
   the Ctrl-C code tries to use it afterwards. Put `.iter().cloned()` back.
2. **Add a test.** In `zwift.rs`, add a test that a bitmap with only
   `0x4000` cleared reports `["SHIFT_DN_R"]`.
3. **Add a flag.** Add `--keepalive-secs N` to `parse_args`, store it in
   `Options`, and pass it through to `run_pod` instead of using the
   `KEEPALIVE_INTERVAL` constant. Hint: `Duration::from_secs(n)`.
4. **Use a `match` with data.** Write a function `fn describe(b: &Buttons) ->
   String` that returns `"ride: 2 pressed"` or `"click v1: plus"` and so on,
   and call it from the probe.

Further reading: [The Rust Book](https://doc.rust-lang.org/book/), chapters
3 to 6 and 9 cover nearly everything above, and
[Rust by Example](https://doc.rust-lang.org/rust-by-example/) is good for
quick lookups.

## 13. The first real run: a bug

The first run on a real Click v2 printed:

```
[pod1]     characteristic 00000003-19ca-...  CharPropFlags(WRITE_WITHOUT_RESPONSE)
[pod1]     characteristic 00000100-19ca-...  CharPropFlags(WRITE_WITHOUT_RESPONSE | WRITE | NOTIFY)
[pod1] using async=00000002-... rx=00000100-... tx=00000004-...
[pod1] error: Runtime Error: The attribute could not be found.
```

The handshake characteristic is `00000003`, but the probe chose `00000100`.
The original code was:

```rust
let sync_rx = find_char(&chars, zwift::CHAR_SYNC_RX, CharPropFlags::WRITE)
    .or_else(|| find_char(&chars, zwift::CHAR_SYNC_RX, CharPropFlags::WRITE_WITHOUT_RESPONSE))
```

`find_char` already falls back to "any Zwift characteristic with this
property" when the exact UUID doesn't match. So the first call, asking for
`WRITE`, didn't find `00000003` (it only has `WRITE_WITHOUT_RESPONSE`) and
fell back to `00000100`, which does have `WRITE`. It returned `Some`, so the
`or_else` never ran. Each piece was correct on its own; the bug was in how
they combined.

The fix asks for *either* property in one call:

```rust
find_char(&chars, zwift::CHAR_SYNC_RX, CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE)
```

- `CharPropFlags` is a **bitflags** type: each property is one bit, and `|`
  combines them into a set.
- `a.contains(b)` is true if `a` has *all* of `b`'s bits.
  `a.intersects(b)` is true if `a` has *any* of them. `find_char` now uses
  `intersects`, so the exact UUID matches whenever it has an acceptable
  property, and the fallback only runs if it's genuinely missing.

The error message itself was vague because `?` passed the Bluetooth error up
without saying which step failed. Each setup step now adds `.context(...)`,
so a future failure reads like `sending RideOn: The attribute could not be
found`. Adding context to every `?` that can fail for an outside reason is a
good habit.

## 14. Reading the real log

The second run worked, and its log showed two things worth changing.

**Unknown buttons were invisible.** One press produced `23 08 ff fe ff ff 0f`
but no `PRESS` line. Decoding the varint by hand (section 6): the second
byte `fe` carries bits 7 to 13, and its lowest bit is 0, so bit 7 (`0x80`)
is cleared. Our `RIDE_BUTTONS` table, copied from the Zwift Ride, has no
entry for `0x80`, so the old filter-the-table code silently skipped it.

`pressed_ride_buttons` now walks all 32 bits instead:

```rust
(0..32)
    .map(|i| 1u32 << i)
    .filter(|bit| bitmap & bit == 0)
    .map(|bit| match RIDE_BUTTONS.iter().find(|(mask, _)| *mask == bit) {
        Some((_, name)) => name.to_string(),
        None => format!("BIT_0x{bit:x}"),
    })
    .collect()
```

- `1u32 << i` is a number with only bit `i` set.
- `match` on the `Option` that `find` returns: a known bit uses its table
  name, an unknown one gets a generated name. Both arms must produce the same
  type, so the known name is converted with `.to_string()` into an owned
  `String`, matching what `format!` produces.
- That's why `pressed` changed from `Vec<&'static str>` to `Vec<String>`: a
  `&'static str` can only point at text that exists for the whole program,
  and `BIT_0x80` is created while it runs.

**The log was mostly repeats.** After any press, each pod resends its full
button state about every 100 ms. The probe now remembers the last async
frame in `last_async: Option<Vec<u8>>` and skips printing identical ones.
`last_async.as_ref() == Some(&n.value)` compares without moving anything:
`as_ref()` turns `&Option<Vec<u8>>` into `Option<&Vec<u8>>`, so both sides
are borrowed. `n.value.clone()` then stores a copy, because `n` is still
needed below.
