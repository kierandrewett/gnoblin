//! `output_profile`: a persistent selector for one monitor and the settings it gets when connected.
//!
//! `data/js/gnoblin-std/display/output_profile.ts` is the reference. The profile keeps no state of its own: the library's profile
//! (`displays.rs`) holds what is applied, and this class checks what a shell writes, with the words the TypeScript checks used, and
//! reads it back.

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};

use super::{PROFILES, edit, look};
use crate::displays::{Hid, Match, NeighbourSide, PlaceTarget, Position, ProfileSettings};
use crate::object::record::{enumeration, field, record};
use crate::state::NodeId;
use gnoblin_std_abi as abi;

/// What an `output_profile` object holds: the profile the library calls `hid`.
pub(super) struct ProfileRef {
    ctx: GnoblinCtx,
    hid: Hid,
}

/// The object of the profile `hid`: the one a shell has, or a new one (a profile made through the std table has none until a shell asks).
pub(super) fn profile_obj(cx: &mut Cx<'_>, hid: Hid) -> Res<Obj> {
    let ctx = cx.ctx();
    let found = PROFILES.with(|held| held.borrow().get(&(ctx, hid)).copied());
    if let Some(obj) = found
        && cx.class_of(obj).is_ok()
    {
        return Ok(obj);
    }
    let obj = cx.spawn("output_profile", ProfileRef { ctx, hid })?;
    PROFILES.with(|held| held.borrow_mut().insert((ctx, hid), obj));
    Ok(obj)
}

/// The object of the profile `hid`, as a value to hand out.
pub(super) fn profile_val(cx: &mut Cx<'_>, hid: Hid) -> Res<Val> {
    let obj = profile_obj(cx, hid)?;
    cx.handout(obj)
}

/// The profile an object of the class `output_profile` stands for.
pub(super) fn profile_of(cx: &Cx<'_>, value: &Val) -> Res<Hid> {
    cx.with_state(value.object()?, |held: &mut ProfileRef| held.hid)
}

const MATCH_KEYS: [&str; 4] = ["connector", "make", "model", "serial"];

/// The compositor's words for the places of `output_transform`.
const TRANSFORMS: [&str; 8] = [
    "normal",
    "90",
    "180",
    "270",
    "flipped",
    "flipped-90",
    "flipped-180",
    "flipped-270",
];

/// The places of `position_kind` that name a side, counted from one: `absolute` is 1.
const SIDES: [NeighbourSide; 4] = [
    NeighbourSide::RightOf,
    NeighbourSide::LeftOf,
    NeighbourSide::Above,
    NeighbourSide::Below,
];

fn side_word(side: NeighbourSide) -> &'static str {
    match side {
        NeighbourSide::RightOf => "right_of",
        NeighbourSide::LeftOf => "left_of",
        NeighbourSide::Above => "above",
        NeighbourSide::Below => "below",
    }
}

/// The words `align` may take along a side: the start, the middle and the end of the shared edge.
fn align_words(side: NeighbourSide) -> [&'static str; 3] {
    match side {
        NeighbourSide::RightOf | NeighbourSide::LeftOf => ["top", "center", "bottom"],
        NeighbourSide::Above | NeighbourSide::Below => ["left", "center", "right"],
    }
}

/// Whether `text` is `WIDTHxHEIGHT@HERTZ`: digits, `x`, digits, `@`, digits with an optional fraction.
fn is_mode(text: &str) -> bool {
    let Some((size, rate)) = text.split_once('@') else {
        return false;
    };
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let rate_ok = match rate.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(rate),
    };
    digits(width) && digits(height) && rate_ok
}

/// The hardware an `output_match` value names: every key given is not empty, and one is given.
fn match_of(value: &Val) -> Res<Match> {
    let fields = value.record()?;
    let mut keys: [Option<String>; 4] = [None, None, None, None];
    for (at, name) in MATCH_KEYS.iter().enumerate() {
        if let Some(given) = field(fields, at) {
            let text = given.text()?;
            if text.is_empty() {
                return Err(Fail::type_error(format!(
                    "OutputProfile.match.{name}: must be a non-empty string"
                )));
            }
            keys[at] = Some(text.to_owned());
        }
    }
    if keys.iter().all(Option::is_none) {
        return Err(Fail::type_error(format!(
            "OutputProfile.match: needs at least one of {}",
            MATCH_KEYS.join(", ")
        )));
    }
    let [connector, make, model, serial] = keys;
    Ok(Match {
        connector,
        make,
        model,
        serial,
    })
}

fn match_val(matcher: &Match) -> Val {
    let text = |value: &Option<String>| value.clone().map_or(Val::None, Val::Text);
    record(vec![
        text(&matcher.connector),
        text(&matcher.make),
        text(&matcher.model),
        text(&matcher.serial),
    ])
}

/// The default name of a profile: the keys of its match, written out.
fn describe(matcher: &Match) -> String {
    [&matcher.connector, &matcher.make, &matcher.model, &matcher.serial]
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join("/")
}

/// The position a `output_position` value names. `outputs` are the nodes of the connected outputs.
fn position_of(cx: &Cx<'_>, value: &Val, path: &str, outputs: &[NodeId]) -> Res<Position> {
    let fields = value.record()?;
    let kind = fields
        .first()
        .ok_or_else(|| Fail::type_error(format!("{path}.position: needs a kind")))?
        .enum_value()?;
    if kind == 1 {
        return match (field(fields, 1), field(fields, 2)) {
            (Some(x), Some(y)) => Ok(Position::Absolute(x.int()? as f64, y.int()? as f64)),
            _ => Err(Fail::type_error(format!(
                "{path}.position: must be x and y with whole numbers, or name a side such as right_of"
            ))),
        };
    }
    let side = usize::try_from(kind)
        .ok()
        .and_then(|place| place.checked_sub(2))
        .and_then(|at| SIDES.get(at))
        .copied()
        .ok_or_else(|| Fail::type_error(format!("{path}.position: not a kind of position")))?;
    for (at, key) in [(1, "x"), (2, "y")] {
        if field(fields, at).is_some() {
            return Err(Fail::type_error(format!(
                "{path}.position.{key}: a relative position takes one side and align, not {key}"
            )));
        }
    }
    let target = match (field(fields, 3), field(fields, 4)) {
        (Some(profile), None) => PlaceTarget::Profile(profile_of(cx, profile)?),
        (None, Some(output)) => {
            let node = output.uint()?;
            if !outputs.contains(&node) {
                return Err(Fail::type_error(format!(
                    "{path}.position.{}: must be an OutputProfile or an Output",
                    side_word(side)
                )));
            }
            PlaceTarget::Output(node)
        }
        _ => {
            return Err(Fail::type_error(format!(
                "{path}.position.{}: must be an OutputProfile or an Output",
                side_word(side)
            )));
        }
    };
    let words = align_words(side);
    let align = match field(fields, 5) {
        None => abi::STD_ALIGN_START,
        Some(word) => {
            let word = word.text()?;
            match words.iter().position(|allowed| *allowed == word) {
                Some(0) => abi::STD_ALIGN_START,
                Some(1) => abi::STD_ALIGN_CENTER,
                Some(_) => abi::STD_ALIGN_END,
                None => {
                    return Err(Fail::type_error(format!(
                        "{path}.position.align: along {} it must be one of {}",
                        side_word(side),
                        words.join(", ")
                    )));
                }
            }
        }
    };
    Ok(Position::Relative { side, align, target })
}

/// The settings a `profile_settings` value asks for, each key checked in turn and named in the failure. `path` names the profile.
pub(super) fn settings_of(cx: &Cx<'_>, value: &Val, path: &str, outputs: &[NodeId]) -> Res<ProfileSettings> {
    let fields = value.record()?;
    let mut settings = ProfileSettings::default();
    if let Some(mode) = field(fields, 0) {
        let mode = mode.text()?;
        if !(mode == "preferred" || is_mode(mode)) {
            return Err(Fail::type_error(format!(
                "{path}.mode must be \"preferred\" or \"WIDTHxHEIGHT@HERTZ\", such as \"3440x1440@144\""
            )));
        }
        settings.mode = Some(mode.to_owned());
    }
    if let Some(scale) = field(fields, 1) {
        let scale = scale.double()?;
        if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
            return Err(Fail::type_error(format!("{path}.scale must be a number from 0.5 to 4")));
        }
        settings.scale = Some(scale);
    }
    if let Some(factor) = field(fields, 2) {
        let factor = factor.double()?;
        if !factor.is_finite() || !(1.0..=16.0).contains(&factor) {
            return Err(Fail::type_error(format!(
                "{path}.supersample must be a number from 1 to 16, such as 1, 2 or 4"
            )));
        }
        settings.supersample = Some(factor);
    }
    if let Some(transform) = field(fields, 3) {
        let word = usize::try_from(transform.enum_value()?)
            .ok()
            .and_then(|place| place.checked_sub(1))
            .and_then(|at| TRANSFORMS.get(at))
            .ok_or_else(|| Fail::type_error(format!("{path}.transform must be one of {}", TRANSFORMS.join(", "))))?;
        settings.transform = Some((*word).to_owned());
    }
    if let Some(vrr) = field(fields, 4) {
        settings.vrr = Some(vrr.bool()?);
    }
    if let Some(position) = field(fields, 5) {
        settings.position = Some(position_of(cx, position, path, outputs)?);
    }
    if let Some(enabled) = field(fields, 6) {
        settings.enabled = Some(enabled.bool()?);
    }
    Ok(settings)
}

fn position_val(cx: &mut Cx<'_>, position: &Position) -> Res<Val> {
    Ok(match *position {
        Position::Absolute(x, y) => record(vec![
            enumeration(1),
            Val::Int(x as i64),
            Val::Int(y as i64),
            Val::None,
            Val::None,
            Val::None,
        ]),
        Position::Relative { side, align, target } => {
            let place = SIDES.iter().position(|one| *one == side).map_or(2, |at| at as u32 + 2);
            let words = align_words(side);
            let word = match align {
                abi::STD_ALIGN_CENTER => Val::Text(words[1].to_owned()),
                abi::STD_ALIGN_END => Val::Text(words[2].to_owned()),
                _ => Val::None,
            };
            let (profile, output) = match target {
                PlaceTarget::Profile(hid) => (super::profile::profile_val(cx, hid)?, Val::None),
                PlaceTarget::Output(node) => (Val::None, Val::Uint(node)),
            };
            record(vec![enumeration(place), Val::None, Val::None, profile, output, word])
        }
    })
}

/// A `profile_settings` value for what a profile asks.
fn settings_val(cx: &mut Cx<'_>, settings: &ProfileSettings) -> Res<Val> {
    let transform = settings
        .transform
        .as_deref()
        .and_then(|word| TRANSFORMS.iter().position(|known| *known == word))
        .map_or(Val::None, |at| enumeration(at as u32 + 1));
    let position = match &settings.position {
        Some(position) => position_val(cx, position)?,
        None => Val::None,
    };
    Ok(record(vec![
        settings.mode.clone().map_or(Val::None, Val::Text),
        settings.scale.map_or(Val::None, Val::Double),
        settings.supersample.map_or(Val::None, Val::Double),
        transform,
        settings.vrr.map_or(Val::None, Val::Bool),
        position,
        settings.enabled.map_or(Val::None, Val::Bool),
    ]))
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<ProfileRef>("output_profile")
        .ctor(|cx, inits| {
            let ctx = cx.ctx();
            let given = inits.take("match").ok_or_else(|| {
                Fail::type_error("OutputProfile.match: needs at least one of connector, make, model, serial")
            })?;
            let matcher = match_of(&given)?;
            let id = describe(&matcher);
            let hid = edit(ctx, |st| Ok(st.new_profile(id, matcher, ProfileSettings::default())))?;
            PROFILES.with(|held| held.borrow_mut().insert((ctx, hid), cx.me()));
            Ok(ProfileRef { ctx, hid })
        })
        .get("match", |held, _| {
            look(held.ctx, |st| st.profile(held.hid).map(|rec| match_val(&rec.matcher)))
        })
        .get("id", |held, _| {
            look(held.ctx, |st| st.profile(held.hid).map(|rec| rec.id.clone())).map(Val::Text)
        })
        .set("id", |held, cx, value| {
            let id = value.text()?.to_owned();
            if id.is_empty() {
                return Err(Fail::type_error("OutputProfile.id: must be a non-empty string"));
            }
            edit(held.ctx, |st| {
                let rec = st.profile(held.hid)?;
                if rec.listed && rec.id != id {
                    return Err(crate::error::Fail::refused(format!(
                        "OutputProfile \"{}\".id: cannot change while the profile is in the manager",
                        rec.id
                    )));
                }
                if let Some(rec) = st.disp.profiles.get_mut(&held.hid) {
                    rec.id = id;
                }
                Ok(())
            })?;
            cx.changed("id");
            Ok(())
        })
        .get("settings", |held, cx| {
            let settings = look(held.ctx, |st| st.profile(held.hid).map(|rec| rec.settings.clone()))?;
            settings_val(cx, &settings)
        })
        .set("settings", |held, cx, value| {
            let (id, outputs) = look(held.ctx, |st| {
                Ok((st.profile(held.hid)?.id.clone(), st.outputs.clone()))
            })?;
            let settings = settings_of(cx, &value, &format!("OutputProfile \"{id}\""), &outputs)?;
            edit(held.ctx, |st| st.set_profile(held.hid, settings))?;
            cx.changed("settings");
            Ok(())
        })
        .get("listed", |held, _| {
            look(held.ctx, |st| st.profile(held.hid).map(|rec| rec.listed)).map(Val::Bool)
        })
        .function("format_mode", |_cx, args| {
            let (width, height, rate) = (args[0].double()?, args[1].double()?, args[2].double()?);
            for (key, value) in [("width", width), ("height", height), ("refresh_rate", rate)] {
                if !value.is_finite() || value <= 0.0 {
                    return Err(Fail::type_error(format!(
                        "OutputProfile.mode.{key}: must be a number above 0 (a mode is a width, a height and a refresh rate)"
                    )));
                }
            }
            if width.fract() != 0.0 || height.fract() != 0.0 {
                return Err(Fail::type_error(
                    "OutputProfile.mode: width and height must be whole numbers of pixels",
                ));
            }
            Ok(Val::Text(format!("{width}x{height}@{rate}")))
        });
}
