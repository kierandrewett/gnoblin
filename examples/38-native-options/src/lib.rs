//! Example 38: the options record of a constructor.
//!
//! A typed class has `new`, which takes the properties its catalog names, and `new_with`, which takes them and then a `<Class>Options`
//! struct: each `Some` field sets the property of its name in the same call that makes the object, so a value the library refuses fails
//! the call and leaves no object, and a field left `None` keeps the library's default. This shell makes:
//!
//! - workspaces with `new`, with every option (`name`, `gaps`, `layout`) and with one of them, and reads what each holds;
//! - output profiles with `new` and with the options `id` and `settings`, which the library sets as the object is made;
//! - a workspace and two profiles with a value the library refuses, and reads the library's words and what the lists hold afterwards.
//!
//! Every line it logs starts with `ctoropts:`, and the TypeScript and Python shells of the same example log the same lines. Build it with
//! `cargo build -p gnoblin-example-native-options` and run it with `gnoblin ./target/debug/libnative_options.so`.

use gnoblin_core::objects::ModelRoot;
use gnoblin_std::objects::{
    Gaps, MasterStackLayout, OutputMatch, OutputProfile, OutputProfileOptions, ProfileSettings, Workspace,
    WorkspaceOptions,
};
use gnoblin_std::prelude::*;

/// The gaps a workspace holds, as `inner/outer`.
fn gaps_of(workspace: &Workspace) -> String {
    match workspace.gaps() {
        Ok(Some(gaps)) => format!("{}/{}", gaps.inner, gaps.outer),
        _ => "none".to_owned(),
    }
}

/// The scale a profile's settings hold, or `unset`.
fn scale_of(profile: &OutputProfile) -> String {
    match profile.settings() {
        Ok(Some(ProfileSettings { scale: Some(scale), .. })) => format!("{scale}"),
        _ => "unset".to_owned(),
    }
}

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let model = ModelRoot::get(&g.objects()?)?;
    let objects = g.std()?.objects()?;
    let workspaces = model.workspaces()?;
    let outputs = model.outputs()?;
    let profiles = outputs.profiles()?;
    let say = |text: String| g.log(&format!("ctoropts: {text}"));
    say("running".to_owned());

    // No options: the constructor of every shell so far.
    let plain = Workspace::new(&objects, "plain")?;
    say(format!(
        "plain workspace: id={} name=\"{}\" gaps={}",
        plain.id()?,
        plain.name()?,
        gaps_of(&plain)
    ));

    // Every option.
    let main = Workspace::new_with(
        &objects,
        "main",
        WorkspaceOptions {
            name: Some("Main".to_owned()),
            gaps: Some(Gaps {
                inner: 6.0,
                outer: 12.0,
            }),
            layout: Some(MasterStackLayout::new(&objects)?.as_layout()?),
        },
    )?;
    say(format!(
        "main workspace: id={} name=\"{}\" gaps={} layout={}",
        main.id()?,
        main.name()?,
        gaps_of(&main),
        main.layout()?.is_some()
    ));

    // One option: what is left out keeps the library's default.
    let half = Workspace::new_with(
        &objects,
        "half",
        WorkspaceOptions {
            name: Some("Half".to_owned()),
            ..WorkspaceOptions::default()
        },
    )?;
    say(format!(
        "half workspace: id={} name=\"{}\" gaps={}",
        half.id()?,
        half.name()?,
        gaps_of(&half)
    ));

    // A value the library refuses fails the call, and the next workspace of the id is made as if nothing had happened.
    let refused = Workspace::new_with(
        &objects,
        "bad",
        WorkspaceOptions {
            gaps: Some(Gaps {
                inner: -1.0,
                outer: 0.0,
            }),
            ..WorkspaceOptions::default()
        },
    );
    say(match refused {
        Ok(_) => "refused gaps: no".to_owned(),
        Err(error) => format!("refused gaps: {error}"),
    });
    let bad = Workspace::new(&objects, "bad")?;
    say(format!(
        "bad workspace after the refusal: id={} name=\"{}\" gaps={}",
        bad.id()?,
        bad.name()?,
        gaps_of(&bad)
    ));

    workspaces.add(&main)?;
    workspaces.add(&plain)?;
    let held: Vec<String> = workspaces
        .list()?
        .iter()
        .map(|workspace| workspace.id().unwrap_or_default())
        .collect();
    say(format!("workspaces hold {}", held.join(", ")));

    // The options of a class whose constructor sets only the match: the library sets the rest after it makes the object.
    let hdmi = OutputMatch {
        connector: Some("HDMI-A-9".to_owned()),
        ..OutputMatch::default()
    };
    let unnamed = OutputProfile::new(&objects, &hdmi)?;
    say(format!(
        "plain profile: id={} scale={}",
        unnamed.id()?,
        scale_of(&unnamed)
    ));
    let mine = OutputProfile::new_with(
        &objects,
        &hdmi,
        OutputProfileOptions {
            id: Some("mine".to_owned()),
            settings: Some(ProfileSettings {
                scale: Some(2.0),
                ..ProfileSettings::default()
            }),
        },
    )?;
    say(format!(
        "profile with options: id={} scale={}",
        mine.id()?,
        scale_of(&mine)
    ));
    let empty = OutputProfile::new_with(
        &objects,
        &hdmi,
        OutputProfileOptions {
            id: Some(String::new()),
            ..OutputProfileOptions::default()
        },
    );
    say(match empty {
        Ok(_) => "refused id: no".to_owned(),
        Err(error) => format!("refused id: {error}"),
    });
    let scaled = OutputProfile::new_with(
        &objects,
        &hdmi,
        OutputProfileOptions {
            id: Some("scaled".to_owned()),
            settings: Some(ProfileSettings {
                scale: Some(9.0),
                ..ProfileSettings::default()
            }),
        },
    );
    say(match scaled {
        Ok(_) => "refused scale: no".to_owned(),
        Err(error) => format!("refused scale: {error}"),
    });

    profiles.add(&mine)?;
    profiles.add(&unnamed)?;
    say(format!("profiles hold {}", profiles.length()?));
    say("done".to_owned());
    Ok(())
}
