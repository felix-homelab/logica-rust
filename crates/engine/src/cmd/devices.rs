//! Devices, capturing and leases (doc 05, ADR 0008). Hardware commands need the device's lease:
//! the first actor to use a device holds it; others get a clear error; the person can always take
//! it over with `lease.take`.

use std::collections::BTreeMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::capture::{Source, SourceKind};
use logica_core::device::{Capabilities, CaptureConfig, DeviceInfo, StopSignal, device_source};
use logica_core::import::StoreTarget;

use crate::contract::{Actor, CommandError, ErrorCode, Event};
use crate::engine::{Ctx, Done, Engine, OpenDevice, Running, err};
use crate::group::Member;
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
    /// A device id from device.list, such as "vla/127.0.0.1:7171".
    pub device: String,
}

/// Fills the catalogue on first use. With `rescan`, drivers whose devices come and go (the
/// virtual analyser) are asked again; devices that are open stay listed.
fn catalogue(e: &mut Engine, rescan: bool) {
    let first = e.catalogue.is_empty();
    if !first && !rescan {
        return;
    }
    for (k, d) in e.drivers.iter().enumerate() {
        if !first && !d.hotplug() {
            continue;
        }
        if !first {
            e.catalogue.retain(|(dk, info, _)| *dk != k || e.devices.contains_key(&info.id));
        }
        for info in d.enumerate() {
            if e.catalogue.iter().any(|c| c.1.id == info.id) {
                continue;
            }
            if let Ok(dev) = d.open(&info.id) {
                e.catalogue.push((k, info, dev.capabilities().clone()));
            }
        }
    }
}

fn entries_of(e: &Engine) -> Vec<DeviceEntry> {
    e.catalogue
        .iter()
        .map(|(_, info, caps)| DeviceEntry {
            info: info.clone(),
            capabilities: caps.clone(),
            connected: e.devices.contains_key(&info.id),
            holder: e.leases.get(&info.id).cloned(),
            capturing: e.running.iter().any(|r| r.devices.contains(&info.id)),
        })
        .collect()
}

/// The devices known, without asking hotplug drivers again (for `session.get`).
pub(crate) fn known_entries(e: &mut Engine) -> Vec<DeviceEntry> {
    catalogue(e, false);
    entries_of(e)
}

/// Every device, asking hotplug drivers again (`device.list`).
pub(crate) fn entries(e: &mut Engine) -> Vec<DeviceEntry> {
    catalogue(e, true);
    entries_of(e)
}

fn entry(e: &mut Engine, id: &str) -> Result<DeviceEntry, CommandError> {
    catalogue(e, false);
    if let Some(d) = entries_of(e).into_iter().find(|d| d.info.id == id) {
        return Ok(d);
    }
    // perhaps a virtual analyser started since the last list
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
        e.devices.insert(
            i.device.clone(),
            OpenDevice {
                device: Some(dev),
                configured: false,
                samples: 0,
                rate_hz: 0.0,
                channels: Vec::new(),
            },
        );
    }
    let d = entry(e, &i.device)?;
    let model = d.info.model.clone();
    Ok(Done::never(out(&d), format!("connected {model}")))
}

fn need_free(e: &Engine, device: &str) -> Result<(), CommandError> {
    if e.running.iter().any(|r| r.devices.iter().any(|d| d == device)) {
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
    d.samples = i.config.samples;
    d.rate_hz = i.config.sample_rate_hz;
    d.channels.clone_from(&i.config.channels);
    let rate = logica_protocols::format::rate(i.config.sample_rate_hz);
    Ok(Done::never(
        out(&Nothing {}),
        format!("configured {} for {} samples at {rate}", i.device, i.config.samples),
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureStartIn {
    pub device: String,
    /// More devices captured at the same time into the same capture, after `device`'s channels
    /// (numbered on, and named by device). Each is configured first, all at the same sample rate.
    /// Devices do not share a clock: they are lined up at their starts, or by `align`.
    pub with: Option<Vec<String>>,
    /// Lines the devices up by a signal they all see (a shared clock or any shared line): for
    /// `device` and every device in `with`, the channel that carries it, by device id. The offset
    /// is found from its rising edges (between equally good offsets, as with a steady clock, the
    /// smallest) and each clock's drift is followed while capturing.
    pub align: Option<BTreeMap<String, u32>>,
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

/// The name a device's channels carry in a capture of several devices: its model without the
/// driver's prefix, numbered when two are alike.
fn labels(models: &[String]) -> Vec<String> {
    let short: Vec<String> =
        models.iter().map(|m| m.strip_prefix("Virtual analyser: ").unwrap_or(m).to_string()).collect();
    short
        .iter()
        .enumerate()
        .map(|(k, l)| {
            let before = short[..k].iter().filter(|x| *x == l).count();
            let alike = short.iter().filter(|x| *x == l).count();
            if alike > 1 { format!("{l} {}", before + 1) } else { l.clone() }
        })
        .collect()
}

/// Checks that `ids` can capture together into one capture, as `start` asks.
fn check_together(
    e: &mut Engine,
    ctx: &Ctx,
    ids: &[String],
    align: Option<&BTreeMap<String, u32>>,
) -> Result<(), CommandError> {
    for (k, id) in ids.iter().enumerate() {
        if ids[..k].contains(id) {
            return Err(err(
                ErrorCode::InvalidInput,
                format!("{id} is listed twice; each device captures once."),
            ));
        }
        entry(e, id)?;
        use_lease(e, id, &ctx.actor)?;
        need_free(e, id)?;
        let d = connected_device(e, id)?;
        if !d.configured {
            return Err(err(ErrorCode::InvalidInput, format!("Configure {id} first with device.configure.")));
        }
        if d.device.is_none() {
            return Err(err(ErrorCode::Busy, format!("{id} is busy.")));
        }
    }
    let first = &ids[0];
    let rate = e.devices.get(first).map_or(0.0, |d| d.rate_hz);
    for id in &ids[1..] {
        let r = e.devices.get(id).map_or(0.0, |d| d.rate_hz);
        if (r - rate).abs() > rate * 1e-9 {
            return Err(err(
                ErrorCode::InvalidInput,
                format!(
                    "{id} is configured for {}, {first} for {}; one capture has one rate. Configure them alike.",
                    logica_protocols::format::rate(r),
                    logica_protocols::format::rate(rate)
                ),
            ));
        }
    }
    if let Some(align) = align {
        if ids.len() < 2 {
            return Err(err(
                ErrorCode::InvalidInput,
                "align lines several devices up; name the others in with.",
            ));
        }
        if let Some(k) = align.keys().find(|k| !ids.contains(k)) {
            return Err(err(
                ErrorCode::InvalidInput,
                format!("align names {k}, which this capture does not use."),
            ));
        }
        for id in ids {
            let Some(ch) = align.get(id) else {
                return Err(err(
                    ErrorCode::InvalidInput,
                    format!(
                        "align has no channel for {id}; give the shared signal's channel for every device."
                    ),
                ));
            };
            if !e.devices.get(id).is_some_and(|d| d.channels.contains(ch)) {
                return Err(err(
                    ErrorCode::InvalidInput,
                    format!(
                        "{id} does not capture channel {ch}; enable it with device.configure to line up by it."
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn start(e: &mut Engine, ctx: &Ctx, i: CaptureStartIn) -> Result<Done, CommandError> {
    let mut ids = vec![i.device.clone()];
    ids.extend(i.with.clone().unwrap_or_default());
    check_together(e, ctx, &ids, i.align.as_ref())?;
    let samples = ids.iter().filter_map(|id| e.devices.get(id).map(|d| d.samples)).min().unwrap_or(0);
    let mut devs = Vec::new();
    for id in &ids {
        if let Some(dev) = e.devices.get_mut(id).and_then(|d| d.device.take()) {
            devs.push((id.clone(), dev));
        }
    }
    let id = e.new_id("c");
    let title = i.title.clone().unwrap_or_else(|| format!("Capture {}", id.trim_start_matches('c')));
    let target = match &e.cfg.data_dir {
        Some(dir) => {
            let work = dir.join(format!("{id}.lacap"));
            // the working folder may not exist yet (a first start)
            let _ = std::fs::create_dir_all(dir);
            match logica_fileio::StdFile::create(&work) {
                Ok(f) => StoreTarget { backing: Arc::new(f), cache_budget: e.cfg.cache_budget },
                Err(x) => {
                    for (d, dev) in devs {
                        if let Some(o) = e.devices.get_mut(&d) {
                            o.device = Some(dev);
                        }
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
    let sources: Vec<Source> = devs.iter().map(|(_, d)| device_source(d.info())).collect();
    let source = match sources.as_slice() {
        [one] => one.clone(),
        many => Source {
            kind: SourceKind::Device,
            detail: format!(
                "several devices together, {}: {}",
                if i.align.is_some() { "lined up by a shared signal" } else { "lined up at their starts" },
                many.iter().map(|s| s.detail.as_str()).collect::<Vec<_>>().join("; ")
            ),
        },
    };
    let (mut sink, feed) = crate::live::relay();
    let handle = if devs.len() == 1 {
        let (d, mut dev) = devs.remove(0);
        std::thread::spawn(move || {
            let r = dev.run(&mut sink, &stop2).map_err(|x| x.0);
            // dropping the sink tells the engine the device is done
            drop(sink);
            (vec![(d, dev)], r)
        })
    } else {
        let models: Vec<String> = devs.iter().map(|(_, d)| d.info().model.clone()).collect();
        let members: Vec<Member> = devs
            .into_iter()
            .zip(labels(&models))
            .map(|((d, dev), label)| Member {
                align: i.align.as_ref().and_then(|a| a.get(&d).copied()),
                id: d,
                label,
                device: dev,
            })
            .collect();
        std::thread::spawn(move || {
            let r = crate::group::run(members, &mut sink, &stop2);
            drop(sink);
            r
        })
    };
    let running = Running {
        devices: ids.clone(),
        capture: id.clone(),
        title: title.clone(),
        source,
        stop,
        handle,
        feed,
        target: Some(target),
        samples,
        begun: false,
        failed: None,
        reported: 0,
    };
    let wait = i.wait.unwrap_or(true);
    let mut capture = None;
    if wait {
        e.finish_running(running);
        capture = e.captures.get(&id).map(|c| CaptureSummary::of(&id, c));
    } else {
        e.running.push(running);
    }
    let o = CaptureStartOut { id: id.clone(), running: !wait, capture };
    Ok(Done::never(out(&o), format!("captured {title} from {}", ids.join(" and "))))
}

pub(crate) fn stop(e: &mut Engine, ctx: &Ctx, i: DeviceRef) -> Result<Done, CommandError> {
    entry(e, &i.device)?;
    use_lease(e, &i.device, &ctx.actor)?;
    let k = e
        .running
        .iter()
        .position(|r| r.devices.contains(&i.device))
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
