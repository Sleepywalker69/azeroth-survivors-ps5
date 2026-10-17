//! The cursor, drawn by the game: the console has no window system to draw a hardware cursor, so
//! the image `crate::cursor` hands the window (`CursorIcon::Custom`, the `Interface\Cursor` BLPs)
//! is a UI image over everything else, at the window's cursor position. It hides when the window's
//! cursor does (`CursorOptions::visible`), as the hardware one would.

use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::window::{CursorIcon, CursorOptions, CustomCursor, PrimaryWindow};

/// The vanilla cursors' size, logical pixels.
const SIZE: f32 = 32.0;

#[derive(Component)]
pub(super) struct SoftwareCursor;

pub(super) fn spawn(mut commands: Commands) {
    commands.spawn((
        SoftwareCursor,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Px(SIZE),
            height: Val::Px(SIZE),
            ..default()
        },
        ImageNode::default(),
        GlobalZIndex(i32::MAX),
        Pickable::IGNORE,
        Visibility::Hidden,
    ));
}

pub(super) fn follow(
    windows: Query<(&Window, Option<&CursorIcon>, Option<&CursorOptions>), With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    mut cursors: Query<(&mut Node, &mut ImageNode, &mut Visibility), With<SoftwareCursor>>,
) {
    // `Val::Px` is multiplied by `UiScale` (survivors lays out on a 768-unit canvas), while the
    // window's cursor position is in plain logical pixels: undo the scale so the image sits on the
    // pointer that picking uses, and stays the vanilla size on screen.
    let inv = if ui_scale.0 > 0.0 { 1.0 / ui_scale.0 } else { 1.0 };
    let Ok((window, icon, options)) = windows.single() else {
        return;
    };
    let image = match icon {
        Some(CursorIcon::Custom(CustomCursor::Image(custom))) => Some(custom.handle.clone()),
        _ => None,
    };
    let at = window.cursor_position();
    let shown = options.is_none_or(|o| o.visible) && at.is_some() && image.is_some();
    for (mut node, mut node_image, mut visibility) in &mut cursors {
        let want = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(want);
        let (Some(at), Some(image)) = (at, image.as_ref()) else {
            continue;
        };
        // The vanilla cursors' hotspot is their top-left tip.
        let (left, top, size) = (Val::Px(at.x * inv), Val::Px(at.y * inv), Val::Px(SIZE * inv));
        if node.left != left || node.top != top {
            node.left = left;
            node.top = top;
        }
        if node.width != size {
            node.width = size;
            node.height = size;
        }
        if node_image.image != *image {
            node_image.image = image.clone();
        }
    }
}
