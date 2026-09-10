use dioxus::desktop::{
    DesktopContext,
    muda::{Menu, MenuItem},
    trayicon::{Icon, init_tray_icon},
    use_muda_event_handler,
};
use dioxus::prelude::*;
use tokio::sync::mpsc::UnboundedSender;

use crate::state::SyncTarget;

pub fn use_system_tray(window: DesktopContext, sync_tx: UnboundedSender<SyncTarget>) {
    let (_tray, open_id, sync_id, quit_id) = use_hook(|| {
        let tray_menu = Menu::new();
        let open_item = MenuItem::new("Open Fumiko", true, None);
        let sync_item = MenuItem::new("Sync All", true, None);
        let quit_item = MenuItem::new("Quit", true, None);

        let _ = tray_menu.append_items(&[&open_item, &sync_item, &quit_item]);

        let icon = {
            let bytes = include_bytes!("../assets/png/logo120x120.png");
            let image = image::load_from_memory(bytes)
                .expect("Failed to load icon memory")
                .into_rgba8();
            
            let (width, height) = image.dimensions();
            let rgba_bytes = image.into_raw();

            Icon::from_rgba(rgba_bytes, width, height)
                .expect("Failed to create tray icon")
        };

        let tray = init_tray_icon(tray_menu, Some(icon));

        (tray, open_item.id().clone(), sync_item.id().clone(), quit_item.id().clone())
    });

    use_muda_event_handler(move |event| {
        if event.id == open_id {
            window.set_visible(true);
            window.set_minimized(false);
            window.set_focus();
        } else if event.id == sync_id {
            let _ = sync_tx.send(SyncTarget::All);
        } else if event.id == quit_id {
            std::process::exit(0);
        }
    });
}