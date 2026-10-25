//! Phase 1 probe: connect to every Zwift controller in range, do the RideOn
//! handshake, keep the pods awake, and log everything they send.
//!
//! Usage: cargo run --bin probe -- [--scan-secs N] [--unlock-ack] [--no-keepalive]

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use click_bridge::zwift::{self, Buttons, HandshakeReply};
use futures::StreamExt;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(3);

struct Options {
    scan_secs: u64,
    unlock_ack: bool,
    keepalive: bool,
}

fn parse_args() -> Result<Options> {
    let mut opts = Options {
        scan_secs: 10,
        unlock_ack: false,
        keepalive: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scan-secs" => {
                opts.scan_secs = args.next().context("--scan-secs needs a value")?.parse()?
            }
            "--unlock-ack" => opts.unlock_ack = true,
            "--no-keepalive" => opts.keepalive = false,
            "-h" | "--help" => {
                println!("probe [--scan-secs N] [--unlock-ack] [--no-keepalive]");
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    Ok(opts)
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts = parse_args()?;
    let start = Instant::now();

    let manager = Manager::new().await?;
    let central = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .context("no Bluetooth adapter found")?;

    println!(
        "Scanning for Zwift controllers for {}s. Press a button on each pod to wake it.",
        opts.scan_secs
    );
    central.start_scan(ScanFilter::default()).await?;
    tokio::time::sleep(Duration::from_secs(opts.scan_secs)).await;
    central.stop_scan().await?;

    let mut pods = Vec::new();
    for peripheral in central.peripherals().await? {
        let Some(props) = peripheral.properties().await? else {
            continue;
        };
        let manufacturer_ids: Vec<u16> = props.manufacturer_data.keys().copied().collect();
        if !zwift::is_zwift_controller(
            props.local_name.as_deref(),
            &props.services,
            &manufacturer_ids,
        ) {
            continue;
        }
        let label = format!("pod{}", pods.len() + 1);
        println!(
            "[{label}] found {:?} id={} rssi={:?}",
            props.local_name.as_deref().unwrap_or("?"),
            peripheral.id(),
            props.rssi
        );
        for (company, data) in &props.manufacturer_data {
            println!("[{label}]   manufacturer 0x{company:04x}: {}", hex(data));
        }
        for service in &props.services {
            println!("[{label}]   advertised service {service}");
        }
        pods.push((label, peripheral));
    }
    if pods.is_empty() {
        bail!(
            "no Zwift controllers found; wake the pods and check Bluetooth permission for your terminal"
        );
    }

    let mut tasks = Vec::new();
    for (label, peripheral) in pods.iter().cloned() {
        let (unlock_ack, keepalive) = (opts.unlock_ack, opts.keepalive);
        tasks.push(tokio::spawn(async move {
            if let Err(err) = run_pod(&label, peripheral, start, unlock_ack, keepalive).await {
                println!("[{label}] error: {err:#}");
            }
        }));
    }

    println!("Logging. Press Ctrl-C to stop.");
    tokio::signal::ctrl_c().await?;
    for task in &tasks {
        task.abort();
    }
    for (_, peripheral) in &pods {
        let _ = peripheral.disconnect().await;
    }
    Ok(())
}

async fn run_pod(
    label: &str,
    peripheral: Peripheral,
    start: Instant,
    unlock_ack: bool,
    keepalive: bool,
) -> Result<()> {
    peripheral.connect().await.context("connect failed")?;
    peripheral.discover_services().await?;
    println!("[{label}] connected");

    for service in peripheral.services() {
        println!("[{label}]   service {}", service.uuid);
        for c in &service.characteristics {
            println!("[{label}]     characteristic {} {:?}", c.uuid, c.properties);
        }
    }

    let chars = peripheral.characteristics();
    let async_char = find_char(&chars, zwift::CHAR_ASYNC, CharPropFlags::NOTIFY)
        .context("no async (notify) characteristic")?;
    let sync_rx = find_char(
        &chars,
        zwift::CHAR_SYNC_RX,
        CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE,
    )
    .context("no sync RX (write) characteristic")?;
    let sync_tx = find_char(&chars, zwift::CHAR_SYNC_TX, CharPropFlags::INDICATE)
        .context("no sync TX (indicate) characteristic")?;
    println!(
        "[{label}] using async={} rx={} tx={}",
        async_char.uuid, sync_rx.uuid, sync_tx.uuid
    );

    let mut notifications = peripheral.notifications().await?;
    peripheral
        .subscribe(&async_char)
        .await
        .context("subscribing to async characteristic")?;
    peripheral
        .subscribe(&sync_tx)
        .await
        .context("subscribing to sync TX characteristic")?;

    let write_type = if sync_rx.properties.contains(CharPropFlags::WRITE) {
        WriteType::WithResponse
    } else {
        WriteType::WithoutResponse
    };
    peripheral
        .write(&sync_rx, zwift::RIDE_ON, write_type)
        .await
        .context("sending RideOn")?;
    println!("[{label}] sent RideOn");
    if unlock_ack {
        peripheral
            .write(&sync_rx, zwift::UNLOCK_ACK, write_type)
            .await?;
        println!("[{label}] sent unlock ack {}", hex(zwift::UNLOCK_ACK));
    }

    if keepalive {
        let (peripheral, sync_rx, label) = (peripheral.clone(), sync_rx.clone(), label.to_owned());
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(KEEPALIVE_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = peripheral.write(&sync_rx, zwift::RIDE_ON, write_type).await {
                    println!("[{label}] keepalive failed: {err}");
                    return;
                }
            }
        });
    }

    let mut held = BTreeSet::new();
    while let Some(n) = notifications.next().await {
        let t = start.elapsed().as_secs_f32();
        let source = if n.uuid == sync_tx.uuid {
            "tx"
        } else if n.uuid == async_char.uuid {
            "async"
        } else {
            "other"
        };
        println!("{t:8.2} [{label}] {source:5} {}", hex(&n.value));

        if n.uuid == sync_tx.uuid {
            match zwift::classify_sync_reply(&n.value) {
                HandshakeReply::RideOn => println!("{t:8.2} [{label}] handshake OK (unlocked)"),
                HandshakeReply::Locked => {
                    println!("{t:8.2} [{label}] LOCKED: crypto challenge, needs a Zwift unlock")
                }
                HandshakeReply::Other => {}
            }
        } else if let Some(buttons) = zwift::decode_buttons(&n.value) {
            let now: BTreeSet<&str> = match buttons {
                Buttons::Ride { pressed, .. } => pressed.into_iter().collect(),
                Buttons::ClickV1 { plus, minus } => [(plus, "PLUS"), (minus, "MINUS")]
                    .into_iter()
                    .filter(|(on, _)| *on)
                    .map(|(_, b)| b)
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
    println!("[{label}] notification stream ended (disconnected?)");
    Ok(())
}

/// Picks the characteristic with the expected UUID, or else the first one
/// in a Zwift service with the required property. `props` lists acceptable
/// properties: a characteristic needs at least one of them.
///
/// The exact UUID always wins, so the fallback can't pick a different
/// characteristic just because it has a property the real one lacks.
fn find_char(
    chars: &BTreeSet<Characteristic>,
    uuid: uuid::Uuid,
    props: CharPropFlags,
) -> Option<Characteristic> {
    chars
        .iter()
        .find(|c| c.uuid == uuid && c.properties.intersects(props))
        .or_else(|| {
            chars.iter().find(|c| {
                (c.service_uuid == zwift::SERVICE_LEGACY || c.service_uuid == zwift::SERVICE_FC82)
                    && c.properties.intersects(props)
            })
        })
        .cloned()
}

fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
