//! Channel settings, views and markers: small edits, each with its exact inverse.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use logica_core::Sample;

use crate::contract::{Actor, CommandError, ErrorCode, Event};
use crate::engine::{Ctx, Done, Engine, err};
use crate::state::{Capture, Marker, View};

use super::{Nothing, inv, out};

fn channel_slot(c: &Capture, channel: u32) -> Result<usize, CommandError> {
    c.meta
        .channels
        .iter()
        .position(|ch| ch.index == channel)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("This capture has no channel {channel}.")))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelRenameIn {
    pub capture: String,
    /// Physical channel number.
    pub channel: u32,
    pub name: String,
}

pub(crate) fn channel_rename(e: &mut Engine, _: &Ctx, i: ChannelRenameIn) -> Result<Done, CommandError> {
    let c = e.capture_mut(&i.capture)?;
    let k = channel_slot(c, i.channel)?;
    let old = std::mem::replace(&mut c.meta.channels[k].name, i.name.clone());
    let back = ChannelRenameIn { name: old.clone(), ..i.clone() };
    Ok(Done::edit(
        out(&Nothing {}),
        inv("channel.rename", &back),
        format!("renamed CH{} from {old} to {}", i.channel, i.name),
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelColourIn {
    pub capture: String,
    pub channel: u32,
    /// "#rrggbb", or none for the default.
    pub colour: Option<String>,
}

pub(crate) fn channel_colour(e: &mut Engine, _: &Ctx, i: ChannelColourIn) -> Result<Done, CommandError> {
    if let Some(col) = &i.colour
        && !(col.len() == 7 && col.starts_with('#') && col[1..].chars().all(|x| x.is_ascii_hexdigit()))
    {
        return Err(err(ErrorCode::InvalidInput, format!("\"{col}\" is not a colour; use #rrggbb.")));
    }
    let c = e.capture_mut(&i.capture)?;
    let k = channel_slot(c, i.channel)?;
    let old = std::mem::replace(&mut c.meta.channels[k].colour, i.colour.clone());
    let back = ChannelColourIn { colour: old, ..i.clone() };
    Ok(Done::edit(
        out(&Nothing {}),
        inv("channel.colour", &back),
        format!("changed the colour of CH{}", i.channel),
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelGroupIn {
    pub capture: String,
    /// The bus group to set.
    pub group: Option<String>,
    /// Its channels; none or empty removes the group. A channel leaves any other group.
    pub channels: Option<Vec<u32>>,
    /// Replaces every group at once (what an undo uses).
    pub all: Option<BTreeMap<String, Vec<u32>>>,
}

pub(crate) fn channel_group(e: &mut Engine, _: &Ctx, i: ChannelGroupIn) -> Result<Done, CommandError> {
    let c = e.capture_mut(&i.capture)?;
    let old = c.groups.clone();
    let summary;
    match (&i.all, &i.group) {
        (Some(all), _) => {
            for ch in all.values().flatten() {
                channel_slot(c, *ch)?;
            }
            c.groups = all.clone();
            summary = "changed the bus groups".to_string();
        }
        (None, Some(g)) => {
            let chans = i.channels.clone().unwrap_or_default();
            for ch in &chans {
                channel_slot(c, *ch)?;
            }
            for list in c.groups.values_mut() {
                list.retain(|ch| !chans.contains(ch));
            }
            c.groups.retain(|_, l| !l.is_empty());
            if chans.is_empty() {
                c.groups.remove(g);
                summary = format!("removed the bus group {g}");
            } else {
                c.groups.insert(g.clone(), chans.clone());
                summary = format!(
                    "grouped {} as {g}",
                    chans.iter().map(|x| format!("CH{x}")).collect::<Vec<_>>().join(", ")
                );
            }
        }
        (None, None) => {
            return Err(err(ErrorCode::InvalidInput, "Name the group to set, or give all groups."));
        }
    }
    let back = ChannelGroupIn { capture: i.capture.clone(), group: None, channels: None, all: Some(old) };
    Ok(Done::edit(out(&Nothing {}), inv("channel.group", &back), summary))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelHideIn {
    pub capture: String,
    pub channel: u32,
    pub hidden: bool,
}

pub(crate) fn channel_hide(e: &mut Engine, _: &Ctx, i: ChannelHideIn) -> Result<Done, CommandError> {
    let c = e.capture_mut(&i.capture)?;
    channel_slot(c, i.channel)?;
    let was = c.hidden.contains(&i.channel);
    if i.hidden {
        c.hidden.insert(i.channel);
    } else {
        c.hidden.remove(&i.channel);
    }
    let back = ChannelHideIn { hidden: was, ..i.clone() };
    let what = if i.hidden { "hid" } else { "showed" };
    Ok(Done::edit(out(&Nothing {}), inv("channel.hide", &back), format!("{what} CH{}", i.channel)))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewSetIn {
    pub capture: String,
    pub from: Option<Sample>,
    pub to: Option<Sample>,
    pub cursor_a: Option<Sample>,
    pub cursor_b: Option<Sample>,
    #[serde(default)]
    pub clear_cursors: bool,
    pub selection: Option<(Sample, Sample)>,
    #[serde(default)]
    pub clear_selection: bool,
    /// Bring this capture's tab to the front.
    #[serde(default)]
    pub activate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ViewOut {
    pub view: View,
}

pub(crate) fn view_set(e: &mut Engine, ctx: &Ctx, i: ViewSetIn) -> Result<Done, CommandError> {
    let c = e.capture_mut(&i.capture)?;
    let n = c.store.sample_count();
    let old = c.view.clone();
    let mut v = old.clone();
    if i.clear_cursors {
        v.cursor_a = None;
        v.cursor_b = None;
    }
    if i.clear_selection {
        v.selection = None;
    }
    v.from = i.from.unwrap_or(v.from);
    v.to = i.to.unwrap_or(v.to);
    v.cursor_a = i.cursor_a.or(v.cursor_a);
    v.cursor_b = i.cursor_b.or(v.cursor_b);
    v.selection = i.selection.or(v.selection);
    if v.from >= v.to || v.to > n.max(1) {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("The view must be a range inside the capture: 0 to {n} samples, with from below to."),
        ));
    }
    c.view = v.clone();
    if i.activate {
        e.active = Some(i.capture.clone());
    }
    e.emit(Event::ViewChanged { capture: i.capture.clone(), actor: ctx.actor.clone() });
    let back = ViewSetIn {
        capture: i.capture.clone(),
        from: Some(old.from),
        to: Some(old.to),
        cursor_a: old.cursor_a,
        cursor_b: old.cursor_b,
        clear_cursors: true,
        selection: old.selection,
        clear_selection: true,
        activate: false,
    };
    let mut done = Done::edit(out(&ViewOut { view: v }), inv("view.set", &back), "moved the view");
    // the person's own panning and zooming is not undo history; an agent moving the view is
    done.record = matches!(ctx.actor, Actor::Agent(_));
    Ok(done)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkerAddIn {
    pub capture: String,
    pub at: Sample,
    /// End of a range note.
    pub end: Option<Sample>,
    pub label: String,
    pub note: Option<String>,
    /// The marker id to use (redo and undo use the id it had); a new one when absent.
    pub id: Option<String>,
    /// Move the view so the marker is in the middle.
    #[serde(default)]
    pub move_view: bool,
    /// Who placed it, when an undo restores someone else's marker (honoured for the person only).
    pub by: Option<Actor>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MarkerOut {
    pub marker: Marker,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkerRef {
    pub capture: String,
    pub marker: String,
}

pub(crate) fn marker_add(e: &mut Engine, ctx: &Ctx, i: MarkerAddIn) -> Result<Done, CommandError> {
    let n = e.capture(&i.capture)?.store.sample_count();
    if i.at >= n.max(1) || i.end.is_some_and(|x| x < i.at || x > n) {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("A marker must lie inside the capture (0 to {n} samples)."),
        ));
    }
    let id = match &i.id {
        Some(id) => {
            e.reserve_id(id);
            id.clone()
        }
        None => e.new_id("m"),
    };
    let c = e.capture_mut(&i.capture)?;
    if c.markers.contains_key(&id) {
        return Err(err(ErrorCode::InvalidInput, format!("A marker with the id \"{id}\" already exists.")));
    }
    let by = match (&i.by, &ctx.actor) {
        (Some(b), Actor::User) => b.clone(),
        _ => ctx.actor.clone(),
    };
    let m = Marker {
        id: id.clone(),
        at: i.at,
        end: i.end,
        label: i.label.clone(),
        note: i.note.clone(),
        by: by.clone(),
    };
    c.markers.insert(id.clone(), m.clone());
    if i.move_view {
        let span = c.view.to.saturating_sub(c.view.from).max(1);
        let from = i.at.saturating_sub(span / 2).min(n.saturating_sub(span));
        c.view.from = from;
        c.view.to = (from + span).min(n);
        e.emit(Event::ViewChanged { capture: i.capture.clone(), actor: ctx.actor.clone() });
    }
    let canonical = MarkerAddIn { id: Some(id.clone()), by: Some(by), ..i.clone() };
    Ok(Done::edit(
        out(&MarkerOut { marker: m }),
        inv("marker.remove", &MarkerRef { capture: i.capture.clone(), marker: id }),
        format!("added the marker \"{}\"", i.label),
    )
    .canonical(out(&canonical)))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkerUpdateIn {
    pub capture: String,
    pub marker: String,
    pub at: Option<Sample>,
    pub end: Option<Sample>,
    #[serde(default)]
    pub clear_end: bool,
    pub label: Option<String>,
    pub note: Option<String>,
    #[serde(default)]
    pub clear_note: bool,
}

fn find_marker<'a>(c: &'a mut Capture, id: &str) -> Result<&'a mut Marker, CommandError> {
    c.markers.get_mut(id).ok_or_else(|| err(ErrorCode::NotFound, format!("There is no marker \"{id}\".")))
}

pub(crate) fn marker_update(e: &mut Engine, _: &Ctx, i: MarkerUpdateIn) -> Result<Done, CommandError> {
    let c = e.capture_mut(&i.capture)?;
    let n = c.store.sample_count();
    let m = find_marker(c, &i.marker)?;
    let old = m.clone();
    let mut new = old.clone();
    if i.clear_end {
        new.end = None;
    }
    if i.clear_note {
        new.note = None;
    }
    new.at = i.at.unwrap_or(new.at);
    new.end = i.end.or(new.end);
    new.label = i.label.clone().unwrap_or(new.label);
    new.note = i.note.clone().or(new.note);
    if new.at >= n.max(1) || new.end.is_some_and(|x| x < new.at || x > n) {
        return Err(err(
            ErrorCode::InvalidInput,
            format!("A marker must lie inside the capture (0 to {n} samples)."),
        ));
    }
    *m = new.clone();
    let back = MarkerUpdateIn {
        capture: i.capture.clone(),
        marker: i.marker.clone(),
        at: Some(old.at),
        end: old.end,
        clear_end: true,
        label: Some(old.label.clone()),
        note: old.note.clone(),
        clear_note: true,
    };
    Ok(Done::edit(
        out(&MarkerOut { marker: new }),
        inv("marker.update", &back),
        format!("changed the marker \"{}\"", old.label),
    ))
}

pub(crate) fn marker_remove(e: &mut Engine, _: &Ctx, i: MarkerRef) -> Result<Done, CommandError> {
    let c = e.capture_mut(&i.capture)?;
    find_marker(c, &i.marker)?;
    let m =
        c.markers.remove(&i.marker).ok_or_else(|| err(ErrorCode::NotFound, "There is no such marker."))?;
    let back = MarkerAddIn {
        capture: i.capture.clone(),
        at: m.at,
        end: m.end,
        label: m.label.clone(),
        note: m.note.clone(),
        id: Some(m.id.clone()),
        move_view: false,
        by: Some(m.by.clone()),
    };
    Ok(Done::edit(out(&Nothing {}), inv("marker.add", &back), format!("removed the marker \"{}\"", m.label)))
}
