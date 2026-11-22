//! Example 24: the scene of `libgnoblin-std`.
//!
//! A scene node is a node of the compositor's tree that the library makes for the shell. This shell makes a perspective camera and a
//! view that draws the first window through it, places both, mounts them under the scene's root, and logs the placement it reads
//! back. Every line it logs starts with `scene:`.

use gnoblin_std::objects::{Camera3D, Projection, Scene, SceneSize, Vec3, View};
use gnoblin_std::prelude::*;

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let objects = g.std()?.objects()?;
    let scene = Scene::get(&objects)?;

    let camera = Camera3D::new(&objects)?;
    camera.set_id("cam")?;
    camera.set_projection(Projection::Perspective)?;
    camera.set_fov(70.0)?;
    camera.set_position(&Vec3 { x: 1.0, y: 2.0, z: 3.0 })?;
    scene.add(&camera.as_scene_node()?)?;

    let view = View::new(&objects)?;
    view.set_id("panel")?;
    view.set_size(&SceneSize {
        width: 400.0,
        height: 300.0,
    })?;
    view.set_position(&Vec3 {
        x: 0.5,
        y: 0.2,
        z: -1.4,
    })?;
    view.set_camera(&camera)?;
    scene.add(&view.as_scene_node()?)?;

    let at = view.position()?.unwrap_or_default();
    g.log(&format!(
        "scene: the view {} is node {} at {} {} {}, the camera {} is node {} with fov {}",
        view.id()?,
        view.node()?,
        at.x,
        at.y,
        at.z,
        camera.id()?,
        camera.node()?,
        camera.fov()?
    ));
    // The nodes are retained: they stay in the tree when the shell lets go of its references.
    Ok(())
}
