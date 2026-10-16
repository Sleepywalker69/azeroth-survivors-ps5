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
    mut cursors: Query<(&mut Node, &mut ImageNode, &mut Visibility), With<SoftwareCursor>>,
) {
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
        if node.left != Val::Px(at.x) || node.top != Val::Px(at.y) {
            node.left = Val::Px(at.x);
            node.top = Val::Px(at.y);
        }
        if node_image.image != *image {
            node_image.image = image.clone();
        }
    }
}
