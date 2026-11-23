//! Devices, capturing and leases (doc 05, ADR 0008). Hardware commands need the device's lease:
//! the first actor to use a device holds it; others get a clear error; the person can always take
//! it over with `lease.take`.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::device::{Capabilities, CaptureConfig, DeviceInfo, StopSignal, StoreSink};
use logica_core::import::StoreTarget;

use crate::contract::{Actor, CommandError, ErrorCode, Event};
use crate::engine::{Ctx, Done, Engine, OpenDevice, Running, err};
use crate::state::CaptureSummary;

use super::{Nothing, inv, out};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEntry {
    pub info: DeviceInfo,
    pub capabilities: Capabilities,
    pub connected: bool,
    /// Who holds the lease, if anyone.
    pub holder: Option<Actor>,
    /// Whether a capture is running on it.
    pub capturing: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeviceListOut {
    pub devices: Vec<DeviceEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceRef {
    /// A device id from device.list, such as "synthetic/mix".
    pub device: String,
}

fn catalogue(e: &mut Engine) {
    if !e.catalogue.is_empty() {
        return;
    }
    for (k, d) in e.drivers.iter().enumerate() {
        for info in d.enumerate() {
            if let Ok(dev) = d.open(&info.id) {
                e.catalogue.push((k, info, dev.capabilities().clone()));
            }
        }
    }
}

pub(crate) fn entries(e: &mut Engine) -> Vec<DeviceEntry> {
    catalogue(e);
    e.catalogue
        .iter()
        .map(|(_, info, caps)| DeviceEntry {
            info: info.clone(),
            capabilities: caps.clone(),
            connected: e.devices.contains_key(&info.id),
            holder: e.leases.get(&info.id).cloned(),
            capturing: e.running.iter().any(|r| r.device == info.id),
        })
        .collect()
}

fn entry(e: &mut Engine, id: &str) -> Result<DeviceEntry, CommandError> {
    entries(e).into_iter().find(|d| d.info.id == id).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("There is no device \"{id}\". device.list shows the devices there are."),
        )
    })
}

fn who(a: &Actor) -> String {
    match a {
        Actor::User => "the person".into(),
        Actor::Agent(n) => format!("agent {n}"),
        Actor::Cli => "the command line".into(),
    }
}

/// Gives `actor` the lease if it is free; refuses if someone else holds it.
fn use_lease(e: &mut Engine, device: &str, actor: &Actor) -> Result<(), CommandError> {
    match e.leases.get(device).cloned() {
        Some(h) if h == *actor => Ok(()),
        Some(h) => {
            let model = entry(e, device).map(|d| d.info.model).unwrap_or_else(|_| device.into());
            let what = if matches!(h, Actor::User) {
                format!("{model} is in use by the person; wait or ask them to hand it over.")
            } else if *actor == Actor::User {
                format!("{model} is in use by {}; take it over with lease.take.", who(&h))
            } else {
                format!("{model} is in use by {}; ask the person to take it over.", who(&h))
            };
            Err(err(ErrorCode::Busy, what))
        }
        None => {
            e.leases.insert(device.into(), actor.clone());
            e.emit(Event::LeaseChanged { device: device.into(), holder: Some(actor.clone()) });
            Ok(())
        }
    }
}

pub(crate) fn list(e: &mut Engine, _: &Ctx, _: Nothing) -> Result<Done, CommandError> {
    Ok(Done::query(out(&DeviceListOut { devices: entries(e) })))
}

pub(crate) fn connect(e: &mut Engine, ctx: &Ctx, i: DeviceRef) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    use_lease(e, &i.device, &ctx.actor)?;
    if !e.devices.contains_key(&i.device) {
        let k = e.catalogue.iter().find(|c| c.1.id == i.device).map(|c| c.0).unwrap_or(0);
        let dev = e.drivers[k].open(&i.device).map_err(|x| err(ErrorCode::Failed, x.0))?;
        e.devices.insert(i.device.clone(), OpenDevice { device: Some(dev), configured: false });
    }
    let d = entry(e, &i.device)?;
    let model = d.info.model.clone();
    Ok(Done::never(out(&d), format!("connected {model}")))
}

fn need_free(e: &Engine, device: &str) -> Result<(), CommandError> {
    if e.running.iter().any(|r| r.device == device) {
        return Err(err(
            ErrorCode::Busy,
            "A capture is running on this device; stop it first with capture.stop.",
        ));
    }
    Ok(())
}

pub(crate) fn disconnect(e: &mut Engine, ctx: &Ctx, i: DeviceRef) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    use_lease(e, &i.device, &ctx.actor)?;
    need_free(e, &i.device)?;
    if let Some(mut d) = e.devices.remove(&i.device)
        && let Some(dev) = d.device.as_mut()
    {
        dev.close();
    }
    e.leases.remove(&i.device);
    e.emit(Event::LeaseChanged { device: i.device.clone(), holder: None });
    Ok(Done::never(out(&Nothing {}), format!("disconnected {}", i.device)))
}

fn connected_device<'a>(e: &'a mut Engine, device: &str) -> Result<&'a mut OpenDevice, CommandError> {
    e.devices
        .get_mut(device)
        .ok_or_else(|| err(ErrorCode::InvalidInput, format!("Connect {device} first with device.connect.")))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceConfigureIn {
    pub device: String,
    pub config: CaptureConfig,
}

pub(crate) fn configure(e: &mut Engine, ctx: &Ctx, i: DeviceConfigureIn) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    use_lease(e, &i.device, &ctx.actor)?;
    need_free(e, &i.device)?;
    let d = connected_device(e, &i.device)?;
    let dev = d.device.as_mut().ok_or_else(|| err(ErrorCode::Busy, "The device is busy."))?;
    dev.configure(&i.config).map_err(|x| err(ErrorCode::InvalidInput, x.0))?;
    d.configured = true;
    let rate = logica_protocols::format::hz(i.config.sample_rate_hz);
    Ok(Done::never(
        out(&Nothing {}),
        format!("configured {} for {} samples at {rate}", i.device, i.config.samples),
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureStartIn {
    pub device: String,
    /// The tab title; "Capture N" when absent.
    pub title: Option<String>,
    /// Wait until the capture ends (default). Without waiting, the capture runs on and
    /// `capture.done` follows.
    pub wait: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStartOut {
    /// The id the capture gets.
    pub id: String,
    pub running: bool,
    /// The finished capture, when waited for.
    pub capture: Option<CaptureSummary>,
}

pub(crate) fn start(e: &mut Engine, ctx: &Ctx, i: CaptureStartIn) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    use_lease(e, &i.device, &ctx.actor)?;
    need_free(e, &i.device)?;
    let d = connected_device(e, &i.device)?;
    if !d.configured {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("Configure {} first with device.configure.", i.device),
        ));
    }
    let mut dev = d.device.take().ok_or_else(|| err(ErrorCode::Busy, "The device is busy."))?;
    let id = e.new_id("c");
    let title = i.title.clone().unwrap_or_else(|| format!("Capture {}", id.trim_start_matches('c')));
    let target = match &e.cfg.data_dir {
        Some(dir) => {
            let work = dir.join(format!("{id}.lacap"));
            match logica_fileio::StdFile::create(&work) {
                Ok(f) => StoreTarget { backing: Arc::new(f), cache_budget: e.cfg.cache_budget },
                Err(x) => {
                    if let Some(d) = e.devices.get_mut(&i.device) {
                        d.device = Some(dev);
                    }
                    return Err(err(
                        ErrorCode::Failed,
                        format!("{} could not be created: {x}.", work.display()),
                    ));
                }
            }
        }
        None => StoreTarget {
            backing: Arc::new(logica_core::io::MemFile::new()),
            cache_budget: e.cfg.cache_budget,
        },
    };
    let stop = StopSignal::new();
    let stop2 = stop.clone();
    let handle = std::thread::spawn(move || {
        let mut sink = StoreSink::new(target);
        let r = dev.run(&mut sink, &stop2).map(|o| (o, sink)).map_err(|x| x.0);
        (dev, r)
    });
    let running =
        Running { device: i.device.clone(), capture: id.clone(), title: title.clone(), stop, handle };
    let wait = i.wait.unwrap_or(true);
    let mut capture = None;
    if wait {
        e.finish_running(running);
        capture = e.captures.get(&id).map(|c| CaptureSummary::of(&id, c));
    } else {
        e.running.push(running);
    }
    let o = CaptureStartOut { id: id.clone(), running: !wait, capture };
    Ok(Done::never(out(&o), format!("captured {title} from {}", i.device)))
}

pub(crate) fn stop(e: &mut Engine, ctx: &Ctx, i: DeviceRef) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    use_lease(e, &i.device, &ctx.actor)?;
    let k = e
        .running
        .iter()
        .position(|r| r.device == i.device)
        .ok_or_else(|| err(ErrorCode::InvalidInput, format!("No capture is running on {}.", i.device)))?;
    let r = e.running.remove(k);
    r.stop.stop();
    e.finish_running(r);
    Ok(Done::never(out(&Nothing {}), format!("stopped the capture on {}", i.device)))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LeaseOut {
    pub holder: Option<Actor>,
}

pub(crate) fn take(e: &mut Engine, ctx: &Ctx, i: DeviceRef) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    let prev = e.leases.get(&i.device).cloned();
    match &prev {
        Some(h) if *h == ctx.actor => return Ok(Done::query(out(&LeaseOut { holder: prev }))),
        Some(h) if ctx.actor != Actor::User => {
            return Err(err(
                ErrorCode::Busy,
                format!("{} holds this device; only the person can take it over.", who(h)),
            ));
        }
        _ => {}
    }
    e.leases.insert(i.device.clone(), ctx.actor.clone());
    e.emit(Event::LeaseChanged { device: i.device.clone(), holder: Some(ctx.actor.clone()) });
    let o = out(&LeaseOut { holder: Some(ctx.actor.clone()) });
    let summary = format!("took the lease of {}", i.device);
    Ok(match prev {
        None => Done::edit(o, inv("lease.release", &i), summary),
        Some(h) => {
            let mut d = Done::never(o, format!("{summary} from {}", who(&h)));
            d.never_because =
                Some(format!("{} can take it back with lease.take when you hand it over.", who(&h)));
            d
        }
    })
}

pub(crate) fn release(e: &mut Engine, ctx: &Ctx, i: DeviceRef) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    match e.leases.get(&i.device) {
        None => return Ok(Done::query(out(&LeaseOut { holder: None }))),
        Some(h) if *h != ctx.actor && ctx.actor != Actor::User => {
            return Err(err(ErrorCode::Busy, format!("{} holds this lease, not you.", who(h))));
        }
        _ => {}
    }
    need_free(e, &i.device)?;
    e.leases.remove(&i.device);
    e.emit(Event::LeaseChanged { device: i.device.clone(), holder: None });
    Ok(Done::edit(
        out(&LeaseOut { holder: None }),
        inv("lease.take", &i),
        format!("released the lease of {}", i.device),
    ))
}
